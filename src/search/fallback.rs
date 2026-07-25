use anyhow::Result;

use super::FallbackInfo;

/// Beyond this length the O(n log n) probe budget is not worth spending.
pub const MAX_FALLBACK_CHARS: usize = 60;

/// Task 4 implements this. `probe` reports whether a candidate substring has
/// any alignment, so this file needs no database.
pub fn longest_matching(
    norm_query: &str,
    probe: impl Fn(&str) -> Result<bool>,
) -> Result<Option<FallbackInfo>> {
    let chars: Vec<char> = norm_query.chars().collect();
    let n = chars.len();
    // n < 3 leaves no proper substring of length >= 2 to try.
    if !(3..=MAX_FALLBACK_CHARS).contains(&n) {
        return Ok(None);
    }

    let substring = |start: usize, len: usize| -> String {
        chars[start..start + len].iter().collect::<String>()
    };
    // Scans starts left to right, so the earliest start wins at a given length.
    let first_hit = |len: usize| -> Result<Option<usize>> {
        for start in 0..=(n - len) {
            if probe(&substring(start, len))? {
                return Ok(Some(start));
            }
        }
        Ok(None)
    };

    // "Some substring of length L matches" is monotonically decreasing in L,
    // because every substring of a matching substring also matches. Binary
    // search the largest feasible L instead of scanning every length.
    let mut best: Option<(usize, usize)> = None;
    let (mut lo, mut hi) = (2usize, n - 1);
    while lo <= hi {
        let mid = lo + (hi - lo) / 2;
        match first_hit(mid)? {
            Some(start) => {
                best = Some((start, mid));
                lo = mid + 1;
            }
            None => {
                hi = mid - 1;
            }
        }
    }

    Ok(best.map(|(start, len)| FallbackInfo {
        matched_substring: substring(start, len),
        char_len: len,
    }))
}
