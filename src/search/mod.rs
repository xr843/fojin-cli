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

/// Skeleton: whole-string query only. Task 3 adds the `from` branch; Task 6
/// adds splitting and fallback.
pub fn run(conn: &Connection, req: &SearchRequest) -> Result<SearchOutcome> {
    let map = normalize::load_norm_map(conn)?;
    let norm = normalize::normalize(req.raw.trim(), &map);
    normalize::validate_query_length(&norm)?;
    let groups = query::search(conn, &norm, req.langs, req.top)?;
    Ok(SearchOutcome::plain(groups, req.limit))
}
