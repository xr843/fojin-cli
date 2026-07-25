use fojin_cli::schema::init_schema;
use fojin_cli::search::{run, SearchRequest};
use rusqlite::{params, Connection};

fn fixture() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    init_schema(&conn).unwrap();
    for (zt, zn, f) in [
        ("觀自在菩薩", "观自在菩萨", "avalokiteśvara"),
        ("照見五蘊皆空", "照见五蕴皆空", "pañcaskandhāḥ śūnyāḥ"),
    ] {
        conn.execute(
            "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
             VALUES (?1,?2,'sa',?3,1.0,'T0251','心經',1)",
            params![zt, zn, f],
        )
        .unwrap();
    }
    for (from, to) in [("觀", "观"), ("薩", "萨"), ("見", "见"), ("蘊", "蕴")] {
        conn.execute(
            "INSERT INTO norm_map(from_char,to_char) VALUES (?1,?2)",
            params![from, to],
        )
        .unwrap();
    }
    conn
}

fn req<'a>(raw: &'a str, no_split: bool) -> SearchRequest<'a> {
    req_with_limit(raw, no_split, Some(10))
}

/// Same as `req`, but lets the caller pick `limit` explicitly — needed for
/// `--all` (`limit: None`) scenarios without disturbing every other test.
fn req_with_limit<'a>(raw: &'a str, no_split: bool, limit: Option<usize>) -> SearchRequest<'a> {
    SearchRequest {
        raw,
        langs: None,
        top: 3,
        limit,
        from: None,
        no_split,
    }
}

#[test]
fn hit_path_sets_no_segments_and_no_fallback() {
    let conn = fixture();
    let out = run(&conn, &req("觀自在菩薩", false)).unwrap();
    assert_eq!(out.total, 1);
    assert!(out.segments.is_none(), "a hit must not trigger splitting");
    assert!(out.fallback.is_none());
}

#[test]
fn zero_hit_long_input_splits_and_merges_segment_hits() {
    let conn = fixture();
    // Neither half spans one stored segment, so the whole string finds nothing;
    // each sentence on its own does.
    let out = run(&conn, &req("觀自在菩薩，照見五蘊皆空", false)).unwrap();
    let segments = out.segments.expect("split must have engaged");
    assert_eq!(segments.len(), 2);
    assert!(segments.iter().all(|s| s.matched));
    assert_eq!(out.total, 2, "top-level groups merge both segments' hits");
}

#[test]
fn no_split_flag_suppresses_splitting() {
    let conn = fixture();
    let out = run(&conn, &req("觀自在菩薩，照見五蘊皆空", true)).unwrap();
    assert!(out.segments.is_none());
    assert_eq!(out.total, 0);
}

#[test]
fn merged_groups_are_deduplicated() {
    let conn = fixture();
    // Both sentences hit the same stored group, which must appear once.
    let out = run(&conn, &req("觀自在，自在菩薩", false)).unwrap();
    assert_eq!(out.total, 1, "same group reached twice must dedupe");
}

