use anyhow::Result;

use super::FallbackInfo;

/// Beyond this length the O(n log n) probe budget is not worth spending.
pub const MAX_FALLBACK_CHARS: usize = 60;

/// Shortest substring worth suggesting. Two characters match almost any of the
/// ~900k aligned segments, so a 2-character hint carried no information; the
/// tool's own guidance is that 4–12 character phrases match best. The floor
/// also keeps every probe at or above `query::FTS_MIN_CHARS`, i.e. on the
/// trigram index instead of a full-table `instr` scan — see the test in
/// `tests/fallback.rs` that pins the two together.
pub const MIN_FALLBACK_CHARS: usize = 3;

/// Longest *proper* substring of `norm_query` that `probe` accepts, earliest
/// start winning among equally long candidates; `None` when nothing of at
/// least `MIN_FALLBACK_CHARS` characters matches. `probe` reports whether a
/// candidate has any alignment, so this file needs no database.
pub fn longest_matching(
    norm_query: &str,
    probe: impl Fn(&str) -> Result<bool>,
) -> Result<Option<FallbackInfo>> {
    let chars: Vec<char> = norm_query.chars().collect();
    let n = chars.len();
    // Candidates are proper substrings, so the longest has n - 1 characters:
    // below MIN_FALLBACK_CHARS + 1 there is nothing long enough to probe.
    if !((MIN_FALLBACK_CHARS + 1)..=MAX_FALLBACK_CHARS).contains(&n) {
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
    let (mut lo, mut hi) = (MIN_FALLBACK_CHARS, n - 1);
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
