use fojin_cli::data::{
    ensure_data, gunzip, open_compatible_db, open_read_only_db, resolve_data_path,
    validate_compatibility, verify_dataset, verify_sha256, DataSource, EXPECTED_DATA_VERSION,
    EXPECTED_NORM_RULESET,
};
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
    let src = DataSource {
        url: "https://example.invalid/x.gz",
        sha256: "0",
    };
    let err = ensure_data(&path, true, &src).unwrap_err().to_string();
    assert!(err.contains("offline") || err.contains("手动"));
}

#[test]
fn present_file_is_a_noop() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.sqlite");
    std::fs::write(&path, b"x").unwrap();
    let src = DataSource {
        url: "https://example.invalid/x.gz",
        sha256: "0",
    };
    // must NOT attempt download when file already exists
    assert!(ensure_data(&path, false, &src).is_ok());
}

#[test]
fn write_atomic_writes_content_and_leaves_no_temp() {
    use fojin_cli::data::write_atomic;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.sqlite");
    write_atomic(&path, b"payload").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"payload");
    assert!(
        !path.with_extension("tmp").exists(),
        "temp sibling must not remain"
    );
}

#[test]
fn progress_reports_each_ten_percent_once() {
    let mut p = fojin_cli::data::Progress::new(Some(1000));
    assert!(p.advance(50).is_none()); // 5%
    let msg = p.advance(50).unwrap(); // 10%
    assert!(msg.contains("10%"), "got: {msg}");
    assert!(p.advance(50).is_none()); // 15%: decile already reported
    let msg = p.advance(50).unwrap(); // 20%
    assert!(msg.contains("20%"), "got: {msg}");
    let msg = p.advance(800).unwrap(); // jump to 100%
    assert!(msg.contains("100%"), "got: {msg}");
    assert!(p.advance(100).is_none()); // past 100%: never repeats
}

#[test]
fn progress_message_shows_mb_when_total_known() {
    let mb = 1024 * 1024;
    let mut p = fojin_cli::data::Progress::new(Some(184 * mb));
    let msg = p.advance(19 * mb).unwrap(); // just past 10%
    assert!(msg.contains("19/184 MB"), "got: {msg}");
}

#[test]
fn progress_silent_without_total() {
    let mut p = fojin_cli::data::Progress::new(None);
    assert!(p.advance(u64::MAX / 2).is_none());
    assert!(p.advance(1).is_none());
}

#[test]
fn download_notice_mentions_size_and_offline() {
    let mb = 1024 * 1024;
    let msg = fojin_cli::data::download_notice(Some(184 * mb));
    assert!(msg.contains("184 MB"), "got: {msg}");
    assert!(msg.contains("离线"), "got: {msg}");
    let msg = fojin_cli::data::download_notice(None);
    assert!(msg.contains("下载"), "got: {msg}");
}

/// Serve one HTTP response on localhost and exercise the full download path:
/// stream -> sha256 verify -> gunzip -> atomic write. No external network.
#[test]
fn ensure_data_downloads_verifies_and_unpacks() {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(b"fake sqlite payload").unwrap();
    let gz = enc.finish().unwrap();

    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(&gz);
    let sha: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let body = gz.clone();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut req = [0u8; 4096];
        let _ = std::io::Read::read(&mut stream, &mut req);
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/gzip\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(&body).unwrap();
    });

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.sqlite");
    let source = DataSource {
        url: &format!("http://127.0.0.1:{port}/data.gz"),
        sha256: &sha,
    };
    ensure_data(&path, false, &source).unwrap();
    server.join().unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), b"fake sqlite payload");
    assert!(
        !path.with_extension("tmp").exists(),
        "temp file must not linger"
    );
}

#[test]
fn dataset_stats_reads_meta_and_counts() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    fojin_cli::schema::init_schema(&conn).unwrap();
    for (k, v) in [("version", "v1"), ("license", "CC BY-SA 4.0")] {
        conn.execute(
            "INSERT INTO meta(key,value) VALUES (?1,?2)",
            rusqlite::params![k, v],
        )
        .unwrap();
    }
    for (lang, cb) in [("sa", "T0251"), ("sa", "T0235"), ("bo", "T0251")] {
        conn.execute(
            "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,cbeta_id)
             VALUES ('色','色',?1,'x',?2)",
            rusqlite::params![lang, cb],
        )
        .unwrap();
    }
    let s = fojin_cli::data::dataset_stats(&conn).unwrap();
    assert_eq!(s.version.as_deref(), Some("v1"));
    assert_eq!(s.license.as_deref(), Some("CC BY-SA 4.0"));
    assert_eq!(s.total, 3);
    assert_eq!(s.texts, 2);
    assert_eq!(
        s.by_lang,
        vec![("bo".to_string(), 1), ("sa".to_string(), 2)]
    );
}

#[test]
fn open_read_only_db_rejects_create_table() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.sqlite");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute("CREATE TABLE existing (id INTEGER)", [])
        .unwrap();
    drop(conn);

    let conn = open_read_only_db(&path).unwrap();
    let err = conn
        .execute("CREATE TABLE forbidden (id INTEGER)", [])
        .unwrap_err();

    assert_eq!(err.sqlite_error_code(), Some(rusqlite::ErrorCode::ReadOnly));
}

