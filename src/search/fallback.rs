use anyhow::Result;

use super::FallbackInfo;

/// Beyond this length the O(n log n) probe budget is not worth spending.
pub const MAX_FALLBACK_CHARS: usize = 60;

/// Task 4 implements this. `probe` reports whether a candidate substring has
/// any alignment, so this file needs no database.
pub fn longest_matching(
    _norm_query: &str,
    _probe: impl Fn(&str) -> Result<bool>,
) -> Result<Option<FallbackInfo>> {
    Ok(None)
}
