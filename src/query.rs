use crate::model::{MatchGroup, Parallel};
use rusqlite::Connection;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

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
    Ok(group_and_rank(
        rows,
        norm_query,
        langs,
        top,
        MatchColumn::ZhNorm,
    ))
}

/// Shortest needle the FTS5 trigram index can serve. Below it, matching falls
/// back to `instr()` over the whole `parallels` table — ~900k rows, no index —
/// so any hot path that probes repeatedly must stay at or above this floor.
pub const FTS_MIN_CHARS: usize = 3;

/// Existence check behind the zero-hit fallback probe: would `search` return
/// anything at all for `norm_query`? Answers with `SELECT 1 … LIMIT 1` instead
/// of materializing every matching row and then grouping and ranking it, which
/// is all wasted work when the caller only wants a yes/no.
///
/// The `--lang` filter is applied here for the same reason `group_and_rank`
/// applies it: a segment whose parallels are all excluded by `--lang` is not a
/// hit for that invocation. Without the filter the fallback would point the
/// reader at a substring the very same command refuses to display.
///
/// Mirrors `fetch_rows`' index choice, so `exists` and `!search(…).is_empty()`
/// agree at every length. The fallback only ever calls it with at least
/// `FTS_MIN_CHARS` characters, which keeps every probe off the `instr` scan.
pub fn exists(
    conn: &Connection,
    norm_query: &str,
    langs: Option<&[String]>,
) -> rusqlite::Result<bool> {
    if norm_query.is_empty() {
        return Ok(false);
    }
    // `group_and_rank` keeps a row only if its language is listed, so an empty
    // list keeps nothing. Short-circuit rather than emit `IN ()`.
    if langs.is_some_and(|codes| codes.is_empty()) {
        return Ok(false);
    }
    let (source, param) = if norm_query.chars().count() >= FTS_MIN_CHARS {
        (
            "parallels_fts f JOIN parallels p ON p.id=f.rowid \
             WHERE parallels_fts MATCH ?1",
            fts_quote(norm_query),
        )
    } else {
        (
            "parallels p WHERE instr(p.zh_norm, ?1) > 0",
            norm_query.to_owned(),
        )
    };
    let mut sql = format!("SELECT 1 FROM {source}");
    let mut params: Vec<String> = vec![param];
    if let Some(codes) = langs {
        let placeholders = (0..codes.len())
            .map(|i| format!("?{}", i + 2))
            .collect::<Vec<_>>()
            .join(",");
        sql.push_str(&format!(" AND p.foreign_lang IN ({placeholders})"));
        params.extend(codes.iter().cloned());
    }
    sql.push_str(" LIMIT 1");
    let mut stmt = conn.prepare(&sql)?;
    stmt.exists(rusqlite::params_from_iter(params.iter()))
}

/// Minimum characters for a reverse query. IAST bigrams like `ka` match tens of
/// thousands of rows and carry no parallel-reading value; this mirrors the
/// 2-character floor on the Chinese side.
pub const MIN_FOREIGN_QUERY_CHARS: usize = 3;

