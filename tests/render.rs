use fojin_cli::model::{MatchGroup, Parallel};
use fojin_cli::render::{render_human, render_json, render_outcome_human, render_outcome_json};
use fojin_cli::search::{FallbackInfo, SearchOutcome, SegmentResult};

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
    assert!(
        !out.contains("巴"),
        "dataset has no Pali; default view must not show a permanent placeholder: {out}"
    );
    assert!(out.contains("完整上下文见 https://fojin.app"));
}

#[test]
fn explicit_lang_pi_still_shows_placeholder() {
    let langs = vec!["pi".to_string()];
    let out = render_human(&[heart()], Some(&langs), 0);
    assert!(
        out.contains("巴  (无对齐)"),
        "explicitly requested language must answer honestly: {out}"
    );
}

#[test]
fn future_pali_data_displays_without_binary_change() {
    let mut g = heart();
    g.parallels.push(fojin_cli::model::Parallel {
        lang: "pi".to_string(),
        text: "rūpaṁ suññatā".to_string(),
        confidence: Some(0.9),
    });
    let out = render_human(&[g], None, 0);
    assert!(
        out.contains("巴  rūpaṁ suññatā"),
        "real Pali parallels must surface via the extra-lang path: {out}"
    );
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
fn json_empty_shown_still_reports_matches_when_total_is_positive() {
    let out = render_json(&[], 3);
    assert!(out.contains("\"matched\": true"));
    assert!(out.contains("\"shown\": 0"));
    assert!(out.contains("\"total\": 3"));
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
    // Full footer, asserted whole: each half is a separate CC BY-SA 4.0
    // condition (creator + license name + change indication; license URI),
    // so dropping either half is a license regression, not a cosmetic one.
    assert!(out.contains(
        "对齐数据:Dharmamitra MITRA-parallel · CC BY-SA 4.0 · 经 fojin 归一化并打包\n\
         https://creativecommons.org/licenses/by-sa/4.0/ · 完整上下文见 https://fojin.app"
    ));
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

fn outcome_with_segments() -> SearchOutcome {
    SearchOutcome {
        groups: vec![heart()],
        total: 1,
        segments: Some(vec![
            SegmentResult {
                text: "色即是空".into(),
                matched: true,
                total: 1,
                groups: vec![heart()],
                fallback: None,
            },
            SegmentResult {
                text: "度一切苦厄".into(),
                matched: false,
                total: 0,
                groups: vec![],
                fallback: Some(FallbackInfo {
                    matched_substring: "一切苦".into(),
                    char_len: 3,
                }),
            },
        ]),
        fallback: None,
        truncated_segments: 0,
    }
}

#[test]
fn human_split_output_labels_each_segment() {
    let out = render_outcome_human(&outcome_with_segments(), None);
    assert!(out.contains("已按句切分查询"), "got: {out}");
    assert!(out.contains("--no-split"));
    assert!(out.contains("【色即是空】"));
    assert!(out.contains("【度一切苦厄】"));
    assert!(out.contains("梵  rūpaṃ śūnyatā  [MITRA 0.91]"));
}

#[test]
fn human_segment_fallback_points_at_the_matching_substring() {
    let out = render_outcome_human(&outcome_with_segments(), None);
    assert!(out.contains("其中「一切苦」"), "got: {out}");
    assert!(out.contains("可单独查询"));
}

#[test]
fn human_reports_truncated_segments_instead_of_dropping_silently() {
    let mut outcome = outcome_with_segments();
    outcome.truncated_segments = 4;
    let out = render_outcome_human(&outcome, None);
    assert!(out.contains("还有 4 句未处理"), "got: {out}");
}

#[test]
fn human_whole_string_fallback_is_a_single_hint() {
    let outcome = SearchOutcome {
        groups: vec![],
        total: 0,
        segments: None,
        fallback: Some(FallbackInfo {
            matched_substring: "色不异空".into(),
            char_len: 4,
        }),
        truncated_segments: 0,
    };
    let out = render_outcome_human(&outcome, None);
    assert!(out.contains("未找到对齐"));
    assert!(out.contains("其中「色不异空」(4 字) 有对齐"), "got: {out}");
}

#[test]
fn json_keeps_the_existing_top_level_contract() {
    let out = render_outcome_json(&outcome_with_segments());
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["matched"], serde_json::json!(true));
    assert_eq!(v["total"], serde_json::json!(1));
    assert_eq!(v["shown"], serde_json::json!(1));
    assert!(v["groups"].is_array());
    assert_eq!(v["schema_version"], serde_json::json!(1));
}

#[test]
fn json_segments_appear_only_when_splitting_happened() {
    let with = render_outcome_json(&outcome_with_segments());
    let v: serde_json::Value = serde_json::from_str(&with).unwrap();
    assert!(v["segments"].is_array());
    assert_eq!(
        v["segments"][1]["fallback"]["matched_substring"],
        serde_json::json!("一切苦")
    );
    assert_eq!(v["segments"][1]["total"], serde_json::json!(0));

    let plain = SearchOutcome {
        groups: vec![heart()],
        total: 1,
        segments: None,
        fallback: None,
        truncated_segments: 0,
    };
    let v: serde_json::Value = serde_json::from_str(&render_outcome_json(&plain)).unwrap();
    assert!(
        v.get("segments").is_none(),
        "no segments key on the hit path"
    );
    assert!(v.get("fallback").is_none());
}

#[test]
fn perfect_confidence_shows_no_tag() {
    let group = MatchGroup {
        zh_text: "色即是空".into(),
        cbeta_id: Some("T0251".into()),
        title_zh: Some("心經".into()),
        juan_num: Some(1),
        parallels: vec![Parallel {
            lang: "sa".into(),
            text: "rūpaṃ śūnyatā".into(),
            confidence: Some(1.0),
        }],
    };
    let out = render_human(&[group], None, 0);
    assert!(
        out.contains("梵  rūpaṃ śūnyatā"),
        "the parallel itself stays: {out}"
    );
    assert!(
        // "[MITRA", not "MITRA": the footer names the source dataset
        // (MITRA-parallel), so only the bracket distinguishes the conf tag.
        !out.contains("[MITRA"),
        "a uniform 1.00 carries no information and must not be printed: {out}"
    );
}

#[test]
fn imperfect_confidence_still_shows_the_tag() {
    let group = MatchGroup {
        zh_text: "色即是空".into(),
        cbeta_id: Some("T0251".into()),
        title_zh: Some("心經".into()),
        juan_num: Some(1),
        parallels: vec![Parallel {
            lang: "sa".into(),
            text: "rūpaṃ śūnyatā".into(),
            confidence: Some(0.87),
        }],
    };
    let out = render_human(&[group], None, 0);
    assert!(
        out.contains("梵  rūpaṃ śūnyatā  [MITRA 0.87]"),
        "got: {out}"
    );
}

#[test]
fn confidence_rounding_to_one_hides_the_tag() {
    // 0.9951 formats as "1.00"; printing `[MITRA 1.00]` while calling the value
    // informative would contradict itself, so the rule keys on what would be
    // displayed, not on the raw float.
    let group = MatchGroup {
        zh_text: "色即是空".into(),
        cbeta_id: Some("T0251".into()),
        title_zh: Some("心經".into()),
        juan_num: Some(1),
        parallels: vec![Parallel {
            lang: "sa".into(),
            text: "rūpaṃ śūnyatā".into(),
            confidence: Some(0.9951),
        }],
    };
    let out = render_human(&[group], None, 0);
    assert!(
        out.contains("梵  rūpaṃ śūnyatā"),
        "the parallel itself stays: {out}"
    );
    // "[MITRA", not "MITRA": the footer names the source dataset
    // (MITRA-parallel), so only the bracket distinguishes the conf tag.
    assert!(!out.contains("[MITRA"), "got: {out}");
}

#[test]
fn absent_confidence_shows_no_tag() {
    let group = MatchGroup {
        zh_text: "色即是空".into(),
        cbeta_id: Some("T0251".into()),
        title_zh: Some("心經".into()),
        juan_num: Some(1),
        parallels: vec![Parallel {
            lang: "sa".into(),
            text: "rūpaṃ śūnyatā".into(),
            confidence: None,
        }],
    };
    let out = render_human(&[group], None, 0);
    assert!(
        out.contains("梵  rūpaṃ śūnyatā"),
        "the parallel itself stays: {out}"
    );
    // "[MITRA", not "MITRA": the footer names the source dataset
    // (MITRA-parallel), so only the bracket distinguishes the conf tag.
    assert!(!out.contains("[MITRA"), "got: {out}");
}

#[test]
fn json_still_carries_confidence_when_the_tag_is_hidden() {
    let group = MatchGroup {
        zh_text: "色即是空".into(),
        cbeta_id: Some("T0251".into()),
        title_zh: Some("心經".into()),
        juan_num: Some(1),
        parallels: vec![Parallel {
            lang: "sa".into(),
            text: "rūpaṃ śūnyatā".into(),
            confidence: Some(1.0),
        }],
    };
    let out = render_json(&[group], 1);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        v["groups"][0]["parallels"][0]["confidence"],
        serde_json::json!(1.0),
        "the JSON contract is unchanged; only the human tag is suppressed"
    );
}
