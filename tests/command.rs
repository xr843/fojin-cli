use fojin_cli::schema::init_schema;
use rusqlite::Connection;

fn write_offline_db(dir: &std::path::Path) {
    let db_path = dir.join("data.sqlite");
    let conn = Connection::open(db_path).unwrap();
    init_schema(&conn).unwrap();
    for (k, v) in [("version", "v1"), ("norm_ruleset", "t2s-char-1to1-v1")] {
        conn.execute(
            "INSERT INTO meta(key,value) VALUES (?1,?2)",
            rusqlite::params![k, v],
        )
        .unwrap();
    }
}

#[test]
fn one_character_query_fails_before_missing_offline_data() {
    let dir = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args(["parallel", "佛", "--offline", "--data-dir"])
        .arg(dir.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("至少需要 2 个汉字"));
    assert!(!stderr.contains("本地数据不存在"));
}

#[test]
fn punctuation_only_query_keeps_empty_result_contract() {
    let dir = tempfile::tempdir().unwrap();
    write_offline_db(dir.path());

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args(["parallel", "，。！？", "--offline", "--data-dir"])
        .arg(dir.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8(output.stderr).unwrap(), "");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "未找到对齐"
    );
}

#[test]
fn zero_top_is_rejected_during_clap_parsing_before_data_access() {
    let dir = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args(["parallel", "般若", "--top", "0", "--offline", "--data-dir"])
        .arg(dir.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("--top <TOP>"));
    assert!(stderr.contains("1.."));
    assert!(!stderr.contains("本地数据不存在"));
}

#[test]
fn zero_limit_is_rejected_during_clap_parsing_before_data_access() {
    let dir = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args([
            "parallel",
            "般若",
            "--limit",
            "0",
            "--offline",
            "--data-dir",
        ])
        .arg(dir.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("--limit <LIMIT>"));
    assert!(stderr.contains("1.."));
    assert!(!stderr.contains("本地数据不存在"));
}

fn write_fixture_db(dir: &std::path::Path) {
    write_fixture_db_with_fts(dir, true);
}

fn write_fixture_db_with_fts(dir: &std::path::Path, rebuild_fts: bool) {
    let conn = Connection::open(dir.join("data.sqlite")).unwrap();
    init_schema(&conn).unwrap();
    for (k, v) in [
        ("version", "v1"),
        ("norm_ruleset", "t2s-char-1to1-v1"),
        ("license", "CC BY-SA 4.0"),
    ] {
        conn.execute(
            "INSERT INTO meta(key,value) VALUES (?1,?2)",
            rusqlite::params![k, v],
        )
        .unwrap();
    }
    let rows = [
        (
            "觀自在菩薩",
            "观自在菩萨",
            "sa",
            "āryāvalokiteśvaro",
            0.95,
            "T0251",
            "般若波羅蜜多心經",
            1,
        ),
        (
            "色即是空",
            "色即是空",
            "sa",
            "rūpaṃ śūnyatā",
            0.91,
            "T0251",
            "般若波羅蜜多心經",
            1,
        ),
        (
            "色即是空",
            "色即是空",
            "bo",
            "gzugs stong pa",
            0.88,
            "T0251",
            "般若波羅蜜多心經",
            1,
        ),
    ];
    for (zt, zn, lang, f, c, cb, ti, j) in rows {
        conn.execute(
            "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            rusqlite::params![zt, zn, lang, f, c, cb, ti, j],
        ).unwrap();
    }
    for (from, to) in [("經", "经"), ("觀", "观")] {
        conn.execute(
            "INSERT INTO norm_map(from_char,to_char) VALUES (?1,?2)",
            rusqlite::params![from, to],
        )
        .unwrap();
    }
    if rebuild_fts {
        conn.execute(
            "INSERT INTO parallels_fts(parallels_fts) VALUES('rebuild')",
            [],
        )
        .unwrap();
    } else {
        conn.execute(
            "INSERT INTO parallels_fts(parallels_fts) VALUES('delete-all')",
            [],
        )
        .unwrap();
    }
}

