use fojin_cli::data::{ensure_data, gunzip, resolve_data_path, verify_sha256, DataSource};
use std::io::Write;
use std::path::PathBuf;

#[test]
fn resolve_honours_data_dir_override() {
    let p = resolve_data_path(Some(PathBuf::from("/tmp/xyz"))).unwrap();
    assert_eq!(p, PathBuf::from("/tmp/xyz/data.sqlite"));
}

#[test]
fn sha256_matches_known_vector() {
    // sha256("abc")
    let hex = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    assert!(verify_sha256(b"abc", hex));
    assert!(!verify_sha256(b"abc", &"0".repeat(64)));
}

#[test]
fn gunzip_roundtrips() {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(b"hello fojin").unwrap();
    let gz = enc.finish().unwrap();
    assert_eq!(gunzip(&gz).unwrap(), b"hello fojin");
}

#[test]
fn offline_and_missing_errors_clearly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.sqlite"); // does not exist
    let src = DataSource { url: "https://example.invalid/x.gz", sha256: "0" };
    let err = ensure_data(&path, true, &src).unwrap_err().to_string();
    assert!(err.contains("offline") || err.contains("手动"));
}

#[test]
fn present_file_is_a_noop() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.sqlite");
    std::fs::write(&path, b"x").unwrap();
    let src = DataSource { url: "https://example.invalid/x.gz", sha256: "0" };
    // must NOT attempt download when file already exists
    assert!(ensure_data(&path, false, &src).is_ok());
}