#[test]
fn merged_groups_preserve_segment_order_over_relevance() {
    let conn = fixture();
    // Two distinct groups, one per segment, neither overlapping the base
    // fixture rows so each segment yields exactly one hit.
    //
    // Segment 1's group is deliberately the WORSE one on every signal
    // `query::search`'s own ranking (group_and_rank in src/query.rs) uses as
    // a tie-break once exact-match is equal: lower confidence, and a
    // cbeta_id that sorts later alphabetically. Segment 2's group is the
    // exact opposite: higher confidence, alphabetically-earlier cbeta_id.
    // Both are exact, zero-excess matches within their own segment, so
    // within-segment ranking never distinguishes them either.
    //
    // The merge contract is "segment order first, stable dedup" — never a
    // global relevance re-sort. If a future change fed `merged` back through
    // group_and_rank's own comparator (exact, excess_chars, confidence desc,
    // cbeta_id, juan_num, zh_text) — the most tempting "obviously correct"
    // refactor, since that comparator already exists in the codebase — or
    // even just sorted by cbeta_id ascending, segment 2's group would win on
    // confidence AND on cbeta_id and jump to position 0. Our assertion pins
    // segment 1's (objectively "worse") group at position 0, so either kind
    // of accidental global re-sort flips the order and fails the test.
    // Written directly in simplified form (zh_text == zh_norm) so no
    // norm_map traditional->simplified conversion is in play; the raw query
    // below is the same simplified text, keeping normalization a no-op and
    // the exact-match/excess_chars signals unambiguous.
    conn.execute(
        "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
         VALUES ('大慈大悲观世音','大慈大悲观世音','sa','avalokita-karuna',0.1,'T9001','甲经',1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
         VALUES ('南无阿弥陀佛圣号','南无阿弥陀佛圣号','sa','namo amitabha',0.99,'T1002','乙经',1)",
        [],
    )
    .unwrap();

    let out = run(&conn, &req("大慈大悲观世音，南无阿弥陀佛圣号", false)).unwrap();
    let segments = out.segments.expect("split must have engaged");
    assert_eq!(segments.len(), 2);
    assert!(
        segments.iter().all(|s| s.matched),
        "each segment must hit its own distinct group"
    );
    assert_eq!(out.total, 2, "two distinct groups, no dedup collision");
    assert_eq!(
        out.groups.len(),
        2,
        "both groups survive the (unhit) --limit cap"
    );

    // Position 0 MUST be segment 1's group, even though it is strictly
    // worse on confidence (0.1 vs 0.99) and cbeta_id ordering (T9001 vs
    // T1002) than segment 2's group.
    assert_eq!(out.groups[0].zh_text, "大慈大悲观世音");
    assert_eq!(out.groups[0].cbeta_id.as_deref(), Some("T9001"));
    assert_eq!(out.groups[1].zh_text, "南无阿弥陀佛圣号");
    assert_eq!(out.groups[1].cbeta_id.as_deref(), Some("T1002"));
}

#[test]
fn empty_segment_gets_its_own_fallback() {
    let conn = fixture();
    // Segment 2 matches nothing whole, but 觀自在菩薩 (normalized 观自在菩萨)
    // sits inside it and IS in the fixture — that is what fallback must find.
    let out = run(&conn, &req("涅槃寂靜，觀自在菩薩摩訶薩", false)).unwrap();
    let segments = out.segments.unwrap();
    let miss = segments
        .iter()
        .find(|s| s.text == "觀自在菩薩摩訶薩")
        .unwrap();
    assert!(!miss.matched);
    let fb = miss
        .fallback
        .as_ref()
        .expect("a segment with a matching substring must carry a fallback");
    assert_eq!(fb.matched_substring, "观自在菩萨");
    assert_eq!(fb.char_len, 5);
}

#[test]
fn segment_with_no_matching_substring_has_no_fallback() {
    let conn = fixture();
    let out = run(&conn, &req("觀自在菩薩，涅槃寂靜無為", false)).unwrap();
    let segments = out.segments.unwrap();
    let miss = segments.iter().find(|s| !s.matched).unwrap();
    assert_eq!(miss.text, "涅槃寂靜無為");
    assert!(
        miss.fallback.is_none(),
        "nothing in this fixture matches any substring of 涅槃寂靜無為"
    );
}

#[test]
fn unsplittable_zero_hit_query_falls_back_on_the_whole_string() {
    let conn = fixture();
    // No sentence punctuation, so there is nothing to split; the whole string
    // gets the substring fallback, and 觀自在菩薩 is present in the fixture.
    let out = run(&conn, &req("觀自在菩薩摩訶薩", false)).unwrap();
    assert!(out.segments.is_none());
    let fb = out.fallback.expect("whole-string fallback must engage");
    assert_eq!(fb.matched_substring, "观自在菩萨");
    assert_eq!(fb.char_len, 5);
}

#[test]
fn fallback_substring_is_reported_in_normalized_form() {
    let conn = fixture();
    let out = run(&conn, &req("觀自在菩薩摩訶薩", false)).unwrap();
    let fb = out.fallback.unwrap();
    assert!(
        fb.matched_substring.contains('观'),
        "fallback reports the normalized (folded) form: {}",
        fb.matched_substring
    );
}

#[test]
fn segment_display_is_capped_but_total_is_honest() {
    let conn = fixture();
    for i in 0..5 {
        conn.execute(
            "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
             VALUES (?1,?2,'sa','x',1.0,?3,'注疏',1)",
            params![
                format!("觀自在菩薩釋{i}"),
                format!("观自在菩萨释{i}"),
                format!("T{:04}", 900 + i)
            ],
        )
        .unwrap();
    }
    let out = run(&conn, &req("觀自在菩薩，涅槃寂靜無為", false)).unwrap();
    let segments = out.segments.unwrap();
    let hit = segments.iter().find(|s| s.matched).unwrap();
    assert_eq!(hit.groups.len(), 3, "per-segment display cap is 3");
    assert_eq!(hit.total, 6, "but the true count is reported");
}

#[test]
fn segment_display_uncapped_with_limit_none() {
    let conn = fixture();
    // Same fixture shape as `segment_display_is_capped_but_total_is_honest`,
    // but with `--all` (`limit: None`): the two truncation layers are
    // independent, and `--all` must lift both — the top-level `groups` cap
    // AND the per-segment `min(--limit, SEGMENT_GROUP_CAP)` cap.
    for i in 0..5 {
        conn.execute(
            "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
             VALUES (?1,?2,'sa','x',1.0,?3,'注疏',1)",
            params![
                format!("觀自在菩薩釋{i}"),
                format!("观自在菩萨释{i}"),
                format!("T{:04}", 900 + i)
            ],
        )
        .unwrap();
    }
    let out = run(
        &conn,
        &req_with_limit("觀自在菩薩，涅槃寂靜無為", false, None),
    )
    .unwrap();
    let segments = out.segments.unwrap();
    let hit = segments.iter().find(|s| s.matched).unwrap();
    assert_eq!(hit.total, 6, "true count unchanged from the capped test");
    assert_eq!(
        hit.groups.len(),
        hit.total,
        "--all lifts the per-segment SEGMENT_GROUP_CAP"
    );
    assert_eq!(
        out.total, 6,
        "top-level total unchanged from the capped test"
    );
    assert_eq!(
        out.groups.len(),
        out.total,
        "--all lifts the top-level --limit cap too"
    );
}
