use fojin_cli::search::fallback::longest_matching;

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
    let probe = |c: &str| Ok(c == "甲乙" || c == "丙丁");
    let fb = longest_matching("甲乙丙丁", probe).unwrap().unwrap();
    assert_eq!(fb.matched_substring, "甲乙");
    assert_eq!(fb.char_len, 2);
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
fn declines_queries_shorter_than_three_chars() {
    let probe = |_: &str| Ok(true);
    assert!(longest_matching("色空", probe).unwrap().is_none());
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
