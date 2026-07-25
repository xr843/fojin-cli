/// Max sentences processed when auto-splitting; the excess is reported, never
/// silently dropped.
pub const MAX_SEGMENTS: usize = 20;

/// Sentence-level punctuation only. Book-title brackets, quotes, parentheses
/// and the interpunct are deliberately absent: they occur mid-sentence, and
/// breaking there would cut phrases in half.
pub const SPLIT_CHARS: &str = "，。；：！？、,.;:!?\n\r";

pub struct SplitOutcome {
    pub segments: Vec<String>,
    pub truncated: usize,
}

/// Split raw (un-normalized) input into sentences. Splitting must happen before
/// `normalize()`, which strips the very punctuation this relies on.
///
/// `keep` decides whether a trimmed segment survives; callers pass a
/// normalized-length predicate, which keeps this function database-free.
pub fn split_sentences(raw: &str, keep: impl Fn(&str) -> bool) -> SplitOutcome {
    let kept: Vec<String> = raw
        .split(|c: char| SPLIT_CHARS.contains(c))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter(|s| keep(s))
        .map(str::to_string)
        .collect();
    let truncated = kept.len().saturating_sub(MAX_SEGMENTS);
    let mut segments = kept;
    segments.truncate(MAX_SEGMENTS);
    SplitOutcome {
        segments,
        truncated,
    }
}