fn run_fojin(args: &[&str], dir: &std::path::Path) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args(args)
        .args(["--data-dir"])
        .arg(dir)
        .output()
        .unwrap()
}

#[test]
fn data_status_json_reports_dataset() {
    let dir = tempfile::tempdir().unwrap();
    write_fixture_db(dir.path());
    let out = run_fojin(&["data", "status", "--json"], dir.path());
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("\"version\": \"v1\""), "got: {stdout}");
    assert!(stdout.contains("\"total\": 3"), "got: {stdout}");
    assert!(stdout.contains("\"exists\": true"), "got: {stdout}");
}

#[test]
fn data_verify_json_reports_exact_compatibility_shape() {
    let dir = tempfile::tempdir().unwrap();
    write_fixture_db(dir.path());

    let out = run_fojin(&["data", "verify", "--json"], dir.path());

    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    let got: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        got,
        serde_json::json!({
            "ok": true,
            "version": "v1",
            "norm_ruleset": "t2s-char-1to1-v1"
        })
    );
}

#[test]
fn data_verify_human_reports_compatibility_summary() {
    let dir = tempfile::tempdir().unwrap();
    write_fixture_db(dir.path());

    let out = run_fojin(&["data", "verify"], dir.path());

    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.starts_with("数据校验通过"), "got: {stdout}");
    assert!(stdout.contains("v1"), "got: {stdout}");
    assert!(stdout.contains("t2s-char-1to1-v1"), "got: {stdout}");
}

#[test]
fn data_verify_rejects_empty_external_content_fts_index() {
    let dir = tempfile::tempdir().unwrap();
    write_fixture_db_with_fts(dir.path(), false);

    let out = run_fojin(&["data", "verify"], dir.path());

    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stdout.trim().is_empty(), "got stdout: {stdout}");
    assert!(stderr.contains("FTS5 integrity-check"), "got: {stderr}");
}

#[test]
fn data_verify_does_not_modify_database_bytes() {
    let dir = tempfile::tempdir().unwrap();
    write_fixture_db(dir.path());
    let path = dir.path().join("data.sqlite");
    let before = std::fs::read(&path).unwrap();

    let out = run_fojin(&["data", "verify"], dir.path());

    assert_eq!(out.status.code(), Some(0));
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[test]
fn data_verify_missing_data_exits_one_without_creating_file() {
    let dir = tempfile::tempdir().unwrap();
    let data_path = dir.path().join("data.sqlite");

    let out = run_fojin(&["data", "verify"], dir.path());

    assert_eq!(out.status.code(), Some(1));
    assert!(
        !data_path.exists(),
        "data verify must not create data.sqlite"
    );
}

#[test]
fn incompatible_version_is_rejected_before_parallel_query() {
    let dir = tempfile::tempdir().unwrap();
    write_fixture_db(dir.path());
    let conn = Connection::open(dir.path().join("data.sqlite")).unwrap();
    conn.execute("UPDATE meta SET value='v0' WHERE key='version'", [])
        .unwrap();

    let out = run_fojin(&["parallel", "色即是空", "--offline"], dir.path());

    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stdout.trim().is_empty(), "got stdout: {stdout}");
    assert!(stderr.contains("dataset incompatibility"), "got: {stderr}");
    assert!(stderr.contains("version"), "got: {stderr}");
}

#[test]
fn data_status_reports_missing_data_without_downloading() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_fojin(&["data", "status"], dir.path());
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("未下载"), "got: {stdout}");
}

#[test]
fn data_clean_removes_file_and_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    write_fixture_db(dir.path());
    let lock_path = dir.path().join("data.sqlite.lock");
    let out = run_fojin(&["data", "clean"], dir.path());
    assert_eq!(out.status.code(), Some(0));
    assert!(!dir.path().join("data.sqlite").exists());
    assert!(lock_path.is_file());
    let out2 = run_fojin(&["data", "clean"], dir.path());
    assert_eq!(out2.status.code(), Some(0));
    assert!(String::from_utf8(out2.stdout).unwrap().contains("无数据"));
    assert!(lock_path.is_file());
}

