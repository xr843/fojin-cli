use fojin_cli::data::{
    clean_data, ensure_data, gunzip, open_compatible_db, open_read_only_db, resolve_data_path,
    update_data, validate_compatibility, verify_dataset, verify_dataset_file, verify_sha256,
    DataSource, EXPECTED_DATA_VERSION, EXPECTED_NORM_RULESET,
};
use std::io::{Read, Write};
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
fn ensure_data_rechecks_existence_after_waiting_for_lock() {
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("data.sqlite");
    let lock_path = sibling_path(&data, ".lock");
    let blocker = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .unwrap();
    blocker.lock().unwrap();

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let source_url = format!("http://{}/data.gz", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut request = [0_u8; 4096];
                    let _ = std::io::Read::read(&mut stream, &mut request);
                    stream
                        .write_all(
                            b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .unwrap();
                    return 1_usize;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() >= deadline {
                        return 0;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(error) => panic!("accept failed: {error}"),
            }
        }
    });

    let worker_data = data.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let source = DataSource {
            url: &source_url,
            sha256: "unused",
        };
        sender
            .send(ensure_data(&worker_data, false, &source))
            .unwrap();
    });

    let early = receiver.recv_timeout(std::time::Duration::from_millis(100));
    let waited_for_lock = matches!(&early, Err(std::sync::mpsc::RecvTimeoutError::Timeout));
    std::fs::write(&data, b"installed by competing process").unwrap();
    drop(blocker);

    let result = match early {
        Ok(result) => result,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("ensure_data remained blocked after the lock was released"),
        Err(error) => panic!("ensure_data worker disconnected: {error}"),
    };
    worker.join().unwrap();
    let requests = server.join().unwrap();
    assert!(
        waited_for_lock,
        "ensure_data did not wait for the operation lock: {result:?}"
    );
    assert_eq!(requests, 0, "lock waiter attempted a redundant download");
    result.unwrap();
    assert_eq!(
        std::fs::read(data).unwrap(),
        b"installed by competing process"
    );
}

