//! The load monitor against a real Linux sshd: numbers come from /proc over SFTP, nothing is executed.
mod common;
use common::*;
use d256shell_core::sftp::SftpConnection;

#[tokio::test(flavor = "multi_thread")]
async fn samples_proc_files_over_sftp() {
    if !enabled() {
        return;
    }
    let d = tmp("mon1");
    let (i, _) = interaction(true, |_| None);
    let c = SftpConnection::connect(params("sshd-password", Some(PASS), &d.join("kh")), i)
        .await
        .expect("connect");
    let a = c.monitor_sample(true).await.expect("sample");
    assert!(a["cpu"][1].as_u64().unwrap() > 0, "{a}");
    assert!(!a["cores"].as_array().unwrap().is_empty());
    assert!(a["mem"]["total"].as_u64().unwrap() > 0);
    assert!(a["mem"]["available"].as_u64().unwrap() <= a["mem"]["total"].as_u64().unwrap());
    assert!(a["uptime"].as_f64().unwrap() > 0.0);
    assert!(a["load"]["procs"].as_u64().unwrap() > 0);
    assert!(a["net"]["rx"].is_u64());
    assert!(
        a["ifaces"].is_array() && a["cpu_mhz"].is_number() && a["mem"]["free"].is_u64(),
        "{a}"
    );
    assert!(
        a["info"]["devices"]["disks"].is_object() && a["info"]["devices"]["ifaces"].is_object(),
        "{a}"
    );
    assert!(a["disks"].is_array(), "{a}"); // /proc/diskstats is read (the list may be empty on idle hosts)
    for d in a["disks"].as_array().unwrap() {
        assert_eq!(d["c"].as_array().unwrap().len(), 8);
    }
    assert!(a["info"]["hostname"].is_string(), "{a}");
    // counters grow between two samples
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let b = c.monitor_sample(false).await.unwrap();
    assert!(b["cpu"][1].as_u64().unwrap() >= a["cpu"][1].as_u64().unwrap());
    assert!(b.get("info").is_none() || b["info"].is_null());
    // a missing file is not an error
    assert!(c
        .read_small("/proc/definitely-not-there", 100)
        .await
        .is_none());
}