#[test]
fn texts_finds_title_via_simplified_keyword() {
    let dir = tempfile::tempdir().unwrap();
    write_fixture_db(dir.path());
    let out = run_fojin(&["texts", "心经", "--offline"], dir.path());
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("T0251"), "got: {stdout}");
    assert!(stdout.contains("般若波羅蜜多心經"), "got: {stdout}");
}

#[test]
fn cite_lists_text_groups_as_json() {
    let dir = tempfile::tempdir().unwrap();
    write_fixture_db(dir.path());
    let out = run_fojin(&["cite", "T0251", "--json", "--offline"], dir.path());
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("觀自在菩薩"), "got: {stdout}");
    assert!(stdout.contains("\"total\": 2"), "got: {stdout}");
}

#[test]
fn cite_unknown_id_suggests_texts_lookup() {
    let dir = tempfile::tempdir().unwrap();
    write_fixture_db(dir.path());
    let out = run_fojin(&["cite", "T9999", "--offline"], dir.path());
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("fojin texts"), "got: {stdout}");
}

#[test]
fn unknown_from_lang_is_a_usage_error_before_touching_data() {
    let dir = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args([
            "parallel",
            "śūnyatā",
            "--from",
            "sk",
            "--offline",
            "--data-dir",
        ])
        .arg(dir.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("未知语种 `sk`"), "got: {stderr}");
    assert!(
        !stderr.contains("本地数据不存在"),
        "validation must precede data access: {stderr}"
    );
}

#[test]
fn unknown_from_lang_outranks_the_query_length_preflight() {
    // A one-character query also fails the length pre-flight, which exits 1
    // with advice about 汉字 — nonsense for a reverse query and the wrong exit
    // code for a bad flag. Usage errors must be reported first.
    let dir = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args(["parallel", "ś", "--from", "sk", "--offline", "--data-dir"])
        .arg(dir.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("未知语种 `sk`"), "got: {stderr}");
    assert!(
        !stderr.contains("至少需要 2 个汉字"),
        "the length pre-flight must not preempt the usage error: {stderr}"
    );
}

#[test]
fn unknown_display_lang_outranks_the_query_length_preflight() {
    let dir = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args(["parallel", "色", "--lang", "sk", "--offline", "--data-dir"])
        .arg(dir.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("未知语种 `sk`"), "got: {stderr}");
    assert!(
        !stderr.contains("至少需要 2 个汉字"),
        "the length pre-flight must not preempt the usage error: {stderr}"
    );
}

#[test]
fn single_char_query_is_still_a_runtime_error_when_flags_are_valid() {
    // The reorder must not weaken the pre-flight itself: with nothing else
    // wrong, a one-character query still fails before any data is opened.
    let dir = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args(["parallel", "色", "--offline", "--data-dir"])
        .arg(dir.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("至少需要 2 个汉字"), "got: {stderr}");
    assert!(
        !stderr.contains("本地数据不存在"),
        "the pre-flight must still precede data access: {stderr}"
    );
}

#[test]
fn unknown_display_lang_is_a_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args([
            "parallel",
            "色即是空",
            "--lang",
            "sk",
            "--offline",
            "--data-dir",
        ])
        .arg(dir.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("未知语种 `sk`"));
}

#[test]
fn from_with_no_split_is_a_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args([
            "parallel",
            "śūnyatā",
            "--from",
            "sa",
            "--no-split",
            "--offline",
            "--data-dir",
        ])
        .arg(dir.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("--from 不做切分"));
}

#[test]
fn short_reverse_query_is_a_runtime_error() {
    let dir = tempfile::tempdir().unwrap();
    write_offline_db(dir.path());

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args(["parallel", "ka", "--from", "sa", "--offline", "--data-dir"])
        .arg(dir.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("至少需要 3 个字符"));
}

