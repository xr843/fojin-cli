use crate::model::MatchGroup;

pub const FOOTER: &str = "完整上下文见 https://fojin.app  ·  数据 CC BY-SA(Dharmamitra + fojin)";

const DISPLAY_LANGS: [&str; 3] = ["sa", "bo", "pi"];

pub fn lang_label(code: &str) -> &str {
    match code {
        "sa" => "梵",
        "pi" => "巴",
        "bo" => "藏",
        "en" => "英",
        "lzh" | "zh" => "汉",
        other => other,
    }
}

fn conf_tag(c: Option<f64>) -> String {
    c.map(|v| format!("  [MITRA {v:.2}]")).unwrap_or_default()
}

pub fn render_human(groups: &[MatchGroup]) -> String {
    if groups.is_empty() {
        return "未找到对齐\n".to_string();
    }
    let mut out = String::new();
    for (gi, g) in groups.iter().enumerate() {
        if gi > 0 {
            out.push('\n');
        }
        let src = match (&g.title_zh, &g.cbeta_id, g.juan_num) {
            (Some(t), Some(c), Some(j)) => format!("  (《{t}》{c} 卷{j})"),
            (Some(t), Some(c), None) => format!("  (《{t}》{c})"),
            _ => String::new(),
        };
        out.push_str(&format!("汉  {}{}\n", g.zh_text, src));

        for code in DISPLAY_LANGS {
            let items: Vec<_> = g.parallels.iter().filter(|p| p.lang == code).collect();
            if items.is_empty() {
                out.push_str(&format!("{}  (无对齐)\n", lang_label(code)));
            } else {
                for p in items {
                    out.push_str(&format!(
                        "{}  {}{}\n",
                        lang_label(code),
                        p.text,
                        conf_tag(p.confidence)
                    ));
                }
            }
        }
        for p in &g.parallels {
            if !DISPLAY_LANGS.contains(&p.lang.as_str()) {
                out.push_str(&format!(
                    "{}  {}{}\n",
                    lang_label(&p.lang),
                    p.text,
                    conf_tag(p.confidence)
                ));
            }
        }
    }
    out.push_str(&format!("\n{FOOTER}\n"));
    out
}

pub fn render_json(groups: &[MatchGroup]) -> String {
    let v = serde_json::json!({
        "matched": !groups.is_empty(),
        "groups": groups,
    });
    serde_json::to_string_pretty(&v).unwrap()
}
