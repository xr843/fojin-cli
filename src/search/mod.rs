use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

use crate::model::MatchGroup;
use crate::{normalize, query};

pub mod fallback;
pub mod split;

/// Bumped when the `--json` shape of `parallel` changes incompatibly.
pub const SCHEMA_VERSION: u32 = 1;
/// Per-segment display cap in split mode; `--all` lifts it.
pub const SEGMENT_GROUP_CAP: usize = 3;

#[derive(Debug, Clone, Serialize)]
pub struct FallbackInfo {
    pub matched_substring: String,
    pub char_len: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SegmentResult {
    pub text: String,
    pub matched: bool,
    pub total: usize,
    pub groups: Vec<MatchGroup>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<FallbackInfo>,
}

#[derive(Debug)]
pub struct SearchOutcome {
    /// Deduplicated hits across every path, already capped by `--limit`.
    pub groups: Vec<MatchGroup>,
    /// Hit count before the `--limit` cap.
    pub total: usize,
    pub segments: Option<Vec<SegmentResult>>,
    pub fallback: Option<FallbackInfo>,
    /// Segments dropped by the MAX_SEGMENTS cap; rendered, never silent.
    pub truncated_segments: usize,
}

impl SearchOutcome {
    /// The plain hit path: no splitting, no fallback.
    fn plain(groups: Vec<MatchGroup>, limit: Option<usize>) -> Self {
        let total = groups.len();
        let shown = match limit {
            Some(n) => n.min(total),
            None => total,
        };
        let mut groups = groups;
        groups.truncate(shown);
        SearchOutcome {
            groups,
            total,
            segments: None,
            fallback: None,
            truncated_segments: 0,
        }
    }
}

pub struct SearchRequest<'a> {
    pub raw: &'a str,
    pub langs: Option<&'a [String]>,
    pub top: usize,
    pub limit: Option<usize>,
    pub from: Option<&'a str>,
    pub no_split: bool,
}

type GroupKey = (String, Option<String>, Option<i64>);

fn group_key(g: &MatchGroup) -> GroupKey {
    (g.zh_text.clone(), g.cbeta_id.clone(), g.juan_num)
}

pub fn run(conn: &Connection, req: &SearchRequest) -> Result<SearchOutcome> {
    if let Some(from_lang) = req.from {
        let needle = req.raw.trim();
        if needle.chars().count() < query::MIN_FOREIGN_QUERY_CHARS {
            anyhow::bail!(
                "反向查询至少需要 {} 个字符;更短的串会命中过多,无对读价值",
                query::MIN_FOREIGN_QUERY_CHARS
            );
        }
        let groups = query::search_foreign(conn, from_lang, needle, req.langs, req.top)?;
        return Ok(SearchOutcome::plain(groups, req.limit));
    }

    let map = normalize::load_norm_map(conn)?;
    let raw = req.raw.trim();
    let norm = normalize::normalize(raw, &map);
    normalize::validate_query_length(&norm)?;

    let groups = query::search(conn, &norm, req.langs, req.top)?;
    if !groups.is_empty() {
        return Ok(SearchOutcome::plain(groups, req.limit));
    }

    // Existence only: `query::exists` stops at the first displayable row, and
    // applies the same `--lang` filter `query::search` would, so a substring is
    // only suggested when this very invocation could show it.
    let probe = |candidate: &str| -> Result<bool> {
        query::exists(conn, candidate, req.langs).map_err(Into::into)
    };

    if !req.no_split {
        let keep = |seg: &str| {
            normalize::normalize(seg, &map).chars().count() >= normalize::MIN_QUERY_CHARS
        };
        let split = split::split_sentences(raw, keep);
        if split.segments.len() >= 2 {
            return split_search(conn, req, &map, &split, &probe);
        }
    }

    Ok(SearchOutcome {
        groups: Vec::new(),
        total: 0,
        segments: None,
        fallback: fallback::longest_matching(&norm, probe)?,
        truncated_segments: 0,
    })
}

fn split_search(
    conn: &Connection,
    req: &SearchRequest,
    map: &normalize::NormMap,
    split: &split::SplitOutcome,
    probe: &impl Fn(&str) -> Result<bool>,
) -> Result<SearchOutcome> {
    let mut merged: Vec<MatchGroup> = Vec::new();
    let mut seen: std::collections::HashSet<GroupKey> = std::collections::HashSet::new();
    let mut segments: Vec<SegmentResult> = Vec::new();

    for text in &split.segments {
        let seg_norm = normalize::normalize(text, map);
        let seg_groups = query::search(conn, &seg_norm, req.langs, req.top)?;
        let total = seg_groups.len();

        // Stable append-then-dedupe: segment order decides position, so output
        // never drifts with the number of segments.
        for g in &seg_groups {
            if seen.insert(group_key(g)) {
                merged.push(g.clone());
            }
        }

        let fb = if total == 0 {
            fallback::longest_matching(&seg_norm, probe)?
        } else {
            None
        };
        // `--all` (limit None) lifts the per-segment cap.
        let shown = match req.limit {
            Some(n) => n.min(SEGMENT_GROUP_CAP).min(total),
            None => total,
        };
        let mut groups = seg_groups;
        groups.truncate(shown);

        segments.push(SegmentResult {
            text: text.clone(),
            matched: total > 0,
            total,
            groups,
            fallback: fb,
        });
    }

    let total = merged.len();
    let shown = match req.limit {
        Some(n) => n.min(total),
        None => total,
    };
    merged.truncate(shown);

    Ok(SearchOutcome {
        groups: merged,
        total,
        segments: Some(segments),
        fallback: None,
        truncated_segments: split.truncated,
    })
}
