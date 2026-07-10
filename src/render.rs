use crate::model::MatchGroup;

pub const FOOTER: &str = "完整上下文见 https://fojin.app  ·  数据 CC BY-SA(Dharmamitra + fojin)";

/// Languages shown by default. Pali is deliberately absent: the current
/// dataset (data-v1) has zero pi rows, so a permanent "(无对齐)" placeholder
/// carries no information. Real pi parallels in a future dataset still
/// surface via the extra-lang path in render_human; explicit --lang pi
/// still answers with the placeholder.
const DISPLAY_LANGS: [&str; 2] = ["sa", "bo"];

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

pub fn render_human(groups: &[MatchGroup], langs: Option<&[String]>, hidden: usize) -> String {
    if groups.is_empty() {
        return "未找到对齐\n".to_string();
    }
    let display: Vec<String> = match langs {
        Some(filter) if !filter.is_empty() => filter.to_vec(),
        _ => DISPLAY_LANGS.iter().map(|s| s.to_string()).collect(),
    };
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

        for code in &display {
            let items: Vec<_> = g.parallels.iter().filter(|p| &p.lang == code).collect();
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
        if langs.is_none() {
            for p in &g.parallels {
                if !display.iter().any(|d| d == &p.lang) {
                    out.push_str(&format!(
                        "{}  {}{}\n",
                        lang_label(&p.lang),
                        p.text,
                        conf_tag(p.confidence)
                    ));
                }
            }
        }
    }
    if hidden > 0 {
        out.push_str(&format!("\n… 还有 {hidden} 组匹配,加 --all 查看全部\n"));
    }
    out.push_str(&format!("\n{FOOTER}\n"));
    out
}

fn group_digits(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

pub fn render_texts(entries: &[crate::query::TextEntry]) -> String {
    if entries.is_empty() {
        return "未找到匹配的经名\n".to_string();
    }
    let mut out = String::new();
    for e in entries {
        let counts: Vec<String> = e
            .by_lang
            .iter()
            .map(|(l, c)| format!("{} {}", lang_label(l), group_digits(*c)))
            .collect();
        out.push_str(&format!(
            "{}  {}  ({})\n",
            e.cbeta_id,
            e.title_zh,
            counts.join(" · ")
        ));
    }
    out.push_str(&format!(
        "\n共 {} 部;用 fojin cite <编号> 查看对齐\n",
        entries.len()
    ));
    out
}

pub fn render_status(
    path: &str,
    size_bytes: Option<u64>,
    stats: Option<&crate::data::DatasetStats>,
) -> String {
    const MB: u64 = 1024 * 1024;
    let mut out = String::new();
    out.push_str(&format!("数据位置  {path}\n"));
    match (size_bytes, stats) {
        (Some(size), Some(s)) => {
            out.push_str(&format!("状态      已下载 ({} MB)\n", size / MB));
            let about: Vec<String> = [
                s.version.as_deref(),
                s.license.as_deref(),
                s.attribution.as_deref(),
            ]
            .iter()
            .flatten()
            .map(|v| v.to_string())
            .collect();
            if !about.is_empty() {
                out.push_str(&format!("数据版本  {}\n", about.join(" · ")));
            }
            out.push_str(&format!("对齐总数  {}\n", group_digits(s.total)));
            for (lang, count) in &s.by_lang {
                out.push_str(&format!(
                    "  {} {}   {}\n",
                    lang_label(lang),
                    lang,
                    group_digits(*count)
                ));
            }
            out.push_str(&format!(
                "收录文本  {} 部 (Taishō)\n",
                group_digits(s.texts)
            ));
        }
        _ => {
            out.push_str("状态      未下载\n");
            out.push_str("提示      运行 fojin parallel \"...\" 或 fojin data update 下载数据\n");
        }
    }
    out
}

pub fn render_json(groups: &[MatchGroup], total: usize) -> String {
    let v = serde_json::json!({
        "matched": total > 0,
        "total": total,
        "shown": groups.len(),
        "groups": groups,
    });
    serde_json::to_string_pretty(&v).unwrap()
}
