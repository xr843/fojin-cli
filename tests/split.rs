use fojin_cli::search::split::{split_sentences, MAX_SEGMENTS};

/// Stand-in for the real predicate (normalized length >= 2).
fn keep_two_chars(s: &str) -> bool {
    s.chars().count() >= 2
}

#[test]
fn splits_on_sentence_punctuation() {
    let out = split_sentences(
        "觀自在菩薩，行深般若波羅蜜多時。照見五蘊皆空",
        keep_two_chars,
    );
    assert_eq!(
        out.segments,
        vec!["觀自在菩薩", "行深般若波羅蜜多時", "照見五蘊皆空"]
    );
    assert_eq!(out.truncated, 0);
}

#[test]
fn consecutive_punctuation_yields_no_empty_segments() {
    let out = split_sentences("色即是空。。。受想行識", keep_two_chars);
    assert_eq!(out.segments, vec!["色即是空", "受想行識"]);
}

#[test]
fn punctuation_only_input_yields_nothing() {
    let out = split_sentences("，。！？", keep_two_chars);
    assert!(out.segments.is_empty());
}

#[test]
fn drops_segments_the_predicate_rejects() {
    let out = split_sentences("色即是空，也。受想行識", keep_two_chars);
    assert_eq!(
        out.segments,
        vec!["色即是空", "受想行識"],
        "the single-character 也 must be dropped"
    );
}

#[test]
fn does_not_split_on_in_sentence_marks() {
    // Book-title brackets, quotes, parens and the interpunct appear inside a
    // sentence; splitting there would cut phrases in half.
    let out = split_sentences("如《心經》所說「色即是空」（略）·如是", keep_two_chars);
    assert_eq!(out.segments.len(), 1, "got: {:?}", out.segments);
}

#[test]
fn splits_on_newlines_and_ascii_punctuation() {
    let out = split_sentences("色即是空\n受想行識,無眼耳鼻舌身意", keep_two_chars);
    assert_eq!(out.segments, vec!["色即是空", "受想行識", "無眼耳鼻舌身意"]);
}

#[test]
fn trims_surrounding_whitespace() {
    let out = split_sentences("  色即是空 ，  受想行識  ", keep_two_chars);
    assert_eq!(out.segments, vec!["色即是空", "受想行識"]);
}

#[test]
fn caps_segments_and_reports_the_overflow() {
    let raw = (0..MAX_SEGMENTS + 3)
        .map(|i| format!("第{i}句子"))
        .collect::<Vec<_>>()
        .join("。");
    let out = split_sentences(&raw, keep_two_chars);
    assert_eq!(out.segments.len(), MAX_SEGMENTS);
    assert_eq!(out.truncated, 3, "the overflow count must be reported");
}