#[test]
fn clean_data_waits_for_operation_lock_before_deleting() {
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("data.sqlite");
    std::fs::write(&data, b"live").unwrap();
    let lock_path = sibling_path(&data, ".lock");
    let blocker = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap();
    blocker.lock().unwrap();

    let worker_data = data.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        sender.send(clean_data(&worker_data)).unwrap();
    });

    assert!(matches!(
        receiver.recv_timeout(std::time::Duration::from_millis(100)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(data.is_file(), "clean deleted live data without the lock");
    drop(blocker);

    assert_eq!(
        receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("clean remained blocked after the lock was released")
            .unwrap(),
        Some(4)
    );
    worker.join().unwrap();
    assert!(!data.exists());
    assert!(lock_path.is_file());
}

#[test]
fn clean_data_removes_known_file_artifacts_and_preserves_other_entries() {
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("data.sqlite");
    let legacy = data.with_extension("tmp");
    let download = sibling_path(&data, ".download.123.0.gz");
    let candidate = sibling_path(&data, ".candidate.123.1");
    let candidate_journal = sibling_path(&data, ".candidate.123.1-journal");
    let matching_directory = sibling_path(&data, ".download.keep");
    let lock_path = sibling_path(&data, ".lock");
    let unrelated = sibling_path(&data, ".unrelated");
    std::fs::write(&data, b"live").unwrap();
    for artifact in [&legacy, &download, &candidate, &candidate_journal] {
        std::fs::write(artifact, b"temporary").unwrap();
    }
    std::fs::create_dir(&matching_directory).unwrap();
    std::fs::write(&lock_path, b"permanent lock inode").unwrap();
    std::fs::write(&unrelated, b"unrelated").unwrap();

    assert_eq!(clean_data(&data).unwrap(), Some(4));
    for removed in [&data, &legacy, &download, &candidate, &candidate_journal] {
        assert!(!removed.exists(), "artifact remains: {}", removed.display());
    }
    assert!(matching_directory.is_dir());
    assert_eq!(std::fs::read(&lock_path).unwrap(), b"permanent lock inode");
    assert_eq!(std::fs::read(&unrelated).unwrap(), b"unrelated");

    assert_eq!(clean_data(&data).unwrap(), None);
    assert!(lock_path.is_file());
    assert!(matching_directory.is_dir());
    assert!(unrelated.is_file());
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
/// stream -> sha256 verify -> bounded gunzip -> verified publication. No external network.
#[test]
fn ensure_data_downloads_verifies_and_unpacks() {
    let gz = gzip_bytes(&replacement_database_bytes());
    let sha = sha256_hex(&gz);

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

    verify_dataset_file(&path).unwrap();
}

#[test]
fn concurrent_first_install_downloads_once() {
    if std::env::var_os("FOJIN_CONCURRENT_WORKER").is_some() {
        let path = PathBuf::from(std::env::var_os("FOJIN_WORKER_DATA").unwrap());
        let url = std::env::var("FOJIN_WORKER_URL").unwrap();
        let sha = std::env::var("FOJIN_WORKER_SHA256").unwrap();
        ensure_data(
            &path,
            false,
            &DataSource {
                url: &url,
                sha256: &sha,
            },
        )
        .unwrap();
        return;
    }
    use std::process::Command;
    use std::time::{Duration, Instant};

    let directory = tempfile::tempdir().unwrap();
    let data_path = directory.path().join("data.sqlite");
    let body = gzip_bytes(&replacement_database_bytes());
    let sha = sha256_hex(&body);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/data.gz", listener.local_addr().unwrap());
    let server_body = body.clone();
    let server = std::thread::spawn(move || {
        let mut requests = 0_usize;
        let (mut first, _) = listener.accept().unwrap();
        requests += 1;
        let mut request = [0_u8; 4096];
        let _ = first.read(&mut request);
        write!(
            first,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            server_body.len()
        )
        .unwrap();
        let midpoint = server_body.len() / 2;
        first.write_all(&server_body[..midpoint]).unwrap();
        first.flush().unwrap();
        std::thread::sleep(Duration::from_millis(200));
        first.write_all(&server_body[midpoint..]).unwrap();
        drop(first);

        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    requests += 1;
                    let _ = stream.read(&mut request);
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        server_body.len()
                    )
                    .unwrap();
                    stream.write_all(&server_body).unwrap();
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("accept failed: {error}"),
            }
        }
        requests
    });

    let spawn_worker = || {
        Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("concurrent_first_install_downloads_once")
            .arg("--nocapture")
            .env("FOJIN_CONCURRENT_WORKER", "1")
            .env("FOJIN_WORKER_DATA", &data_path)
            .env("FOJIN_WORKER_URL", &url)
            .env("FOJIN_WORKER_SHA256", &sha)
            .spawn()
            .unwrap()
    };
    let mut first = spawn_worker();
    let mut second = spawn_worker();
    assert!(first.wait().unwrap().success());
    assert!(second.wait().unwrap().success());
    assert_eq!(server.join().unwrap(), 1);
    verify_dataset_file(&data_path).unwrap();
    assert_no_owned_candidate_artifacts(&data_path);
}

#[test]
fn first_install_rejects_incompatible_database() {
    let gz = gzip_bytes(b"fake sqlite payload");
    let sha = sha256_hex(&gz);

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let body = gz.clone();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 4096];
        let _ = std::io::Read::read(&mut stream, &mut request);
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
    let error = ensure_data(&path, false, &source).unwrap_err().to_string();
    server.join().unwrap();

    assert!(error.contains("dataset incompatibility"), "got: {error}");
    assert!(!path.exists(), "incompatible dataset was published");
    assert_no_owned_candidate_artifacts(&path);
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
fn compatibility_rejects_parallels_fts_lookalike_table() {
    let conn = compatible_conn();
    conn.execute("DROP TABLE parallels_fts", []).unwrap();
    conn.execute("CREATE TABLE parallels_fts (zh_norm TEXT NOT NULL)", [])
        .unwrap();

    let err = validate_compatibility(&conn).unwrap_err().to_string();

    assert!(err.contains("parallels_fts"), "got: {err}");
    assert!(err.contains("fojin data update"), "got: {err}");
}