fn write_split_fixture(dir: &std::path::Path) {
    let db_path = dir.join("data.sqlite");
    let conn = Connection::open(db_path).unwrap();
    init_schema(&conn).unwrap();
    for (k, v) in [("version", "v1"), ("norm_ruleset", "t2s-char-1to1-v1")] {
        conn.execute(
            "INSERT INTO meta(key,value) VALUES (?1,?2)",
            rusqlite::params![k, v],
        )
        .unwrap();
    }
    for (zt, zn, f) in [
        ("色即是空", "色即是空", "rūpaṃ śūnyatā"),
        ("受想行識", "受想行识", "vedanā saṃjñā"),
    ] {
        conn.execute(
            "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
             VALUES (?1,?2,'sa',?3,1.0,'T0251','心經',1)",
            rusqlite::params![zt, zn, f],
        )
        .unwrap();
    }
}

#[test]
fn long_query_splits_and_stdout_stays_pure_json() {
    let dir = tempfile::tempdir().unwrap();
    write_split_fixture(dir.path());

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args([
            "parallel",
            "色即是空，受想行識",
            "--json",
            "--offline",
            "--data-dir",
        ])
        .arg(dir.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("stdout must be pure JSON");
    assert_eq!(v["schema_version"], serde_json::json!(1));
    assert_eq!(v["segments"].as_array().unwrap().len(), 2);
}

#[test]
fn reverse_lookup_finds_chinese_from_sanskrit() {
    let dir = tempfile::tempdir().unwrap();
    write_split_fixture(dir.path());

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args([
            "parallel",
            "śūnyatā",
            "--from",
            "sa",
            "--json",
            "--offline",
            "--data-dir",
        ])
        .arg(dir.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(output.stdout).unwrap()).unwrap();
    assert_eq!(v["matched"], serde_json::json!(true));
    assert_eq!(v["groups"][0]["zh_text"], serde_json::json!("色即是空"));
}

fn write_cite_fixture(dir: &std::path::Path) {
    let db_path = dir.join("data.sqlite");
    let conn = Connection::open(db_path).unwrap();
    init_schema(&conn).unwrap();
    for (k, v) in [("version", "v1"), ("norm_ruleset", "t2s-char-1to1-v1")] {
        conn.execute(
            "INSERT INTO meta(key,value) VALUES (?1,?2)",
            rusqlite::params![k, v],
        )
        .unwrap();
    }
    for (zt, zn, lang, f, juan) in [
        ("色即是空", "色即是空", "sa", "rūpaṃ śūnyatā", 1),
        ("受想行識", "受想行识", "bo", "gzugs stong pa", 2),
    ] {
        conn.execute(
            "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
             VALUES (?1,?2,?3,?4,0.9,'T0251','心經',?5)",
            rusqlite::params![zt, zn, lang, f, juan],
        )
        .unwrap();
    }
}

fn index_exists(dir: &std::path::Path) -> bool {
    let conn = Connection::open(dir.join("data.sqlite")).unwrap();
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_parallels_cbeta'",
        [],
        |row| row.get::<_, i64>(0),
    )
    .unwrap()
        > 0
}

