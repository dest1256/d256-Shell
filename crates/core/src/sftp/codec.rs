//! Text encodings of the editor: detection, strict decoding and encoding.
//!
//! Safety rule: a file is only opened in an encoding if writing the unchanged text back gives exactly the same
//! bytes (otherwise saving would silently damage it). A character that the chosen encoding cannot represent is
//! reported on save and never replaced.
use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};
use encoding_rs::{Encoding, UTF_16BE, UTF_16LE, UTF_8};

/// Why a file cannot be shown as text.
#[derive(Debug, PartialEq, Eq)]
pub enum DecodeFail {
    /// Looks like a binary file (NUL bytes in a non-UTF-16 encoding).
    Binary,
    /// Unknown encoding name.
    Unknown(String),
    /// Bytes that are invalid in the (chosen or guessed) encoding.
    Invalid(&'static str),
    /// Decoding works but writing the text back would not reproduce the file.
    NotReversible(&'static str),
}

pub struct Decoded {
    /// Text exactly as stored (line endings untouched).
    pub text: String,
    pub bom: bool,
    pub encoding: &'static Encoding,
    /// The encoding was guessed (not forced, not announced by a BOM and not valid UTF-8).
    pub guessed: bool,
}

/// Encoding names shown in the editor (WHATWG labels understood by `encoding_rs`).
pub const CHOICES: &[(&str, &str)] = &[
    ("UTF-8", "Юникод (UTF-8)"),
    ("UTF-16LE", "Юникод (UTF-16 LE, как в Windows)"),
    ("UTF-16BE", "Юникод (UTF-16 BE)"),
    ("windows-1251", "Кириллица (Windows-1251)"),
    ("KOI8-R", "Кириллица (KOI8-R)"),
    ("KOI8-U", "Кириллица (KOI8-U)"),
    ("IBM866", "Кириллица (DOS, CP866)"),
    ("ISO-8859-5", "Кириллица (ISO 8859-5)"),
    ("x-mac-cyrillic", "Кириллица (Mac)"),
    ("windows-1252", "Западная Европа (Windows-1252 / Latin-1)"),
    ("ISO-8859-15", "Западная Европа (Latin-9)"),
    ("windows-1250", "Центральная Европа (Windows-1250)"),
    ("ISO-8859-2", "Центральная Европа (ISO 8859-2)"),
    ("windows-1253", "Греческий (Windows-1253)"),
    ("windows-1254", "Турецкий (Windows-1254)"),
    ("windows-1255", "Иврит (Windows-1255)"),
    ("windows-1256", "Арабский (Windows-1256)"),
    ("windows-1257", "Балтийский (Windows-1257)"),
    ("windows-1258", "Вьетнамский (Windows-1258)"),
    ("windows-874", "Тайский (Windows-874)"),
    ("Shift_JIS", "Японский (Shift_JIS)"),
    ("EUC-JP", "Японский (EUC-JP)"),
    ("ISO-2022-JP", "Японский (ISO-2022-JP)"),
    ("GBK", "Китайский упрощённый (GBK)"),
    ("gb18030", "Китайский (GB18030)"),
    ("Big5", "Китайский традиционный (Big5)"),
    ("EUC-KR", "Корейский (EUC-KR)"),
    ("macintosh", "Западная Европа (Mac Roman)"),
];

pub fn by_name(name: &str) -> Option<&'static Encoding> {
    Encoding::for_label(name.trim().as_bytes())
}

fn bom_of(bytes: &[u8]) -> Option<&'static Encoding> {
    Encoding::for_bom(bytes).map(|(e, _)| e)
}

fn is_utf16(e: &'static Encoding) -> bool {
    e == UTF_16LE || e == UTF_16BE
}

fn guess(bytes: &[u8]) -> &'static Encoding {
    let mut d = EncodingDetector::new(Iso2022JpDetection::Allow);
    d.feed(bytes, true);
    d.guess(None, Utf8Detection::Deny)
}

