#[test]
fn version_is_nonempty() {
    assert!(!fojin_cli::version().is_empty());
}