#[test]
fn compatibility_accepts_expected_dataset_metadata() {
    let conn = compatible_conn();
    let got = validate_compatibility(&conn).unwrap();
    assert_eq!(got.version, EXPECTED_DATA_VERSION);
    assert_eq!(got.norm_ruleset, EXPECTED_NORM_RULESET);
    assert_eq!(
        serde_json::to_value(&got).unwrap(),
        serde_json::json!({
            "version": "v1",
            "norm_ruleset": "t2s-char-1to1-v1"
        })
    );
}

#[test]
fn compatibility_rejects_wrong_version() {
    let conn = compatible_conn();
    conn.execute("UPDATE meta SET value='v0' WHERE key='version'", [])
        .unwrap();

    let err = validate_compatibility(&conn).unwrap_err().to_string();
    assert!(err.contains("version"), "got: {err}");
    assert!(err.contains(EXPECTED_DATA_VERSION), "got: {err}");
    assert!(err.contains("fojin data update"), "got: {err}");
}

#[test]
fn compatibility_rejects_wrong_norm_ruleset() {
    let conn = compatible_conn();
    conn.execute(
        "UPDATE meta SET value='legacy-rules' WHERE key='norm_ruleset'",
        [],
    )
    .unwrap();

    let err = validate_compatibility(&conn).unwrap_err().to_string();
    assert!(err.contains("norm_ruleset"), "got: {err}");
    assert!(err.contains(EXPECTED_NORM_RULESET), "got: {err}");
    assert!(err.contains("fojin data update"), "got: {err}");
}

#[test]
fn compatibility_rejects_missing_required_schema() {
    let conn = compatible_conn();
    conn.execute("DROP TABLE norm_map", []).unwrap();

    let err = validate_compatibility(&conn).unwrap_err().to_string();
    assert!(err.contains("norm_map"), "got: {err}");
    assert!(err.contains("fojin data update"), "got: {err}");
}

#[test]
fn compatibility_rejects_missing_parallels_fts() {
    let conn = compatible_conn();
    conn.execute("DROP TABLE parallels_fts", []).unwrap();

    let err = validate_compatibility(&conn).unwrap_err().to_string();
    assert!(err.contains("parallels_fts"), "got: {err}");
    assert!(err.contains("fojin data update"), "got: {err}");
}

#[test]
fn compatibility_rejects_missing_query_required_parallels_column() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);
         CREATE TABLE norm_map (from_char TEXT PRIMARY KEY, to_char TEXT NOT NULL);
         CREATE TABLE parallels (
             id INTEGER PRIMARY KEY,
             zh_text TEXT NOT NULL,
             zh_norm TEXT NOT NULL,
             foreign_lang TEXT NOT NULL,
             foreign_text TEXT NOT NULL,
             cbeta_id TEXT,
             title_zh TEXT,
             juan_num INTEGER
         );
         CREATE VIRTUAL TABLE parallels_fts USING fts5(
             zh_norm,
             content='parallels',
             content_rowid='id',
             tokenize='trigram'
         );",
    )
    .unwrap();
    insert_compat_meta(&conn);

    let err = validate_compatibility(&conn).unwrap_err().to_string();
    assert!(err.contains("parallels"), "got: {err}");
    assert!(err.contains("fojin data update"), "got: {err}");
}

#[test]
fn compatibility_open_compatible_db_checks_file_before_returning_connection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.sqlite");
    let conn = rusqlite::Connection::open(&path).unwrap();
    fojin_cli::schema::init_schema(&conn).unwrap();
    insert_compat_meta(&conn);
    drop(conn);

    let conn = open_compatible_db(&path).unwrap();
    let got = validate_compatibility(&conn).unwrap();
    assert_eq!(got.version, EXPECTED_DATA_VERSION);
    assert_eq!(got.norm_ruleset, EXPECTED_NORM_RULESET);
}

#[test]
fn verify_dataset_runs_quick_check_on_compatible_db() {
    let conn = compatible_conn();
    let got = verify_dataset(&conn).unwrap();
    assert_eq!(got.version, EXPECTED_DATA_VERSION);
    assert_eq!(got.norm_ruleset, EXPECTED_NORM_RULESET);
}

#[test]
fn verify_dataset_accepts_compatible_read_only_db() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.sqlite");
    let conn = rusqlite::Connection::open(&path).unwrap();
    fojin_cli::schema::init_schema(&conn).unwrap();
    insert_compat_meta(&conn);
    drop(conn);

    let conn = open_read_only_db(&path).unwrap();
    let got = verify_dataset(&conn).unwrap();

    assert_eq!(got.version, EXPECTED_DATA_VERSION);
    assert_eq!(got.norm_ruleset, EXPECTED_NORM_RULESET);
}

fn compatible_conn() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    fojin_cli::schema::init_schema(&conn).unwrap();
    insert_compat_meta(&conn);
    conn
}

fn insert_compat_meta(conn: &rusqlite::Connection) {
    for (key, value) in [
        ("version", EXPECTED_DATA_VERSION),
        ("norm_ruleset", EXPECTED_NORM_RULESET),
    ] {
        conn.execute(
            "INSERT INTO meta(key,value) VALUES (?1,?2)",
            rusqlite::params![key, value],
        )
        .unwrap();
    }
}