/// Reverse lookup: find Chinese segments whose `from_lang` parallel contains
/// `raw_query`. Matching happens in Rust with full Unicode case folding —
/// SQLite's `instr` is case-sensitive and its `LOWER()` is ASCII-only, so
/// `tasmāc` would never find the stored `Tasmāc`.
pub fn search_foreign(
    conn: &Connection,
    from_lang: &str,
    raw_query: &str,
    langs: Option<&[String]>,
    top: usize,
) -> rusqlite::Result<Vec<MatchGroup>> {
    let needle = raw_query.trim().to_lowercase();
    if needle.is_empty() {
        return Ok(vec![]);
    }
    let query_chars = needle.chars().count();

    // Pass 1: locate hit groups, scanning only the source language's rows.
    // The (exact, excess_chars) ranking signal is recorded here, from the
    // actual matching `foreign_text` — it must NOT be recomputed later from
    // whatever row happens to survive the caller's display `--lang` filter,
    // since that row can be in a different, non-matching language (see
    // group_and_rank's `MatchColumn::Precomputed` arm). A group can have
    // several `from_lang` witnesses; keep the best signal per group: `exact`
    // is OR'd, `excess_chars` is MIN'd. Both are commutative/associative, so
    // no tie-break on processing order is needed for determinism.
    let mut stmt = conn.prepare(
        "SELECT zh_text, cbeta_id, juan_num, foreign_text \
         FROM parallels WHERE foreign_lang = ?1",
    )?;
    let mut hit_signals: HashMap<GroupKey, (bool, usize)> = HashMap::new();
    let mut rows = stmt.query([from_lang])?;
    while let Some(row) = rows.next()? {
        let foreign_text: String = row.get(3)?;
        let hay = foreign_text.to_lowercase();
        if !hay.contains(&needle) {
            continue;
        }
        let exact = hay == needle;
        let excess_chars = hay.chars().count().saturating_sub(query_chars);
        let key = GroupKey {
            zh_text: row.get(0)?,
            cbeta_id: row.get(1)?,
            juan_num: row.get(2)?,
        };
        hit_signals
            .entry(key)
            .and_modify(|signal| {
                signal.0 |= exact;
                if excess_chars < signal.1 {
                    signal.1 = excess_chars;
                }
            })
            .or_insert((exact, excess_chars));
    }
    if hit_signals.is_empty() {
        return Ok(vec![]);
    }

    // Pass 2: pull every language's rows for the hit groups, narrowed to
    // their `cbeta_id`s so we don't materialize the whole table (~900k rows,
    // no index on `cbeta_id`) just to keep a handful of groups. `cbeta_id`
    // is nullable and SQL `IN` never matches NULL, so hit groups with a NULL
    // `cbeta_id` are fetched separately via a dedicated `IS NULL` query.
    // SQLite's default bound-parameter limit is 999 and the dataset has
    // ~1016 distinct `cbeta_id`s, so the `IN` list is batched — the whole
    // dataset fits in two batches at `CBETA_ID_BATCH`, comfortably under the
    // limit, rather than risking "too many SQL variables" or silently
    // falling back to the unbounded scan this fix exists to avoid.
    const CBETA_ID_BATCH: usize = 500;
    let mut cbeta_ids: Vec<String> = Vec::new();
    let mut seen_ids: HashSet<&str> = HashSet::new();
    let mut has_null_cbeta_id = false;
    for key in hit_signals.keys() {
        match &key.cbeta_id {
            Some(id) => {
                if seen_ids.insert(id.as_str()) {
                    cbeta_ids.push(id.clone());
                }
            }
            None => has_null_cbeta_id = true,
        }
    }

    let mut collected: Vec<Row> = Vec::new();
    for batch in cbeta_ids.chunks(CBETA_ID_BATCH) {
        let placeholders = std::iter::repeat_n("?", batch.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT zh_text,zh_norm,foreign_lang,foreign_text,confidence,\
                    cbeta_id,title_zh,juan_num \
             FROM parallels WHERE cbeta_id IN ({placeholders})"
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(batch.iter()))?;
        collect_hit_rows(&mut rows, &hit_signals, &mut collected)?;
    }
    if has_null_cbeta_id {
        let mut stmt = conn.prepare(
            "SELECT zh_text,zh_norm,foreign_lang,foreign_text,confidence,\
                    cbeta_id,title_zh,juan_num \
             FROM parallels WHERE cbeta_id IS NULL",
        )?;
        let mut rows = stmt.query([])?;
        collect_hit_rows(&mut rows, &hit_signals, &mut collected)?;
    }

    Ok(group_and_rank(
        collected,
        &needle,
        langs,
        top,
        MatchColumn::Precomputed(&hit_signals),
    ))
}

/// Shared row-materializing loop for pass 2's batched queries: keep only
/// rows whose full group key (`zh_text`, `cbeta_id`, `juan_num`) is an
/// actual pass-1 hit — the `cbeta_id`-only SQL narrowing can still return
/// siblings under the same text that aren't part of any hit group (same
/// `cbeta_id`, different juan or segment).
fn collect_hit_rows(
    rows: &mut rusqlite::Rows<'_>,
    hit_signals: &HashMap<GroupKey, (bool, usize)>,
    collected: &mut Vec<Row>,
) -> rusqlite::Result<()> {
    while let Some(r) = rows.next()? {
        let row = Row {
            zh_text: r.get(0)?,
            zh_norm: r.get(1)?,
            foreign_lang: r.get(2)?,
            foreign_text: r.get(3)?,
            confidence: r.get(4)?,
            cbeta_id: r.get(5)?,
            title_zh: r.get(6)?,
            juan_num: r.get(7)?,
        };
        let key = GroupKey {
            zh_text: row.zh_text.clone(),
            cbeta_id: row.cbeta_id.clone(),
            juan_num: row.juan_num,
        };
        if hit_signals.contains_key(&key) {
            collected.push(row);
        }
    }
    Ok(())
}

fn fts_quote(q: &str) -> String {
    format!("\"{}\"", q.replace('"', "\"\""))
}

