use crate::model::{MatchGroup, Parallel};
use rusqlite::Connection;
use std::cmp::Ordering;
use std::collections::BTreeMap;

struct Row {
    zh_text: String,
    zh_norm: String,
    foreign_lang: String,
    foreign_text: String,
    confidence: Option<f64>,
    cbeta_id: Option<String>,
    title_zh: Option<String>,
    juan_num: Option<i64>,
}

pub fn search(
    conn: &Connection,
    norm_query: &str,
    langs: Option<&[String]>,
    top: usize,
) -> rusqlite::Result<Vec<MatchGroup>> {
    if norm_query.is_empty() {
        return Ok(vec![]);
    }
    let rows = fetch_rows(conn, norm_query)?;
    Ok(group_and_rank(rows, norm_query, langs, top))
}

fn fts_quote(q: &str) -> String {
    format!("\"{}\"", q.replace('"', "\"\""))
}

fn fetch_rows(conn: &Connection, norm_query: &str) -> rusqlite::Result<Vec<Row>> {
    let (sql, param) = if norm_query.chars().count() >= 3 {
        (
            "SELECT p.zh_text,p.zh_norm,p.foreign_lang,p.foreign_text,p.confidence,\
                    p.cbeta_id,p.title_zh,p.juan_num \
             FROM parallels_fts f JOIN parallels p ON p.id=f.rowid \
             WHERE parallels_fts MATCH ?1",
            fts_quote(norm_query),
        )
    } else {
        (
            "SELECT zh_text,zh_norm,foreign_lang,foreign_text,confidence,\
                    cbeta_id,title_zh,juan_num \
             FROM parallels WHERE zh_norm LIKE ?1",
            format!("%{norm_query}%"),
        )
    };
    let mut stmt = conn.prepare(sql)?;
    let iter = stmt.query_map([param], |r| {
        Ok(Row {
            zh_text: r.get(0)?,
            zh_norm: r.get(1)?,
            foreign_lang: r.get(2)?,
            foreign_text: r.get(3)?,
            confidence: r.get(4)?,
            cbeta_id: r.get(5)?,
            title_zh: r.get(6)?,
            juan_num: r.get(7)?,
        })
    })?;
    iter.collect()
}

struct Acc {
    zh_text: String,
    cbeta_id: Option<String>,
    title_zh: Option<String>,
    juan_num: Option<i64>,
    contains: bool,
    max_conf: f64,
    parallels: Vec<Parallel>,
}

fn group_and_rank(
    rows: Vec<Row>,
    norm_query: &str,
    langs: Option<&[String]>,
    top: usize,
) -> Vec<MatchGroup> {
    let mut accs: Vec<Acc> = Vec::new();
    for row in rows {
        if let Some(filter) = langs {
            if !filter.iter().any(|l| l == &row.foreign_lang) {
                continue;
            }
        }
        let contains = row.zh_norm.contains(norm_query);
        let idx = accs.iter().position(|a| {
            a.zh_text == row.zh_text && a.cbeta_id == row.cbeta_id && a.juan_num == row.juan_num
        });
        let idx = match idx {
            Some(i) => i,
            None => {
                accs.push(Acc {
                    zh_text: row.zh_text.clone(),
                    cbeta_id: row.cbeta_id.clone(),
                    title_zh: row.title_zh.clone(),
                    juan_num: row.juan_num,
                    contains: false,
                    max_conf: 0.0,
                    parallels: Vec::new(),
                });
                accs.len() - 1
            }
        };
        let conf = row.confidence.unwrap_or(0.0);
        if conf > accs[idx].max_conf {
            accs[idx].max_conf = conf;
        }
        accs[idx].contains |= contains;
        accs[idx].parallels.push(Parallel {
            lang: row.foreign_lang,
            text: row.foreign_text,
            confidence: row.confidence,
        });
    }

    let mut ranked: Vec<(bool, f64, MatchGroup)> = accs
        .into_iter()
        .map(|a| {
            (
                a.contains,
                a.max_conf,
                MatchGroup {
                    zh_text: a.zh_text,
                    cbeta_id: a.cbeta_id,
                    title_zh: a.title_zh,
                    juan_num: a.juan_num,
                    parallels: cap_per_lang(a.parallels, top),
                },
            )
        })
        .collect();

    ranked.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal))
    });
    ranked.into_iter().map(|(_, _, g)| g).collect()
}

