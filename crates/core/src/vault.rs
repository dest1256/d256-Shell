//! Encrypted vault: Argon2id -> XChaCha20-Poly1305. File layout:
//!
//! `magic "SHPV"(4) | version(1) | m_cost KiB (u32 LE) | t_cost (u32 LE) | p_cost (u32 LE) | salt(16) | nonce(24) | ciphertext+tag`
//!
//! The whole header (everything before the ciphertext) is the AEAD associated data, so any change
//! to it (or to the ciphertext) fails authentication.
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{Key, KeyInit, XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use zeroize::{Zeroize, Zeroizing};

const MAGIC: &[u8; 4] = b"SHPV";
const FORMAT_VERSION: u8 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const HEADER_LEN: usize = 4 + 1 + 4 + 4 + 4 + SALT_LEN + NONCE_LEN;
const TAG_LEN: usize = 16;

// Sanity limits for parameters read from an (untrusted) file header.
const MAX_M_COST_KIB: u32 = 1 << 20; // 1 GiB
const MAX_T_COST: u32 = 20;
const MAX_P_COST: u32 = 16;

#[derive(Debug)]
pub enum VaultError {
    /// No vault file. `backup_exists` tells the UI whether recovery from `.bak` is possible.
    Missing {
        backup_exists: bool,
    },
    /// File is structurally broken (bad magic, truncated, absurd parameters).
    Corrupt(String),
    UnsupportedVersion(u8),
    /// AEAD authentication failed: wrong master password OR the file was modified.
    AuthFailed,
    Io(std::io::Error),
    Crypto(String),
}

impl std::fmt::Display for VaultError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing {
                backup_exists: true,
            } => {
                write!(
                    f,
                    "The vault file was not found, but there is a backup (vault.bin.bak)."
                )
            }
            Self::Missing { .. } => write!(f, "The vault was not found."),
            Self::Corrupt(d) => write!(f, "The vault file is damaged ({d})."),
            Self::UnsupportedVersion(v) => {
                write!(f, "Unsupported vault format version: {v}.")
            }
            Self::AuthFailed => write!(
                f,
                "Wrong master password, or the vault file was changed or damaged."
            ),
            Self::Io(e) => write!(f, "Error accessing the vault file: {e}"),
            Self::Crypto(e) => write!(f, "Encryption error: {e}"),
        }
    }
}

impl std::error::Error for VaultError {}

impl From<std::io::Error> for VaultError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    pub m_cost_kib: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self {
            m_cost_kib: 64 * 1024,
            t_cost: 3,
            p_cost: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Server {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_extra_args: Option<String>,
    /// Private key file for key login (the path only: the key itself stays where it is).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_path: Option<String>,
    /// Passphrase of that key, if the user chose to save it (encrypted with the rest of the vault).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub key_passphrase: String,
    pub created: u64,
    pub updated: u64,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.password.zeroize();
        self.key_passphrase.zeroize();
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct VaultData {
    pub servers: Vec<Server>,
}

pub struct Vault {
    path: PathBuf,
    key: Zeroizing<[u8; 32]>,
    params: KdfParams,
    salt: [u8; SALT_LEN],
    pub data: VaultData,
}

fn random<const N: usize>() -> Result<[u8; N], VaultError> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|e| VaultError::Crypto(format!("RNG: {e}")))?;
    Ok(b)
}

fn derive_key(
    password: &str,
    salt: &[u8],
    p: KdfParams,
) -> Result<Zeroizing<[u8; 32]>, VaultError> {
    let params = Params::new(p.m_cost_kib, p.t_cost, p.p_cost, Some(32))
        .map_err(|e| VaultError::Crypto(format!("KDF params: {e}")))?;
    let mut key = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, key.as_mut())
        .map_err(|e| VaultError::Crypto(format!("KDF: {e}")))?;
    Ok(key)
}

fn bak_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".bak");
    PathBuf::from(s)
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".tmp");
    PathBuf::from(s)
}

/// Points at which `write_atomic` can be made to fail, for crash simulation in tests.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CrashAt {
    Never,
    AfterTmpWritten,
    AfterBackupRename,
}