#[test]
fn compatibility_rejects_non_trigram_parallels_fts() {
    let conn = compatible_conn();
    conn.execute("DROP TABLE parallels_fts", []).unwrap();
    conn.execute(
        "CREATE VIRTUAL TABLE parallels_fts USING fts5(zh_norm, content='parallels', content_rowid='id')",
        [],
    )
    .unwrap();

    let err = validate_compatibility(&conn).unwrap_err().to_string();

    assert!(err.contains("parallels_fts"), "got: {err}");
    assert!(err.contains("fojin data update"), "got: {err}");
}

#[test]
fn compatibility_rejects_default_tokenizer_fts_spoofing_trigram_declaration() {
    let conn = compatible_conn();
    conn.execute("DROP TABLE parallels_fts", []).unwrap();
    conn.execute(
        "CREATE VIRTUAL TABLE parallels_fts USING fts5(
             zh_norm,
             \"tokenize='trigram'\",
             content='parallels',
             content_rowid='id'
         )",
        [],
    )
    .unwrap();

    let err = validate_compatibility(&conn).unwrap_err().to_string();

    assert!(err.contains("parallels_fts"), "got: {err}");
    assert!(err.contains("fojin data update"), "got: {err}");
}

#[test]
fn compatibility_rejects_parallels_fts_view_lookalike() {
    let conn = compatible_conn();
    conn.execute("DROP TABLE parallels_fts", []).unwrap();
    conn.execute(
        "CREATE VIEW parallels_fts AS SELECT id AS rowid, zh_norm FROM parallels",
        [],
    )
    .unwrap();

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

#[test]
fn verify_dataset_rejects_quick_check_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.sqlite");
    let conn = rusqlite::Connection::open(&path).unwrap();
    fojin_cli::schema::init_schema(&conn).unwrap();
    insert_compat_meta(&conn);
    conn.execute("CREATE TABLE quick_check_probe (value TEXT)", [])
        .unwrap();
    let meta_rootpage: i64 = conn
        .query_row(
            "SELECT rootpage FROM sqlite_schema WHERE name = 'meta'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    conn.execute_batch("PRAGMA writable_schema = ON").unwrap();
    conn.execute(
        "UPDATE sqlite_schema SET rootpage = ?1 WHERE name = 'quick_check_probe'",
        [meta_rootpage],
    )
    .unwrap();
    conn.execute_batch("PRAGMA writable_schema = OFF").unwrap();
    drop(conn);

    let conn = open_read_only_db(&path).unwrap();
    let err = verify_dataset(&conn).unwrap_err().to_string();

    assert!(err.contains("PRAGMA quick_check failed"), "got: {err}");
    assert!(err.contains("2nd reference to page"), "got: {err}");
}

#[test]
fn update_data_preserves_live_dataset_when_candidate_fails_compatibility() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.sqlite");
    std::fs::write(&path, b"live dataset").unwrap();

    let source_path = dir.path().join("candidate.sqlite");
    let conn = rusqlite::Connection::open(&source_path).unwrap();
    fojin_cli::schema::init_schema(&conn).unwrap();
    insert_compat_meta(&conn);
    conn.execute("UPDATE meta SET value = 'v0' WHERE key = 'version'", [])
        .unwrap();
    drop(conn);

    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(&std::fs::read(&source_path).unwrap())
        .unwrap();
    let gz = enc.finish().unwrap();
    std::fs::remove_file(&source_path).unwrap();

    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(&gz);
    let sha: String = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 4096];
        let _ = std::io::Read::read(&mut stream, &mut request);
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            gz.len()
        );
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(&gz).unwrap();
    });
    let source_url = format!("http://127.0.0.1:{port}/data.gz");
    let source = DataSource {
        url: &source_url,
        sha256: &sha,
    };

    let err = update_data(&path, &source).unwrap_err().to_string();
    server.join().unwrap();

    assert!(err.contains("dataset incompatibility"), "got: {err}");
    assert_eq!(std::fs::read(&path).unwrap(), b"live dataset");
    let entries: std::collections::BTreeSet<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    let expected: std::collections::BTreeSet<_> = ["data.sqlite", "data.sqlite.lock"]
        .into_iter()
        .map(std::ffi::OsString::from)
        .collect();
    assert_eq!(entries, expected);
}