/// Decode `bytes`. `forced` = the user chose the encoding; otherwise: BOM, then valid UTF-8, then a guess.
pub fn decode(bytes: &[u8], forced: Option<&str>) -> Result<Decoded, DecodeFail> {
    let forced = match forced {
        Some(n) => Some(by_name(n).ok_or_else(|| DecodeFail::Unknown(n.to_string()))?),
        None => None,
    };
    let (enc, guessed) = match forced {
        Some(e) => (e, false),
        None => match bom_of(bytes) {
            Some(e) => (e, false),
            None if std::str::from_utf8(bytes).is_ok() && !bytes.contains(&0) => (UTF_8, false),
            None if bytes.contains(&0) => return Err(DecodeFail::Binary),
            None => (guess(bytes), true),
        },
    };
    if !is_utf16(enc) && bytes.contains(&0) {
        return Err(DecodeFail::Binary);
    }
    let bom = Encoding::for_bom(bytes).is_some_and(|(b, _)| b == enc);
    let (cow, had_errors) = enc.decode_with_bom_removal(bytes);
    if had_errors {
        return Err(DecodeFail::Invalid(enc.name()));
    }
    let text = cow.into_owned();
    if enc != UTF_8 {
        // legacy encodings: make sure that nothing would change on save
        let mut back = Vec::new();
        if bom {
            back.extend_from_slice(bom_bytes(enc));
        }
        match encode_raw(&text, enc) {
            Ok(b) => back.extend(b),
            Err(_) => return Err(DecodeFail::NotReversible(enc.name())),
        }
        if back != bytes {
            return Err(DecodeFail::NotReversible(enc.name()));
        }
    }
    Ok(Decoded {
        text,
        bom,
        encoding: enc,
        guessed,
    })
}

