use fojin_cli::schema::init_schema;
use rusqlite::Connection;

fn write_offline_db(dir: &std::path::Path) {
    let db_path = dir.join("data.sqlite");
    let conn = Connection::open(db_path).unwrap();
    init_schema(&conn).unwrap();
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