/// List a text's aligned groups by Taishō id (case-insensitive), optionally
/// filtered to one juan, in juan-then-insertion order (canonical text order,
/// not relevance order — this is a browse, not a search).
pub fn by_cbeta(
    conn: &Connection,
    cbeta_id: &str,
    juan: Option<i64>,
    langs: Option<&[String]>,
    top: usize,
) -> rusqlite::Result<Vec<MatchGroup>> {
    let mut sql = String::from(
        "SELECT zh_text,zh_norm,foreign_lang,foreign_text,confidence,\
                cbeta_id,title_zh,juan_num \
         FROM parallels WHERE cbeta_id = ?1 COLLATE NOCASE",
    );
    if juan.is_some() {
        sql.push_str(" AND juan_num = ?2");
    }
    sql.push_str(" ORDER BY juan_num, id");
    let mut stmt = conn.prepare(&sql)?;
    let map_row = |r: &rusqlite::Row| {
        Ok(Row {
            zh_text: r.get(0)?,
            zh_norm: r.get(1)?,
            foreign_lang: r.get(2)?,
            foreign_text: r.get(3)?,
            confidence: r.get(4)?,
            cbeta_id: r.get(5)?,
            title_zh: r.get(6)?,
            juan_num: r.get(7)?,
        })
    };
    let rows: Vec<Row> = if let Some(j) = juan {
        stmt.query_map(rusqlite::params![cbeta_id, j], map_row)?
            .collect::<rusqlite::Result<_>>()?
    } else {
        stmt.query_map([cbeta_id], map_row)?
            .collect::<rusqlite::Result<_>>()?
    };
    // Group in encounter order (already canonical from ORDER BY).
    let mut groups: Vec<MatchGroup> = Vec::new();
    for row in rows {
        if let Some(filter) = langs {
            if !filter.iter().any(|l| l == &row.foreign_lang) {
                continue;
            }
        }
        let idx = groups.iter().position(|g| {
            g.zh_text == row.zh_text && g.cbeta_id == row.cbeta_id && g.juan_num == row.juan_num
        });
        let idx = match idx {
            Some(i) => i,
            None => {
                groups.push(MatchGroup {
                    zh_text: row.zh_text.clone(),
                    cbeta_id: row.cbeta_id.clone(),
                    title_zh: row.title_zh.clone(),
                    juan_num: row.juan_num,
                    parallels: Vec::new(),
                });
                groups.len() - 1
            }
        };
        groups[idx].parallels.push(Parallel {
            lang: row.foreign_lang,
            text: row.foreign_text,
            confidence: row.confidence,
        });
    }
    for g in &mut groups {
        g.parallels = cap_per_lang(std::mem::take(&mut g.parallels), top);
    }
    Ok(groups)
}

#[derive(Debug, serde::Serialize)]
pub struct TextEntry {
    pub cbeta_id: String,
    pub title_zh: String,
    /// (lang, aligned-segment count) sorted by lang code
    pub by_lang: Vec<(String, u64)>,
}

/// Fuzzy title search: normalizes both the stored (traditional) titles and the
/// caller-normalized keyword, then substring-matches. The distinct-title list
/// is small (~1k rows), so folding in Rust is cheap.
pub fn texts_matching(
    conn: &Connection,
    norm_keyword: &str,
    map: &crate::normalize::NormMap,
) -> rusqlite::Result<Vec<TextEntry>> {
    if norm_keyword.is_empty() {
        return Ok(vec![]);
    }
    let mut stmt = conn.prepare(
        "SELECT cbeta_id, title_zh, foreign_lang, COUNT(*) FROM parallels \
         WHERE cbeta_id IS NOT NULL AND title_zh IS NOT NULL \
         GROUP BY cbeta_id, title_zh, foreign_lang ORDER BY cbeta_id, foreign_lang",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, u64>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out: Vec<TextEntry> = Vec::new();
    for (cbeta_id, title_zh, lang, count) in rows {
        if !crate::normalize::normalize(&title_zh, map).contains(norm_keyword) {
            continue;
        }
        match out
            .iter_mut()
            .find(|e| e.cbeta_id == cbeta_id && e.title_zh == title_zh)
        {
            Some(e) => e.by_lang.push((lang, count)),
            None => out.push(TextEntry {
                cbeta_id,
                title_zh,
                by_lang: vec![(lang, count)],
            }),
        }
    }
    Ok(out)
}

fn cap_per_lang(parallels: Vec<Parallel>, top: usize) -> Vec<Parallel> {
    let mut by_lang: BTreeMap<String, Vec<Parallel>> = BTreeMap::new();
    for p in parallels {
        by_lang.entry(p.lang.clone()).or_default().push(p);
    }
    let mut out = Vec::new();
    for (_lang, mut items) in by_lang {
        items.sort_by(|a, b| {
            b.confidence
                .unwrap_or(0.0)
                .partial_cmp(&a.confidence.unwrap_or(0.0))
                .unwrap_or(Ordering::Equal)
        });
        items.truncate(top.max(1)); // floor of 1: top=0 would yield a useless 0-parallel result
        out.extend(items);
    }
    out
}