fn bom_bytes(e: &'static Encoding) -> &'static [u8] {
    if e == UTF_16LE {
        &[0xFF, 0xFE]
    } else if e == UTF_16BE {
        &[0xFE, 0xFF]
    } else if e == UTF_8 {
        &[0xEF, 0xBB, 0xBF]
    } else {
        &[]
    }
}

/// Encode text as is. `Err(c)` = the first character this encoding cannot represent.
pub fn encode_raw(text: &str, enc: &'static Encoding) -> Result<Vec<u8>, char> {
    if enc == UTF_8 {
        return Ok(text.as_bytes().to_vec());
    }
    if is_utf16(enc) {
        let le = enc == UTF_16LE;
        let mut out = Vec::with_capacity(text.len() * 2);
        for u in text.encode_utf16() {
            out.extend_from_slice(&if le { u.to_le_bytes() } else { u.to_be_bytes() });
        }
        return Ok(out);
    }
    let (cow, _, unmappable) = enc.encode(text);
    if unmappable {
        // slow path, only to name the culprit
        for c in text.chars() {
            let mut b = [0u8; 4];
            let (_, _, bad) = enc.encode(c.encode_utf8(&mut b));
            if bad {
                return Err(c);
            }
        }
        return Err('\u{FFFD}');
    }
    Ok(cow.into_owned())
}

/// What the editor writes: BOM (if wanted), then the text in the encoding. Line endings are handled by the caller.
pub fn encode(text: &str, bom: bool, enc: &'static Encoding) -> Result<Vec<u8>, char> {
    let mut out = Vec::with_capacity(text.len() + 3);
    if bom {
        out.extend_from_slice(bom_bytes(enc));
    }
    out.extend(encode_raw(text, enc)?);
    Ok(out)
}

/// Message for the user.
pub fn fail_message(f: &DecodeFail) -> String {
    match f {
        DecodeFail::Binary => "Файл бинарный (есть нулевые байты): редактор его не открывает. Скачайте файл.".into(),
        DecodeFail::Unknown(n) => format!("Неизвестная кодировка «{n}»."),
        DecodeFail::Invalid(n) => format!("В файле есть байты, недопустимые в кодировке {n}. Выберите другую кодировку (список справа внизу)."),
        DecodeFail::NotReversible(n) => format!("Файл нельзя безопасно редактировать как {n}: после сохранения часть байтов изменилась бы. Выберите другую кодировку."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(bytes: &[u8], enc: &str) {
        let d = decode(bytes, Some(enc)).unwrap();
        assert_eq!(encode(&d.text, d.bom, d.encoding).unwrap(), bytes, "{enc}");
    }

    #[test]
    fn utf8_and_bom_are_found_without_guessing() {
        let d = decode("привет".as_bytes(), None).unwrap();
        assert!(d.encoding == UTF_8 && !d.guessed && !d.bom);
        let d = decode(b"\xEF\xBB\xBFx", None).unwrap();
        assert!(d.encoding == UTF_8 && d.bom && d.text == "x");
    }

    #[test]
    fn utf16_with_bom_is_recognised_and_roundtrips() {
        let mut le = vec![0xFF, 0xFE];
        for u in "Привет\r\nмир".encode_utf16() {
            le.extend_from_slice(&u.to_le_bytes());
        }
        let d = decode(&le, None).unwrap();
        assert!(
            d.encoding == UTF_16LE && d.bom && !d.guessed,
            "{:?}",
            d.encoding
        );
        assert_eq!(d.text, "Привет\r\nмир");
        roundtrip(&le, "UTF-16LE");
        let mut be = vec![0xFE, 0xFF];
        for u in "ab".encode_utf16() {
            be.extend_from_slice(&u.to_be_bytes());
        }
        assert!(decode(&be, None).unwrap().encoding == UTF_16BE);
        roundtrip(&be, "UTF-16BE");
        // UTF-16 without BOM: the user picks it
        let plain: Vec<u8> = "text"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        assert_eq!(decode(&plain, None).err(), Some(DecodeFail::Binary));
        assert_eq!(decode(&plain, Some("UTF-16LE")).unwrap().text, "text");
    }

    #[test]
    fn russian_legacy_encodings_are_guessed_and_roundtrip() {
        let text = "Привет, мир! Это файл с настройками сервера: пароль, адрес, порт.\n";
        for name in ["windows-1251", "KOI8-R", "IBM866"] {
            let enc = by_name(name).unwrap();
            let bytes = encode_raw(text, enc).unwrap();
            let d = decode(&bytes, None).unwrap();
            assert!(d.guessed, "{name}");
            // KOI8-R / 1251 / 866 are all "valid": whatever it guessed must at least be reversible,
            // and the right one must give the right text when chosen
            assert_eq!(decode(&bytes, Some(name)).unwrap().text, text, "{name}");
            roundtrip(&bytes, name);
        }
        let b1251 = encode_raw(text, by_name("windows-1251").unwrap()).unwrap();
        assert_eq!(
            decode(&b1251, None).unwrap().encoding.name(),
            "windows-1251"
        );
    }

    #[test]
    fn east_asian_encodings_work() {
        for (name, text) in [
            ("Shift_JIS", "日本語のテキスト"),
            ("GBK", "中文文本"),
            ("EUC-KR", "한국어 텍스트"),
            ("Big5", "繁體中文"),
        ] {
            let b = encode_raw(text, by_name(name).unwrap()).unwrap();
            assert_eq!(decode(&b, Some(name)).unwrap().text, text, "{name}");
            roundtrip(&b, name);
        }
    }

    #[test]
    fn every_listed_choice_is_a_known_encoding() {
        for (n, _) in CHOICES {
            assert!(by_name(n).is_some(), "{n}");
        }
    }

    #[test]
    fn invalid_bytes_and_binary_are_refused_not_replaced() {
        // 0x98 is undefined in windows-1251? it maps; use UTF-8 invalid for a forced utf-8 instead
        assert!(matches!(
            decode(b"a\xffb", Some("UTF-8")),
            Err(DecodeFail::Invalid(_))
        ));
        assert_eq!(decode(b"a\0b", None).err(), Some(DecodeFail::Binary));
        assert_eq!(
            decode(b"a\0b", Some("windows-1251")).err(),
            Some(DecodeFail::Binary)
        );
        assert!(matches!(
            decode(b"x", Some("nope")),
            Err(DecodeFail::Unknown(_))
        ));
    }

    #[test]
    fn unrepresentable_characters_are_named_not_replaced() {
        let e = by_name("windows-1251").unwrap();
        assert_eq!(encode("цена 5 ß?", false, e).err(), Some('ß'));
        assert_eq!(
            encode("цена 5 ß?", false, UTF_8).unwrap(),
            "цена 5 ß?".as_bytes()
        );
        assert_eq!(encode("日本", false, e).err(), Some('日'));
    }

    #[test]
    fn bom_is_only_written_when_asked() {
        assert_eq!(encode("a", true, UTF_8).unwrap(), b"\xEF\xBB\xBFa");
        assert_eq!(encode("a", true, UTF_16LE).unwrap(), b"\xFF\xFEa\0");
        assert_eq!(encode("a", false, UTF_16BE).unwrap(), b"\0a");
        assert_eq!(
            encode("a", true, by_name("windows-1251").unwrap()).unwrap(),
            b"a"
        );
    }
}
