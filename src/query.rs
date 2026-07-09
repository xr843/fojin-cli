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