fn fetch_rows(conn: &Connection, norm_query: &str) -> rusqlite::Result<Vec<Row>> {
    let (sql, param) = if norm_query.chars().count() >= FTS_MIN_CHARS {
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
             FROM parallels WHERE instr(zh_norm, ?1) > 0",
            norm_query.to_owned(),
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

#[derive(Hash, PartialEq, Eq)]
struct GroupKey {
    zh_text: String,
    cbeta_id: Option<String>,
    juan_num: Option<i64>,
}

/// How a row's ranking signal (`exact`, `excess_chars`) is determined.
///
/// `ZhNorm` (forward search) computes it per row from `zh_norm`, which is
/// identical across every row in a group, so per-row computation is exact.
/// `Precomputed` (reverse search) instead looks up a per-group signal that
/// pass 1 already derived from the actual `from_lang`-matching text — it
/// must NOT be recomputed from `row.foreign_text` here, because by the time
/// a row reaches this loop it may have been kept only for *display*
/// (post `--lang` filtering) and be in a language that never matched the
/// query at all.
#[derive(Clone, Copy)]
enum MatchColumn<'a> {
    ZhNorm,
    Precomputed(&'a HashMap<GroupKey, (bool, usize)>),
}

struct Acc {
    zh_text: String,
    cbeta_id: Option<String>,
    title_zh: Option<String>,
    juan_num: Option<i64>,
    exact: bool,
    excess_chars: usize,
    max_conf: f64,
    parallels: Vec<Parallel>,
}

fn group_and_rank(
    rows: Vec<Row>,
    needle: &str,
    langs: Option<&[String]>,
    top: usize,
    column: MatchColumn,
) -> Vec<MatchGroup> {
    let mut accs: Vec<Acc> = Vec::new();
    let mut acc_idx: HashMap<GroupKey, usize> = HashMap::new();
    let query_chars = needle.chars().count();
    for row in rows {
        if let Some(filter) = langs {
            if !filter.iter().any(|l| l == &row.foreign_lang) {
                continue;
            }
        }
        let key = GroupKey {
            zh_text: row.zh_text.clone(),
            cbeta_id: row.cbeta_id.clone(),
            juan_num: row.juan_num,
        };
        let (exact, excess_chars) = match column {
            MatchColumn::ZhNorm => {
                let hay = row.zh_norm.as_str();
                let contains = hay.contains(needle);
                let excess_chars = if contains {
                    hay.chars().count().saturating_sub(query_chars)
                } else {
                    hay.chars().count()
                };
                (hay == needle, excess_chars)
            }
            MatchColumn::Precomputed(signals) => *signals
                .get(&key)
                .expect("pass 1 recorded a signal for every hit group key reaching pass 2"),
        };
        let idx = match acc_idx.get(&key).copied() {
            Some(i) => i,
            None => {
                accs.push(Acc {
                    zh_text: row.zh_text.clone(),
                    cbeta_id: row.cbeta_id.clone(),
                    title_zh: row.title_zh.clone(),
                    juan_num: row.juan_num,
                    exact: false,
                    excess_chars,
                    max_conf: 0.0,
                    parallels: Vec::new(),
                });
                let idx = accs.len() - 1;
                acc_idx.insert(key, idx);
                idx
            }
        };
        let conf = row.confidence.unwrap_or(0.0);
        if conf > accs[idx].max_conf {
            accs[idx].max_conf = conf;
        }
        accs[idx].exact |= exact;
        if excess_chars < accs[idx].excess_chars {
            accs[idx].excess_chars = excess_chars;
        }
        accs[idx].parallels.push(Parallel {
            lang: row.foreign_lang,
            text: row.foreign_text,
            confidence: row.confidence,
        });
    }

    accs.sort_by(|a, b| {
        b.exact
            .cmp(&a.exact)
            .then(a.excess_chars.cmp(&b.excess_chars))
            .then_with(|| b.max_conf.total_cmp(&a.max_conf))
            .then_with(|| a.cbeta_id.cmp(&b.cbeta_id))
            .then_with(|| a.juan_num.cmp(&b.juan_num))
            .then_with(|| a.zh_text.cmp(&b.zh_text))
    });

    accs.into_iter()
        .map(|a| MatchGroup {
            zh_text: a.zh_text,
            cbeta_id: a.cbeta_id,
            title_zh: a.title_zh,
            juan_num: a.juan_num,
            parallels: cap_per_lang(a.parallels, top),
        })
        .collect()
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
    let count = |row: &rusqlite::Row<'_>, index: usize| -> rusqlite::Result<u64> {
        let value = row.get::<_, i64>(index)?;
        u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
    };
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                count(r, 3)?,
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
        let items = by_lang.entry(p.lang.clone()).or_default();
        if let Some(existing) = items.iter_mut().find(|existing| existing.text == p.text) {
            if p.confidence
                .unwrap_or(0.0)
                .total_cmp(&existing.confidence.unwrap_or(0.0))
                == Ordering::Greater
            {
                *existing = p;
            }
        } else {
            items.push(p);
        }
    }
    let mut out = Vec::new();
    for (_lang, mut items) in by_lang {
        items.sort_by(|a, b| {
            b.confidence
                .unwrap_or(0.0)
                .total_cmp(&a.confidence.unwrap_or(0.0))
                .then_with(|| a.text.cmp(&b.text))
        });
        items.truncate(top.max(1)); // floor of 1: top=0 would yield a useless 0-parallel result
        out.extend(items);
    }
    out
}
