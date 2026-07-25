use fojin_cli::query::FTS_MIN_CHARS;
use fojin_cli::search::fallback::{longest_matching, MIN_FALLBACK_CHARS};

#[test]
fn finds_longest_present_substring() {
    let corpus = "色不異空，空不異色";
    let probe = |c: &str| Ok(corpus.contains(c));
    let fb = longest_matching("舍利子色不異空義", probe)
        .unwrap()
        .unwrap();
    assert_eq!(fb.matched_substring, "色不異空");
    assert_eq!(fb.char_len, 4);
}

#[test]
fn returns_none_when_nothing_matches() {
    let probe = |_: &str| Ok(false);
    assert!(longest_matching("涅槃寂靜", probe).unwrap().is_none());
}

#[test]
fn prefers_earliest_start_among_equal_lengths() {
    // Two candidates of the same (minimum) length match; the earlier start
    // must win.
    let probe = |c: &str| Ok(c == "甲乙丙" || c == "丙丁戊");
    let fb = longest_matching("甲乙丙丁戊", probe).unwrap().unwrap();
    assert_eq!(fb.matched_substring, "甲乙丙");
    assert_eq!(fb.char_len, 3);
}

#[test]
fn never_returns_the_whole_query() {
    // The whole string already failed upstream; probing it again is wasted work
    // and would report a "fallback" identical to the failed query.
    let probe = |_: &str| Ok(true);
    let fb = longest_matching("色即是空", probe).unwrap().unwrap();
    assert_eq!(fb.char_len, 3);
}

#[test]
fn declines_queries_too_short_to_hold_a_proper_substring() {
    // A candidate must be a *proper* substring of at least MIN_FALLBACK_CHARS
    // characters, so a query needs MIN_FALLBACK_CHARS + 1 characters before
    // anything is probeable. Both 2 and 3 characters are below that.
    let probe = |_: &str| Ok(true);
    assert!(longest_matching("色空", probe).unwrap().is_none());
    assert!(longest_matching("色空義", probe).unwrap().is_none());
}

#[test]
fn skips_pathologically_long_input() {
    let long = "空".repeat(61);
    let probe = |_: &str| Ok(true);
    assert!(longest_matching(&long, probe).unwrap().is_none());
}

#[test]
fn probes_are_bounded_for_a_long_query() {
    use std::cell::Cell;
    let calls = Cell::new(0usize);
    let probe = |_: &str| {
        calls.set(calls.get() + 1);
        Ok(false)
    };
    let _ = longest_matching(&"空".repeat(40), probe).unwrap();
    assert!(
        calls.get() < 400,
        "binary search over lengths must stay well under the O(n^2) naive scan, got {}",
        calls.get()
    );
}

#[test]
fn propagates_probe_error() {
    let probe = |_: &str| Err(anyhow::anyhow!("db exploded"));
    assert!(longest_matching("色即是空", probe).is_err());
}

#[test]
fn accepts_four_char_query_at_lower_bound() {
    let probe = |_: &str| Ok(true);
    let fb = longest_matching("色空義理", probe).unwrap().unwrap();
    assert_eq!(fb.char_len, 3);
}

#[test]
fn never_probes_a_candidate_below_the_fts_floor() {
    // The whole point of the MIN_FALLBACK_CHARS floor: a shorter candidate
    // would leave the trigram index and scan all ~900k rows, once per start,
    // once per segment. Watch every candidate the search actually issues.
    use std::cell::Cell;
    let shortest = Cell::new(usize::MAX);
    let probe = |c: &str| {
        shortest.set(shortest.get().min(c.chars().count()));
        Ok(false)
    };
    let _ = longest_matching(&"空".repeat(40), probe).unwrap();
    assert!(
        shortest.get() >= FTS_MIN_CHARS,
        "probed a {}-char candidate, below the FTS floor of {FTS_MIN_CHARS}",
        shortest.get()
    );
    // …and refuse to compile if a future edit lowers the floor itself.
    const { assert!(MIN_FALLBACK_CHARS >= FTS_MIN_CHARS) };
}

#[test]
fn accepts_sixty_char_query_at_upper_bound() {
    let long = "空".repeat(60);
    let probe = |_: &str| Ok(true);
    let fb = longest_matching(&long, probe).unwrap().unwrap();
    assert_eq!(fb.char_len, 59);
}