/// `vault.bin.tmp` -> fsync -> current becomes `.bak` -> tmp becomes `vault.bin`.
fn write_atomic(path: &Path, bytes: &[u8], crash: CrashAt) -> std::io::Result<()> {
    let tmp = tmp_path(path);
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    if crash == CrashAt::AfterTmpWritten {
        return Err(std::io::Error::other("simulated crash after tmp written"));
    }
    if path.exists() {
        fs::rename(path, bak_path(path))?;
    }
    if crash == CrashAt::AfterBackupRename {
        return Err(std::io::Error::other("simulated crash after backup rename"));
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

impl Vault {
    pub fn exists(path: &Path) -> bool {
        path.exists()
    }

    /// Create a new vault file (fails if one already exists).
    pub fn create(path: &Path, master: &str, params: KdfParams) -> Result<Vault, VaultError> {
        if path.exists() {
            return Err(VaultError::Corrupt("the file already exists".into()));
        }
        let salt = random::<SALT_LEN>()?;
        let key = derive_key(master, &salt, params)?;
        let v = Vault {
            path: path.to_path_buf(),
            key,
            params,
            salt,
            data: VaultData::default(),
        };
        v.save()?;
        Ok(v)
    }

    /// Unlock the vault at `path`. If it is missing but `.bak` exists, returns `Missing{backup_exists:true}`.
    pub fn unlock(path: &Path, master: &str) -> Result<Vault, VaultError> {
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(VaultError::Missing {
                    backup_exists: bak_path(path).exists(),
                })
            }
            Err(e) => return Err(e.into()),
        };
        Self::open_bytes(path, &bytes, master)
    }

    /// Replace a broken `vault.bin` with `vault.bin.bak` after verifying that the backup unlocks
    /// with `master`. The broken file is kept as `vault.bin.corrupt`.
    pub fn restore_from_backup(path: &Path, master: &str) -> Result<Vault, VaultError> {
        let bak = bak_path(path);
        let bytes = fs::read(&bak).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                VaultError::Missing {
                    backup_exists: false,
                }
            } else {
                e.into()
            }
        })?;
        let v = Self::open_bytes(path, &bytes, master)?;
        if path.exists() {
            let mut c = path.as_os_str().to_owned();
            c.push(".corrupt");
            fs::rename(path, PathBuf::from(c))?;
        }
        fs::copy(&bak, path)?;
        Ok(v)
    }

    fn open_bytes(path: &Path, bytes: &[u8], master: &str) -> Result<Vault, VaultError> {
        if bytes.len() < HEADER_LEN + TAG_LEN || &bytes[..4] != MAGIC {
            return Err(VaultError::Corrupt("invalid header".into()));
        }
        if bytes[4] != FORMAT_VERSION {
            return Err(VaultError::UnsupportedVersion(bytes[4]));
        }
        let u32_at = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().expect("4 bytes"));
        let params = KdfParams {
            m_cost_kib: u32_at(5),
            t_cost: u32_at(9),
            p_cost: u32_at(13),
        };
        if params.m_cost_kib > MAX_M_COST_KIB
            || params.t_cost == 0
            || params.t_cost > MAX_T_COST
            || params.p_cost == 0
            || params.p_cost > MAX_P_COST
        {
            return Err(VaultError::Corrupt("invalid KDF parameters".into()));
        }
        let salt: [u8; SALT_LEN] = bytes[17..17 + SALT_LEN].try_into().expect("salt");
        let nonce = &bytes[17 + SALT_LEN..HEADER_LEN];
        let key = derive_key(master, &salt, params)?;
        let cipher = XChaCha20Poly1305::new(&Key::from(*key));
        let nonce = XNonce::try_from(nonce).map_err(|_| VaultError::Corrupt("nonce".into()))?;
        let plain = Zeroizing::new(
            cipher
                .decrypt(
                    &nonce,
                    Payload {
                        msg: &bytes[HEADER_LEN..],
                        aad: &bytes[..HEADER_LEN],
                    },
                )
                .map_err(|_| VaultError::AuthFailed)?,
        );
        let data: VaultData = serde_json::from_slice(&plain)
            .map_err(|e| VaultError::Corrupt(format!("content: {e}")))?;
        Ok(Vault {
            path: path.to_path_buf(),
            key,
            params,
            salt,
            data,
        })
    }

    fn encode(&self) -> Result<Vec<u8>, VaultError> {
        let nonce = random::<NONCE_LEN>()?; // fresh nonce for every write
        let mut out = Vec::with_capacity(HEADER_LEN + 256);
        out.extend_from_slice(MAGIC);
        out.push(FORMAT_VERSION);
        out.extend_from_slice(&self.params.m_cost_kib.to_le_bytes());
        out.extend_from_slice(&self.params.t_cost.to_le_bytes());
        out.extend_from_slice(&self.params.p_cost.to_le_bytes());
        out.extend_from_slice(&self.salt);
        out.extend_from_slice(&nonce);
        let plain = Zeroizing::new(
            serde_json::to_vec(&self.data).map_err(|e| VaultError::Crypto(e.to_string()))?,
        );
        let cipher = XChaCha20Poly1305::new(&Key::from(*self.key));
        let ct = cipher
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: &plain,
                    aad: &out,
                },
            )
            .map_err(|_| VaultError::Crypto("encrypt".into()))?;
        out.extend_from_slice(&ct);
        Ok(out)
    }

    pub fn save(&self) -> Result<(), VaultError> {
        write_atomic(&self.path, &self.encode()?, CrashAt::Never)?;
        Ok(())
    }

    /// Re-encrypt under a new master password (new salt and key).
    pub fn change_master(&mut self, new_master: &str) -> Result<(), VaultError> {
        let salt = random::<SALT_LEN>()?;
        let key = derive_key(new_master, &salt, self.params)?;
        let (old_salt, old_key) = (self.salt, std::mem::replace(&mut self.key, key));
        self.salt = salt;
        if let Err(e) = self.save() {
            self.salt = old_salt;
            self.key = old_key;
            return Err(e);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Cheap KDF so tests are fast; production default is KdfParams::default().
    const FAST: KdfParams = KdfParams {
        m_cost_kib: 64,
        t_cost: 1,
        p_cost: 1,
    };

    fn dir(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("d256shell-vault-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn server(name: &str, pw: &str) -> Server {
        Server {
            id: name.into(),
            name: name.into(),
            host: "h".into(),
            port: 22,
            username: "u".into(),
            password: pw.into(),
            initial_dir: Some("/srv".into()),
            ssh_extra_args: None,
            key_path: None,
            key_passphrase: String::new(),
            created: 1,
            updated: 2,
        }
    }

    #[test]
    fn default_params_match_spec() {
        let p = KdfParams::default();
        assert_eq!((p.m_cost_kib, p.t_cost, p.p_cost), (65536, 3, 1));
    }

    #[test]
    fn roundtrip() {
        let d = dir("rt");
        let p = d.join("vault.bin");
        let mut v = Vault::create(&p, "master", FAST).unwrap();
        v.data.servers.push(server("a", "pa$$ юникод"));
        v.save().unwrap();
        let v2 = Vault::unlock(&p, "master").unwrap();
        assert_eq!(v2.data, v.data);
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn wrong_password_is_error_not_panic() {
        let d = dir("wp");
        let p = d.join("vault.bin");
        Vault::create(&p, "master", FAST).unwrap();
        let e = Vault::unlock(&p, "nope").err().unwrap();
        assert!(matches!(e, VaultError::AuthFailed));
        assert!(e.to_string().contains("Wrong master password"));
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn tampering_with_any_byte_is_detected() {
        let d = dir("tamper");
        let p = d.join("vault.bin");
        let mut v = Vault::create(&p, "master", FAST).unwrap();
        v.data.servers.push(server("a", "secret"));
        v.save().unwrap();
        let orig = fs::read(&p).unwrap();
        for i in 0..orig.len() {
            let mut b = orig.clone();
            b[i] ^= 0x01;
            fs::write(&p, &b).unwrap();
            assert!(
                Vault::unlock(&p, "master").is_err(),
                "byte {i} modification not detected"
            );
        }
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn nonce_changes_on_every_write() {
        let d = dir("nonce");
        let p = d.join("vault.bin");
        let v = Vault::create(&p, "master", FAST).unwrap();
        let n1 = fs::read(&p).unwrap()[17 + SALT_LEN..HEADER_LEN].to_vec();
        v.save().unwrap();
        let n2 = fs::read(&p).unwrap()[17 + SALT_LEN..HEADER_LEN].to_vec();
        assert_ne!(n1, n2);
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn file_does_not_contain_plaintext() {
        let d = dir("plain");
        let p = d.join("vault.bin");
        let mut v = Vault::create(&p, "master", FAST).unwrap();
        v.data.servers.push(server("myserver", "TopSecretPassword"));
        v.save().unwrap();
        let raw = fs::read(&p).unwrap();
        let has = |needle: &[u8]| raw.windows(needle.len()).any(|w| w == needle);
        assert!(!has(b"TopSecretPassword") && !has(b"myserver"));
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn crash_after_tmp_written_keeps_existing_vault() {
        let d = dir("crash1");
        let p = d.join("vault.bin");
        let mut v = Vault::create(&p, "master", FAST).unwrap();
        v.data.servers.push(server("a", "pa"));
        v.save().unwrap();
        let before = fs::read(&p).unwrap();
        v.data.servers.push(server("b", "pb"));
        assert!(write_atomic(&p, &v.encode().unwrap(), CrashAt::AfterTmpWritten).is_err());
        assert_eq!(fs::read(&p).unwrap(), before);
        assert_eq!(Vault::unlock(&p, "master").unwrap().data.servers.len(), 1);
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn crash_between_renames_is_recoverable_from_bak() {
        let d = dir("crash2");
        let p = d.join("vault.bin");
        let mut v = Vault::create(&p, "master", FAST).unwrap();
        v.data.servers.push(server("a", "pa"));
        v.save().unwrap();
        v.data.servers.push(server("b", "pb"));
        assert!(write_atomic(&p, &v.encode().unwrap(), CrashAt::AfterBackupRename).is_err());
        // vault.bin is gone, but the previous version sits in .bak
        match Vault::unlock(&p, "master") {
            Err(VaultError::Missing {
                backup_exists: true,
            }) => {}
            other => panic!("unexpected: {:?}", other.err()),
        }
        let r = Vault::restore_from_backup(&p, "master").unwrap();
        assert_eq!(r.data.servers.len(), 1);
        assert_eq!(Vault::unlock(&p, "master").unwrap().data.servers.len(), 1);
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn corrupt_vault_restored_from_bak() {
        let d = dir("bak");
        let p = d.join("vault.bin");
        let mut v = Vault::create(&p, "master", FAST).unwrap();
        v.data.servers.push(server("a", "pa"));
        v.save().unwrap();
        v.data.servers.push(server("b", "pb"));
        v.save().unwrap(); // .bak now holds the 1-server version
        fs::write(&p, b"garbage").unwrap();
        assert!(matches!(
            Vault::unlock(&p, "master"),
            Err(VaultError::Corrupt(_))
        ));
        let r = Vault::restore_from_backup(&p, "master").unwrap();
        assert_eq!(r.data.servers.len(), 1);
        assert!(d.join("vault.bin.corrupt").exists());
        assert!(Vault::unlock(&p, "master").is_ok());
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn restore_with_wrong_password_changes_nothing() {
        let d = dir("bakwrong");
        let p = d.join("vault.bin");
        let v = Vault::create(&p, "master", FAST).unwrap();
        v.save().unwrap();
        fs::write(&p, b"garbage").unwrap();
        assert!(Vault::restore_from_backup(&p, "wrong").is_err());
        assert_eq!(fs::read(&p).unwrap(), b"garbage");
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn change_master_password() {
        let d = dir("chg");
        let p = d.join("vault.bin");
        let mut v = Vault::create(&p, "old", FAST).unwrap();
        v.data.servers.push(server("a", "pa"));
        v.change_master("new").unwrap();
        assert!(matches!(
            Vault::unlock(&p, "old"),
            Err(VaultError::AuthFailed)
        ));
        assert_eq!(Vault::unlock(&p, "new").unwrap().data.servers.len(), 1);
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn hostile_header_parameters_are_rejected_without_huge_allocation() {
        let d = dir("hostile");
        let p = d.join("vault.bin");
        Vault::create(&p, "master", FAST).unwrap();
        let mut b = fs::read(&p).unwrap();
        b[5..9].copy_from_slice(&u32::MAX.to_le_bytes()); // m_cost = 4 TiB
        fs::write(&p, &b).unwrap();
        assert!(matches!(
            Vault::unlock(&p, "master"),
            Err(VaultError::Corrupt(_))
        ));
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn unsupported_version_and_missing_file() {
        let d = dir("ver");
        let p = d.join("vault.bin");
        assert!(matches!(
            Vault::unlock(&p, "m"),
            Err(VaultError::Missing {
                backup_exists: false
            })
        ));
        Vault::create(&p, "master", FAST).unwrap();
        let mut b = fs::read(&p).unwrap();
        b[4] = 99;
        fs::write(&p, &b).unwrap();
        assert!(matches!(
            Vault::unlock(&p, "master"),
            Err(VaultError::UnsupportedVersion(99))
        ));
        fs::remove_dir_all(d).unwrap();
    }
}