#[test]
fn update_data_replaces_valid_existing_dataset() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.sqlite");
    std::fs::write(&path, b"old live dataset").unwrap();
    let gz = gzip_bytes(&replacement_database_bytes());
    let sha = sha256_hex(&gz);

    serve_update(&path, gz, &sha).unwrap();

    assert_replacement_marker(&path);
    assert_no_candidate_artifacts(&path);
}

#[test]
fn update_download_keeps_live_dataset_readable_without_query_locking() {
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("data.sqlite");
    let live = compatible_database_bytes(|connection| {
        connection
            .execute("INSERT INTO meta(key, value) VALUES ('marker', 'live')", [])
            .unwrap();
    });
    std::fs::write(&data, live).unwrap();

    let replacement = gzip_bytes(&replacement_database_bytes());
    let sha = sha256_hex(&replacement);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let source_url = format!("http://{}/data.gz", listener.local_addr().unwrap());
    let (midpoint_sender, midpoint_receiver) = std::sync::mpsc::channel();
    let (release_sender, release_receiver) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let _ = std::io::Read::read(&mut stream, &mut request);
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            replacement.len()
        )
        .unwrap();
        let midpoint = replacement.len() / 2;
        stream.write_all(&replacement[..midpoint]).unwrap();
        stream.flush().unwrap();
        midpoint_sender.send(()).unwrap();
        release_receiver.recv().unwrap();
        stream.write_all(&replacement[midpoint..]).unwrap();
    });

    let update_path = data.clone();
    let updater = std::thread::spawn(move || {
        update_data(
            &update_path,
            &DataSource {
                url: &source_url,
                sha256: &sha,
            },
        )
    });
    midpoint_receiver
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("updater did not begin its download");

    let lock_path = sibling_path(&data, ".lock");
    let lock_probe = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .unwrap();
    let update_holds_lock = match lock_probe.try_lock() {
        Err(std::fs::TryLockError::WouldBlock) => true,
        Ok(()) => false,
        Err(std::fs::TryLockError::Error(error)) => {
            panic!("probing the update operation lock failed: {error}")
        }
    };
    drop(lock_probe);

    let query_path = data.clone();
    let (query_sender, query_receiver) = std::sync::mpsc::channel();
    let query = std::thread::spawn(move || {
        let result = (|| {
            let connection = open_compatible_db(&query_path)?;
            let marker =
                connection.query_row("SELECT value FROM meta WHERE key = 'marker'", [], |row| {
                    row.get::<_, String>(0)
                })?;
            Ok::<_, anyhow::Error>(marker)
        })();
        query_sender.send(result).unwrap();
    });
    let live_marker = query_receiver
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("ordinary query waited for the data operation lock")
        .unwrap();
    query.join().unwrap();

    release_sender.send(()).unwrap();
    server.join().unwrap();
    let update_result = updater.join().unwrap();
    assert!(
        update_holds_lock,
        "update_data did not hold the operation lock while downloading"
    );
    assert_eq!(live_marker, "live");
    update_result.unwrap();
    assert_replacement_marker(&data);
}

#[test]
fn update_data_installs_valid_dataset_when_target_is_absent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.sqlite");
    let gz = gzip_bytes(&replacement_database_bytes());
    let sha = sha256_hex(&gz);

    serve_update(&path, gz, &sha).unwrap();

    assert_replacement_marker(&path);
    assert_no_candidate_artifacts(&path);
}

#[test]
fn update_data_preserves_live_dataset_on_download_failure() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.sqlite");
    std::fs::write(&path, b"old live dataset").unwrap();

    let err = serve_update_response(
        &path,
        "500 Internal Server Error",
        b"download failed".to_vec(),
        "unused",
    )
    .unwrap_err()
    .to_string();

    assert!(err.contains("下载失败"), "got: {err}");
    assert_eq!(std::fs::read(&path).unwrap(), b"old live dataset");
    assert_no_candidate_artifacts(&path);
}

