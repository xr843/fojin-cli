use fojin_cli::lang::{validate_langs, KNOWN_LANGS};
use fojin_cli::render::lang_label;

#[test]
fn accepts_known_codes() {
    let codes = vec!["sa".to_string(), "bo".to_string()];
    assert!(validate_langs(&codes).is_ok());
}

#[test]
fn rejects_unknown_code_and_lists_alternatives() {
    let codes = vec!["sk".to_string()];
    let err = validate_langs(&codes).unwrap_err().to_string();
    assert!(err.contains("未知语种 `sk`"), "got: {err}");
    assert!(err.contains("sa"), "error must list usable codes: {err}");
}

#[test]
fn every_known_lang_has_a_real_label() {
    // Guards KNOWN_LANGS against render::lang_label drifting apart: a code with
    // no label falls through to the `other => other` arm and returns itself.
    for code in KNOWN_LANGS {
        assert_ne!(
            lang_label(code),
            code,
            "`{code}` is in KNOWN_LANGS but has no label in render::lang_label"
        );
    }
}
