use fojin_cli::model::{MatchGroup, Parallel};
use fojin_cli::render::{render_human, render_json};

fn heart() -> MatchGroup {
    MatchGroup {
        zh_text: "色即是空".into(),
        cbeta_id: Some("T0251".into()),
        title_zh: Some("心經".into()),
        juan_num: Some(1),
        parallels: vec![
            Parallel { lang: "sa".into(), text: "rūpaṃ śūnyatā".into(), confidence: Some(0.91) },
            Parallel { lang: "bo".into(), text: "gzugs stong pa".into(), confidence: Some(0.88) },
        ],
    }
}

#[test]
fn human_shows_parallels_wuduiqi_and_footer() {
    let out = render_human(&[heart()]);
    assert!(out.contains("汉  色即是空  (《心經》T0251 卷1)"));
    assert!(out.contains("梵  rūpaṃ śūnyatā  [MITRA 0.91]"));
    assert!(out.contains("藏  gzugs stong pa  [MITRA 0.88]"));
    assert!(out.contains("巴  (无对齐)"));
    assert!(out.contains("完整上下文见 https://fojin.app"));
}

#[test]
fn human_empty_is_honest() {
    assert_eq!(render_human(&[]).trim(), "未找到对齐");
}

#[test]
fn json_flags_matched() {
    assert!(render_json(&[]).contains("\"matched\": false"));
    assert!(render_json(&[heart()]).contains("\"matched\": true"));
    assert!(render_json(&[heart()]).contains("rūpaṃ śūnyatā"));
}