#[test]
fn cite_builds_the_missing_index_and_output_is_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    write_cite_fixture(dir.path());
    assert!(
        !index_exists(dir.path()),
        "fixture must start without the index"
    );

    let first = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args(["cite", "T0251", "--offline", "--data-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(first.status.code(), Some(0));
    assert!(index_exists(dir.path()), "cite must have built the index");
    assert!(
        String::from_utf8_lossy(&first.stderr).contains("建立索引"),
        "the first run actually built the index, so it must announce it: {}",
        String::from_utf8_lossy(&first.stderr)
    );

    // Second run: index already present, so no rebuild and no notice. This is
    // also the idempotency check — `ensure_cbeta_index` returns before it even
    // opens the file read-write once the index exists.
    let second = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args(["cite", "T0251", "--offline", "--data-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(first.stdout).unwrap(),
        String::from_utf8(second.stdout).unwrap()
    );
    assert!(
        !String::from_utf8(second.stderr)
            .unwrap()
            .contains("建立索引"),
        "the notice must appear only when actually building"
    );
}

#[test]
fn a_held_lock_skips_the_build_and_leaves_results_identical() {
    // Holding the operation lock forces `cite` down the un-indexed path, which
    // is the only portable way to compare indexed vs un-indexed output — the
    // first run of the previous test already has the index by the time it
    // queries, so it cannot make this comparison.
    let indexed_dir = tempfile::tempdir().unwrap();
    write_cite_fixture(indexed_dir.path());
    let indexed = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args(["cite", "T0251", "--offline", "--data-dir"])
        .arg(indexed_dir.path())
        .output()
        .unwrap();
    assert!(index_exists(indexed_dir.path()));

    let plain_dir = tempfile::tempdir().unwrap();
    write_cite_fixture(plain_dir.path());
    let lock_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(plain_dir.path().join("data.sqlite.lock"))
        .unwrap();
    lock_file.try_lock().expect("test must own the lock");

    let plain = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args(["cite", "T0251", "--offline", "--data-dir"])
        .arg(plain_dir.path())
        .output()
        .unwrap();
    drop(lock_file);

    assert_eq!(
        plain.status.code(),
        Some(0),
        "a busy lock must not fail the query"
    );
    assert!(
        !index_exists(plain_dir.path()),
        "a busy lock must skip the build rather than wait for it"
    );
    assert_eq!(
        String::from_utf8(indexed.stdout).unwrap(),
        String::from_utf8(plain.stdout).unwrap(),
        "the index changes speed only — never results"
    );
}

#[test]
fn parallel_does_not_build_the_cite_index() {
    let dir = tempfile::tempdir().unwrap();
    write_cite_fixture(dir.path());

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args(["parallel", "色即是空", "--offline", "--data-dir"])
        .arg(dir.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    assert!(
        !index_exists(dir.path()),
        "only the cite path needs this index; parallel must not pay for it"
    );
}

// Verifies the earliest fallback path: data directory not writable, no
// pre-existing lock file. `operation_lock::try_acquire` fails at lock-file
// creation (which needs directory write access) and returns early, before
// reaching the read-write open, the announcement, or the index build.
//
// Because that path exits so early, the stderr and index assertions below are
// trivially true by construction: this test CANNOT distinguish graceful
// degradation from the wiring being absent altogether. The tests that do pin
// the wiring down are `cite_builds_the_missing_index_and_output_is_unchanged`
// and `a_held_lock_skips_the_build_and_leaves_results_identical`.
#[cfg(unix)]
#[test]
fn cite_degrades_gracefully_when_the_data_dir_is_read_only() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    write_cite_fixture(dir.path());
    let original = std::fs::metadata(dir.path()).unwrap().permissions();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args(["cite", "T0251", "--offline", "--data-dir"])
        .arg(dir.path())
        .output()
        .unwrap();

    // Restore before asserting so a failure still leaves a removable tempdir.
    std::fs::set_permissions(dir.path(), original).unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "a read-only data dir must not fail the query: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8(output.stdout)
        .unwrap()
        .contains("色即是空"));
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("建立索引"),
        "a fresh read-only directory must never build the index, so it must not announce one: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !index_exists(dir.path()),
        "a read-only directory must not have the index"
    );
}

#[cfg(unix)]
#[test]
fn cite_stays_silent_when_a_preexisting_lock_file_meets_a_read_only_directory() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    write_cite_fixture(dir.path());

    // A lock file left behind by any earlier `cite` or `data update` run
    // already exists here, unlocked. `try_acquire` succeeds on it, so the
    // code reaches the read-write open, which also succeeds — the file
    // itself is writable. Only the later `CREATE INDEX` fails, once the
    // directory itself blocks the new file SQLite needs for its rollback
    // journal. A pre-success notice would fire every single time despite the
    // index never actually being built.
    std::fs::File::create(dir.path().join("data.sqlite.lock")).unwrap();

    let original = std::fs::metadata(dir.path()).unwrap().permissions();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args(["cite", "T0251", "--offline", "--data-dir"])
        .arg(dir.path())
        .output()
        .unwrap();

    // Restore before asserting so a failure still leaves a removable tempdir.
    std::fs::set_permissions(dir.path(), original).unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "a read-only data dir must not fail the query: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8(output.stdout)
        .unwrap()
        .contains("色即是空"));
    assert!(
        !String::from_utf8(output.stderr)
            .unwrap()
            .contains("建立索引"),
        "must not announce an index build that never actually completed"
    );
    assert!(
        !index_exists(dir.path()),
        "the read-only directory must have blocked index creation"
    );
}

