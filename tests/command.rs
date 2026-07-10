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