#[test]
fn update_data_preserves_live_dataset_on_checksum_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.sqlite");
    std::fs::write(&path, b"old live dataset").unwrap();
    let gz = gzip_bytes(&replacement_database_bytes());

    let err = serve_update(&path, gz, &"0".repeat(64))
        .unwrap_err()
        .to_string();

    assert!(err.contains("sha256"), "got: {err}");
    assert_eq!(std::fs::read(&path).unwrap(), b"old live dataset");
    assert_no_candidate_artifacts(&path);
}

#[test]
fn update_data_preserves_live_dataset_on_decompression_failure() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.sqlite");
    std::fs::write(&path, b"old live dataset").unwrap();
    let invalid_gzip = b"not gzip data".to_vec();
    let sha = sha256_hex(&invalid_gzip);

    let err = serve_update(&path, invalid_gzip, &sha)
        .unwrap_err()
        .to_string();

    assert!(err.contains("解压 gzip"), "got: {err}");
    assert_eq!(std::fs::read(&path).unwrap(), b"old live dataset");
    assert_no_candidate_artifacts(&path);
}

#[test]
fn update_data_preserves_live_dataset_when_candidate_is_corrupt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.sqlite");
    std::fs::write(&path, b"old live dataset").unwrap();
    let database = compatible_database_bytes(|conn| {
        conn.execute("CREATE TABLE quick_check_probe (value TEXT)", [])
            .unwrap();
        let meta_rootpage: i64 = conn
            .query_row(
                "SELECT rootpage FROM sqlite_schema WHERE name = 'meta'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        conn.execute_batch("PRAGMA writable_schema = ON").unwrap();
        conn.execute(
            "UPDATE sqlite_schema SET rootpage = ?1 WHERE name = 'quick_check_probe'",
            [meta_rootpage],
        )
        .unwrap();
        conn.execute_batch("PRAGMA writable_schema = OFF").unwrap();
    });
    let gz = gzip_bytes(&database);
    let sha = sha256_hex(&gz);

    let err = serve_update(&path, gz, &sha).unwrap_err().to_string();

    assert!(err.contains("PRAGMA quick_check failed"), "got: {err}");
    assert_eq!(std::fs::read(&path).unwrap(), b"old live dataset");
    assert_no_candidate_artifacts(&path);
}

#[test]
fn update_data_rejects_candidate_with_empty_external_content_fts_index() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.sqlite");
    std::fs::write(&path, b"old live dataset").unwrap();
    let database = compatible_database_bytes(|conn| {
        conn.execute(
            "INSERT INTO parallels(zh_text, zh_norm, foreign_lang, foreign_text) \
             VALUES ('色即是空', '色即是空', 'sa', 'rupam sunyata')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO parallels_fts(parallels_fts) VALUES('delete-all')",
            [],
        )
        .unwrap();
    });
    let gz = gzip_bytes(&database);
    let sha = sha256_hex(&gz);

    let err = serve_update(&path, gz, &sha).unwrap_err().to_string();

    assert!(err.contains("FTS5 integrity-check"), "got: {err}");
    assert_eq!(std::fs::read(&path).unwrap(), b"old live dataset");
    assert_no_candidate_artifacts(&path);
}

#[test]
fn update_data_preserves_foreign_candidate_artifacts_and_cleans_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.sqlite");
    std::fs::write(&path, b"old live dataset").unwrap();
    let foreign_candidate = sibling_path(&path, ".candidate.999999.42");
    for suffix in ["", "-journal", "-shm", "-wal"] {
        std::fs::write(sibling_path(&foreign_candidate, suffix), b"foreign").unwrap();
    }
    let database = compatible_database_bytes(|conn| {
        conn.execute("UPDATE meta SET value = 'v0' WHERE key = 'version'", [])
            .unwrap();
    });
    let gz = gzip_bytes(&database);
    let sha = sha256_hex(&gz);

    serve_update(&path, gz, &sha).unwrap_err();

    assert_eq!(std::fs::read(&path).unwrap(), b"old live dataset");
    for suffix in ["", "-journal", "-shm", "-wal"] {
        assert_eq!(
            std::fs::read(sibling_path(&foreign_candidate, suffix)).unwrap(),
            b"foreign"
        );
    }
    let candidate_prefix = format!("{}.candidate.", path.file_name().unwrap().to_string_lossy());
    let remaining: std::collections::BTreeSet<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name.to_string_lossy().starts_with(&candidate_prefix))
        .collect();
    let expected: std::collections::BTreeSet<_> = ["", "-journal", "-shm", "-wal"]
        .into_iter()
        .map(|suffix| {
            sibling_path(&foreign_candidate, suffix)
                .file_name()
                .unwrap()
                .to_os_string()
        })
        .collect();
    assert_eq!(remaining, expected);
}

