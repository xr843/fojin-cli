use fojin_cli::model::{MatchGroup, Parallel};
use fojin_cli::render::{render_human, render_json};

fn heart() -> MatchGroup {
    MatchGroup {
        zh_text: "色即是空".into(),
        cbeta_id: Some("T0251".into()),
        title_zh: Some("心經".into()),
        juan_num: Some(1),
        parallels: vec![
            Parallel {
                lang: "sa".into(),
                text: "rūpaṃ śūnyatā".into(),
                confidence: Some(0.91),
            },
            Parallel {
                lang: "bo".into(),
                text: "gzugs stong pa".into(),
                confidence: Some(0.88),
            },
        ],
    }
}

#[test]
fn human_shows_parallels_wuduiqi_and_footer() {
    let out = render_human(&[heart()], None, 0);
    assert!(out.contains("汉  色即是空  (《心經》T0251 卷1)"));
    assert!(out.contains("梵  rūpaṃ śūnyatā  [MITRA 0.91]"));
    assert!(out.contains("藏  gzugs stong pa  [MITRA 0.88]"));
    assert!(out.contains("巴  (无对齐)"));
    assert!(out.contains("完整上下文见 https://fojin.app"));
}

#[test]
fn human_empty_is_honest() {
    assert_eq!(render_human(&[], None, 0).trim(), "未找到对齐");
}

#[test]
fn json_flags_matched() {
    assert!(render_json(&[], 0).contains("\"matched\": false"));
    assert!(render_json(&[heart()], 1).contains("\"matched\": true"));
    assert!(render_json(&[heart()], 1).contains("rūpaṃ śūnyatā"));
}

#[test]
fn human_shows_extra_lang_and_full_footer() {
    let mut g = heart();
    g.parallels.push(Parallel {
        lang: "en".into(),
        text: "form is emptiness".into(),
        confidence: Some(0.75),
    });
    let out = render_human(&[g], None, 0);
    assert!(
        out.contains("英  form is emptiness  [MITRA 0.75]"),
        "extra lang en prints when present"
    );
    // full footer including the CC BY-SA attribution half (a regression to that half must fail)
    assert!(out.contains("完整上下文见 https://fojin.app  ·  数据 CC BY-SA(Dharmamitra + fojin)"));
}

#[test]
fn human_multi_group_footer_once() {
    let out = render_human(&[heart(), heart()], None, 0);
    assert_eq!(
        out.matches("完整上下文见 https://fojin.app").count(),
        1,
        "footer must appear exactly once across multiple groups"
    );
}

#[test]
fn json_exposes_only_public_fields() {
    let out = render_json(&[heart()], 1);
    // must never surface normalized/internal columns
    assert!(!out.contains("zh_norm"), "internal zh_norm must not leak");
    assert!(!out.contains("\"method\""), "internal method must not leak");
    // sanity: expected public keys present
    assert!(
        out.contains("\"zh_text\"")
            && out.contains("\"parallels\"")
            && out.contains("\"confidence\"")
    );
}

#[test]
fn human_lang_filter_hides_unrequested_and_no_false_wuduiqi() {
    let langs = vec!["sa".to_string()];
    let out = render_human(&[heart()], Some(&langs), 0);
    assert!(
        out.contains("梵  rūpaṃ śūnyatā"),
        "requested language shown"
    );
    assert!(
        !out.contains("(无对齐)"),
        "must NOT print false (无对齐) for a filtered-out language"
    );
    assert!(
        !out.contains("藏"),
        "filtered-out language must not appear at all"
    );
}

#[test]
fn human_shows_hidden_count_hint_when_truncated() {
    let out = render_human(&[heart()], None, 5);
    assert!(out.contains("还有 5 组"), "must show hidden-count hint");
    assert!(
        out.contains("--all"),
        "hint must mention --all escape hatch"
    );
}

#[test]
fn human_no_hidden_hint_when_not_truncated() {
    let out = render_human(&[heart()], None, 0);
    assert!(
        !out.contains("还有"),
        "must not show hidden-count hint when nothing is hidden"
    );
}

#[test]
fn json_includes_total_and_shown() {
    let out = render_json(&[heart()], 40);
    assert!(out.contains("\"total\": 40"));
    assert!(out.contains("\"shown\": 1"));
}