/// SQLite 回滚日志的文件头 magic。日志一旦带上它就是"热"的:任何新进程
/// 打开这个数据库时都必须先把它回放掉,而回放需要写权限。
const JOURNAL_MAGIC: [u8; 8] = [0xd9, 0xd5, 0x05, 0xf9, 0x20, 0xa1, 0x63, 0xd7];

/// 在 `dir` 里布置"写事务进行到一半、进程猝死"留下的现场:一份已经被
/// 部分改写的 `data.sqlite`,外加一份仍然有效的 `data.sqlite-journal`。
///
/// 做法不是杀进程(时序不可重复),而是把一个**尚未提交**的事务在磁盘上
/// 的状态原样复制出来。SQLite 的不变量是"任何被写回主库文件的页,其原始
/// 内容必定已经先同步进日志",所以复制出来的这一对,恰好就是进程死在该
/// 窗口内时留在磁盘上的东西。文件锁不会跟着字节被复制走,因此复制品里
/// 的日志对任何新进程都是"热"的。
///
/// 触发点是页缓存溢出:缓存放不下时 SQLite 必须先把日志(含 magic 头)
/// 同步落盘,才能把脏页写回主库 —— 这正是 588 MB 数据上 `CREATE INDEX`
/// 会走到的那一步。
fn plant_hot_journal(dir: &std::path::Path) {
    let scratch = tempfile::tempdir().unwrap();
    write_cite_fixture(scratch.path());
    let source = scratch.path().join("data.sqlite");
    let source_journal = scratch.path().join("data.sqlite-journal");

    let conn = Connection::open(&source).unwrap();
    conn.pragma_update(None, "cache_size", 10_i64).unwrap();
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    for index in 0..4000 {
        conn.execute(
            "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
             VALUES (?1,?1,'sa','x',0.9,'T9999','未提交',1)",
            rusqlite::params![format!("未提交的垃圾行{index}")],
        )
        .unwrap();
    }

    // 既不提交也不回滚:此刻磁盘上就是崩溃现场。先复制主库再复制日志,
    // 保证日志不会比主库更旧。用 read/write 而不是 `fs::copy`,因为前者
    // 的共享模式在各平台上都允许读取另一个连接正打开着的文件。
    let data = dir.join("data.sqlite");
    let journal = dir.join("data.sqlite-journal");
    std::fs::write(&data, std::fs::read(&source).unwrap()).unwrap();
    std::fs::write(&journal, std::fs::read(&source_journal).unwrap()).unwrap();
    drop(conn); // scratch 自行回滚,与 dir 里的复制品无关

    let planted_journal = std::fs::read(&journal).unwrap();
    assert!(
        planted_journal.starts_with(&JOURNAL_MAGIC),
        "现场必须留下一份带 magic 头的热日志,否则这个测试什么都没复现"
    );
    let planted_data = std::fs::read(&data).unwrap();
    assert!(
        planted_data
            .windows("未提交的垃圾行".len())
            .any(|window| window == "未提交的垃圾行".as_bytes()),
        "主库文件里必须真的落下了未提交的页,否则回滚有没有发生无从区分"
    );
}