#[test]
fn update_data_replaces_live_dataset_without_backup_path_dependency() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.sqlite");
    std::fs::write(&path, b"old live dataset").unwrap();
    let backup_sentinel = dir.path().join("data.sqlite.backup");
    std::fs::create_dir(&backup_sentinel).unwrap();

    let database = compatible_database_bytes(|conn| {
        conn.execute(
            "INSERT INTO meta(key, value) VALUES ('marker', 'replacement')",
            [],
        )
        .unwrap();
    });
    let gz = gzip_bytes(&database);
    let sha = sha256_hex(&gz);

    serve_update(&path, gz, &sha).unwrap();

    let conn = open_read_only_db(&path).unwrap();
    verify_dataset(&conn).unwrap();
    let marker: String = conn
        .query_row("SELECT value FROM meta WHERE key = 'marker'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(marker, "replacement");
    assert!(backup_sentinel.is_dir());
    assert_no_candidate_artifacts(&path);
}

fn replacement_database_bytes() -> Vec<u8> {
    compatible_database_bytes(|conn| {
        conn.execute(
            "INSERT INTO meta(key, value) VALUES ('marker', 'replacement')",
            [],
        )
        .unwrap();
    })
}

fn assert_replacement_marker(path: &std::path::Path) {
    let conn = open_read_only_db(path).unwrap();
    verify_dataset(&conn).unwrap();
    let marker: String = conn
        .query_row("SELECT value FROM meta WHERE key = 'marker'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(marker, "replacement");
}

fn assert_no_candidate_artifacts(path: &std::path::Path) {
    let candidate = sibling_path(path, ".candidate");
    for suffix in ["", "-journal", "-shm", "-wal"] {
        let artifact = sibling_path(&candidate, suffix);
        assert!(
            !artifact.exists(),
            "artifact remains: {}",
            artifact.display()
        );
    }
    assert_no_owned_candidate_artifacts(path);
}

fn assert_no_owned_candidate_artifacts(path: &std::path::Path) {
    let mut prefix = path.file_name().unwrap().to_os_string();
    prefix.push(".candidate.");
    let prefix = prefix.to_string_lossy();
    let owned: Vec<_> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name.to_string_lossy().starts_with(prefix.as_ref()))
        .collect();
    assert!(
        owned.is_empty(),
        "owned candidate artifacts remain: {owned:?}"
    );
}

fn sibling_path(path: &std::path::Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

fn compatible_database_bytes(configure: impl FnOnce(&rusqlite::Connection)) -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("candidate.sqlite");
    let conn = rusqlite::Connection::open(&path).unwrap();
    fojin_cli::schema::init_schema(&conn).unwrap();
    insert_compat_meta(&conn);
    configure(&conn);
    drop(conn);
    std::fs::read(path).unwrap()
}

fn gzip_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(bytes);
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn serve_update(
    path: &std::path::Path,
    body: Vec<u8>,
    expected_sha256: &str,
) -> anyhow::Result<()> {
    serve_update_response(path, "200 OK", body, expected_sha256)
}

fn serve_update_response(
    path: &std::path::Path,
    status: &'static str,
    body: Vec<u8>,
    expected_sha256: &str,
) -> anyhow::Result<()> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 4096];
        let _ = std::io::Read::read(&mut stream, &mut request);
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len(),
        );
        stream.write_all(head.as_bytes()).unwrap();
        let _ = stream.write_all(&body);
    });
    let source_url = format!("http://127.0.0.1:{port}/data.gz");
    let source = DataSource {
        url: &source_url,
        sha256: expected_sha256,
    };

    let result = update_data(path, &source);
    server.join().unwrap();
    result
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
