/// Max sentences processed when auto-splitting; the excess is reported, never
/// silently dropped.
pub const MAX_SEGMENTS: usize = 20;

pub struct SplitOutcome {
    pub segments: Vec<String>,
    pub truncated: usize,
}

/// Task 5 implements this. `keep` decides whether a raw segment survives
/// (callers pass a normalized-length predicate), keeping this file DB-free.
pub fn split_sentences(_raw: &str, _keep: impl Fn(&str) -> bool) -> SplitOutcome {
    SplitOutcome {
        segments: Vec::new(),
        truncated: 0,
    }
}