fn parallel_rows(dir: &std::path::Path) -> (i64, i64) {
    let conn = Connection::open(dir.join("data.sqlite")).unwrap();
    let total = conn
        .query_row("SELECT COUNT(*) FROM parallels", [], |row| row.get(0))
        .unwrap();
    let uncommitted = conn
        .query_row(
            "SELECT COUNT(*) FROM parallels WHERE cbeta_id = 'T9999'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    (total, uncommitted)
}

// 回归 CRITICAL:懒建索引被中断后留下的热日志,曾经让所有查询命令永久
// 退出 1 并谎称数据不兼容。只读连接回滚不了日志,但数据目录本身是可写
// 的 —— 查询路径必须自己把它回滚掉,而不是把用户推去重下 183 MB。
#[test]
fn cite_rolls_back_a_hot_journal_left_by_an_interrupted_index_build() {
    let dir = tempfile::tempdir().unwrap();
    plant_hot_journal(dir.path());

    let output = run_fojin(&["cite", "T0251", "--offline"], dir.path());

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "a rollback-able journal must not fail the query: {stderr}"
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("色即是空"),
        "the recovered dataset must answer normally"
    );
    assert!(
        !stderr.contains("dataset incompatibility"),
        "an interrupted write is not a dataset incompatibility: {stderr}"
    );
    assert!(
        !dir.path().join("data.sqlite-journal").exists(),
        "SQLite must have rolled the journal back and removed it"
    );
    assert_eq!(
        parallel_rows(dir.path()),
        (2, 0),
        "rollback must restore the pre-transaction rows, not commit the junk"
    );
}

// 同一份现场对每条读路径都成立:`parallel` 与 `data verify` 曾经一起
// 退出 1,而 `data verify` 正是用户会用来诊断的那条命令。
#[test]
fn every_read_command_recovers_from_a_hot_journal() {
    for args in [
        vec!["parallel", "色即是空", "--offline"],
        vec!["data", "verify"],
    ] {
        let dir = tempfile::tempdir().unwrap();
        plant_hot_journal(dir.path());

        let output = run_fojin(&args, dir.path());

        let stderr = String::from_utf8(output.stderr).unwrap();
        assert_eq!(
            output.status.code(),
            Some(0),
            "`{args:?}` must recover instead of failing: {stderr}"
        );
        assert!(
            !stderr.contains("dataset incompatibility"),
            "`{args:?}` must not blame the dataset: {stderr}"
        );
        assert_eq!(parallel_rows(dir.path()), (2, 0), "`{args:?}`");
    }
}

// 自愈的边界:数据真的不可写时,回滚做不到,查询也**不能**假装成功。
// 区分两者的唯一判据是以读写方式重开能不能真的读到数据 —— 这里内核会
// 拒绝写入(SQLite 静默降级为只读打开),于是自愈失败,命令仍然报错。
#[cfg(unix)]
#[test]
fn a_read_only_dataset_still_fails_instead_of_pretending_to_recover() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    plant_hot_journal(dir.path());
    let data = dir.path().join("data.sqlite");
    let journal = dir.path().join("data.sqlite-journal");
    let data_before = std::fs::read(&data).unwrap();
    let journal_before = std::fs::read(&journal).unwrap();
    for path in [&data, &journal] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o444)).unwrap();
    }
    let directory_permissions = std::fs::metadata(dir.path()).unwrap().permissions();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();

    let output = run_fojin(&["cite", "T0251", "--offline"], dir.path());

    // 先恢复权限,失败时 tempdir 才删得掉。
    std::fs::set_permissions(dir.path(), directory_permissions).unwrap();
    for path in [&data, &journal] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
    }

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "an unrecoverable dataset must not report success: {stderr}"
    );
    assert!(
        stderr.contains("日志") && stderr.contains("写权限"),
        "the message must name the real cause and what to do: {stderr}"
    );
    assert!(
        !stderr.contains("dataset incompatibility") && !stderr.contains("data update"),
        "neither the diagnosis nor the advice was ever true here: {stderr}"
    );
    assert_eq!(
        std::fs::read(&data).unwrap(),
        data_before,
        "a failed self-heal must leave the dataset byte-identical"
    );
    assert_eq!(
        std::fs::read(&journal).unwrap(),
        journal_before,
        "a failed self-heal must never delete the journal it could not replay"
    );
}
