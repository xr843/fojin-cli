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
    let out = run_fojin(&["data", "clean"], dir.path());
    assert_eq!(out.status.code(), Some(0));
    assert!(!dir.path().join("data.sqlite").exists());
    let out2 = run_fojin(&["data", "clean"], dir.path());
    assert_eq!(out2.status.code(), Some(0));
    assert!(String::from_utf8(out2.stdout).unwrap().contains("无数据"));
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
