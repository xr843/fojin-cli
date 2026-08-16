use crate::model::MatchGroup;
use crate::search::{SearchOutcome, SCHEMA_VERSION};

/// Printed once per query, and the only place a CLI user is guaranteed to see
/// the notice, so it carries every CC BY-SA 4.0 attribution condition on its
/// own: names the creator and the source dataset, names the license and links
/// its text, and states that the material was changed.
///
/// "经 fojin 归一化并打包" is the change indication, and it is also the whole
/// of fojin's claim on this data — every alignment here is Dharmamitra's.
/// fojin contributed the Taishō/title linkage, the simplified-Chinese
/// normalization column, and the SQLite/FTS packaging, nothing else. The
/// earlier "Dharmamitra + fojin" read as joint authorship of the alignments.
pub const FOOTER: &str = "对齐数据:Dharmamitra MITRA-parallel · CC BY-SA 4.0 · 经 fojin 归一化并打包\nhttps://creativecommons.org/licenses/by-sa/4.0/ · 完整上下文见 https://fojin.app";

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

/// The tag appears only when it would show a number other than 1.00.
///
/// Keying on the formatted string rather than the raw float keeps the rule
/// self-consistent: a value like 0.9951 would render as "1.00", so treating it
/// as informative would print a tag that says exactly what we suppress
/// elsewhere. Absence of the tag reads as "no caveat".
fn conf_tag(c: Option<f64>) -> String {
    match c {
        Some(v) => {
            let shown = format!("{v:.2}");
            if shown == "1.00" {
                String::new()
            } else {
                format!("  [MITRA {shown}]")
            }
        }
        None => String::new(),
    }
}

/// Renders groups only — no footer, no "还有 N 组" line. Shared by the plain
/// and the split renderers.
fn render_groups(groups: &[MatchGroup], langs: Option<&[String]>) -> String {
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
    out
}

pub fn render_human(groups: &[MatchGroup], langs: Option<&[String]>, hidden: usize) -> String {
    if groups.is_empty() {
        return "未找到对齐\n".to_string();
    }
    let mut out = render_groups(groups, langs);
    if hidden > 0 {
        out.push_str(&format!("\n… 还有 {hidden} 组匹配,加 --all 查看全部\n"));
    }
    out.push_str(&format!("\n{FOOTER}\n"));
    out
}

fn render_fallback_hint(fallback: &crate::search::FallbackInfo) -> String {
    format!(
        "其中「{}」({} 字) 有对齐,可单独查询\n",
        fallback.matched_substring, fallback.char_len
    )
}

pub fn render_outcome_human(outcome: &SearchOutcome, langs: Option<&[String]>) -> String {
    let Some(segments) = &outcome.segments else {
        if let Some(fallback) = &outcome.fallback {
            let mut out = String::from("未找到对齐;");
            out.push_str(&render_fallback_hint(fallback));
            out.push_str(&format!("\n{FOOTER}\n"));
            return out;
        }
        let hidden = outcome.total - outcome.groups.len();
        return render_human(&outcome.groups, langs, hidden);
    };

    let mut out = String::from("整串未找到对齐,已按句切分查询(加 --no-split 关闭):\n");
    for segment in segments {
        out.push_str(&format!("\n【{}】", segment.text));
        if segment.matched {
            let hidden = segment.total - segment.groups.len();
            let suffix = if hidden > 0 {
                format!("{} 组(另有 {hidden} 组,加 --all 查看)\n", segment.total)
            } else {
                format!("{} 组\n", segment.total)
            };
            out.push_str(&suffix);
            out.push_str(&render_groups(&segment.groups, langs));
        } else {
            out.push_str("未找到对齐");
            match &segment.fallback {
                Some(fallback) => {
                    out.push(';');
                    out.push_str(&render_fallback_hint(fallback));
                }
                None => out.push('\n'),
            }
        }
    }
    if outcome.truncated_segments > 0 {
        out.push_str(&format!(
            "\n(超出 {} 句上限,还有 {} 句未处理)\n",
            crate::search::split::MAX_SEGMENTS,
            outcome.truncated_segments
        ));
    }
    out.push_str(&format!("\n{FOOTER}\n"));
    out
}

pub fn render_outcome_json(outcome: &SearchOutcome) -> String {
    let mut v = serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "matched": outcome.total > 0,
        "total": outcome.total,
        "shown": outcome.groups.len(),
        "groups": outcome.groups,
    });
    if let Some(segments) = &outcome.segments {
        v["segments"] = serde_json::json!(segments);
    }
    if let Some(fallback) = &outcome.fallback {
        v["fallback"] = serde_json::json!(fallback);
    }
    if outcome.truncated_segments > 0 {
        v["truncated_segments"] = serde_json::json!(outcome.truncated_segments);
    }
    serde_json::to_string_pretty(&v).unwrap()
}

fn group_digits(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
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
