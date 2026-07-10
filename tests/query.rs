use fojin_cli::query::search;
use fojin_cli::schema::init_schema;
use rusqlite::{params, Connection};

fn fixture() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    init_schema(&conn).unwrap();
    let rows = [
        (
            "色即是空",
            "色即是空",
            "sa",
            "rūpaṃ śūnyatā",
            0.91,
            "T0251",
            "心經",
            1,
        ),
        (
            "色即是空",
            "色即是空",
            "bo",
            "gzugs stong pa",
            0.88,
            "T0251",
            "心經",
            1,
        ),
        (
            "受想行識",
            "受想行识",
            "sa",
            "vedanā saṃjñā",
            0.70,
            "T0251",
            "心經",
            1,
        ),
    ];
    for (zt, zn, lang, f, c, cb, ti, j) in rows {
        conn.execute(
            "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![zt, zn, lang, f, c, cb, ti, j],
        ).unwrap();
    }
    conn
}

#[test]
fn exact_match_groups_two_langs() {
    let conn = fixture();
    let g = search(&conn, "色即是空", None, 3).unwrap();
    assert_eq!(g.len(), 1);
    assert_eq!(g[0].zh_text, "色即是空");
    assert_eq!(g[0].parallels.len(), 2);
}

#[test]
fn lang_filter_keeps_only_requested() {
    let conn = fixture();
    let langs = vec!["sa".to_string()];
    let g = search(&conn, "色即是空", Some(&langs), 3).unwrap();
    assert_eq!(g[0].parallels.len(), 1);
    assert_eq!(g[0].parallels[0].lang, "sa");
}

#[test]
fn no_match_is_empty_not_error() {
    let conn = fixture();
    let g = search(&conn, "涅槃寂静", None, 3).unwrap();
    assert!(g.is_empty());
}

#[test]
fn short_query_uses_like_fallback() {
    let conn = fixture();
    let g = search(&conn, "色即", None, 3).unwrap(); // 2 chars < 3
    assert_eq!(g.len(), 1);
    assert_eq!(g[0].zh_text, "色即是空");
}

#[test]
fn per_lang_cap_and_order() {
    let conn = Connection::open_in_memory().unwrap();
    init_schema(&conn).unwrap();
    // one group, three Sanskrit parallels with distinct confidence
    for (f, c) in [("sa-lo", 0.50_f64), ("sa-hi", 0.95), ("sa-mid", 0.70)] {
        conn.execute(
            "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
             VALUES ('色即是空','色即是空','sa',?1,?2,'T0251','心經',1)",
            params![f, c],
        ).unwrap();
    }
    let g = search(&conn, "色即是空", None, 2).unwrap();
    assert_eq!(g.len(), 1);
    let sa: Vec<_> = g[0].parallels.iter().filter(|p| p.lang == "sa").collect();
    assert_eq!(sa.len(), 2, "capped to top=2 per language");
    assert_eq!(sa[0].text, "sa-hi", "highest confidence first");
    assert_eq!(
        sa[1].text, "sa-mid",
        "second highest; lowest (sa-lo) dropped"
    );
}

#[test]
fn top_zero_floors_to_one() {
    let conn = fixture();
    let g = search(&conn, "色即是空", None, 0).unwrap();
    let sa = g[0].parallels.iter().filter(|p| p.lang == "sa").count();
    assert_eq!(sa, 1, "top=0 floors to 1 per language");
}

#[test]
fn empty_query_returns_empty() {
    let conn = fixture();
    assert!(search(&conn, "", None, 3).unwrap().is_empty());
}

#[test]
fn exact_sentence_outranks_longer_higher_confidence_match() {
    let conn = Connection::open_in_memory().unwrap();
    init_schema(&conn).unwrap();
    for (zt, zn, lang, f, c, cb, ti, j) in [
        (
            "舍利子色不异空色即是空空即是色",
            "舍利子色不异空色即是空空即是色",
            "sa",
            "long-high",
            1.0,
            "T0002",
            "長句",
            1,
        ),
        (
            "色即是空",
            "色即是空",
            "sa",
            "exact-lower",
            0.8,
            "T0001",
            "短句",
            1,
        ),
    ] {
        conn.execute(
            "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![zt, zn, lang, f, c, cb, ti, j],
        )
        .unwrap();
    }

    let groups = search(&conn, "色即是空", None, 3).unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].zh_text, "色即是空");
}

#[test]
fn shorter_containing_sentence_outranks_longer_peer() {
    let conn = Connection::open_in_memory().unwrap();
    init_schema(&conn).unwrap();
    for (zt, zn, lang, f, c, cb, ti, j) in [
        (
            "觀自在菩薩行深般若波羅蜜多時照見五蘊皆空色即是空",
            "观自在菩萨行深般若波罗蜜多时照见五蕴皆空色即是空",
            "sa",
            "long-peer",
            1.0,
            "T0002",
            "長句",
            1,
        ),
        (
            "五蘊皆空色即是空",
            "五蕴皆空色即是空",
            "sa",
            "short-peer",
            1.0,
            "T0003",
            "短句",
            1,
        ),
    ] {
        conn.execute(
            "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![zt, zn, lang, f, c, cb, ti, j],
        )
        .unwrap();
    }

    let groups = search(&conn, "色即是空", None, 3).unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].zh_text, "五蘊皆空色即是空");
}

#[test]
fn relevance_ties_use_stable_source_order() {
    let conn = Connection::open_in_memory().unwrap();
    init_schema(&conn).unwrap();
    for (zt, zn, lang, f, cb) in [
        (
            "甲色即是空乙",
            "甲色即是空乙",
            "sa",
            "first-inserted",
            "T0002",
        ),
        (
            "丙色即是空丁",
            "丙色即是空丁",
            "sa",
            "second-inserted",
            "T0001",
        ),
    ] {
        conn.execute(
            "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
             VALUES (?1,?2,?3,?4,1.0,?5,'同分',1)",
            params![zt, zn, lang, f, cb],
        )
        .unwrap();
    }

    let groups = search(&conn, "色即是空", None, 3).unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].cbeta_id.as_deref(), Some("T0001"));
}

#[test]
fn relevance_ties_order_by_juan_then_zh_text() {
    let conn = Connection::open_in_memory().unwrap();
    init_schema(&conn).unwrap();
    for (zt, j) in [("色即是空C", 2), ("色即是空B", 1), ("色即是空A", 1)] {
        conn.execute(
            "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
             VALUES (?1,?1,'sa',?1,1.0,'T0001','同分',?2)",
            params![zt, j],
        )
        .unwrap();
    }

    let groups = search(&conn, "色即是空", None, 3).unwrap();
    let order: Vec<_> = groups
        .iter()
        .map(|group| (group.juan_num, group.zh_text.as_str()))
        .collect();
    assert_eq!(
        order,
        [
            (Some(1), "色即是空A"),
            (Some(1), "色即是空B"),
            (Some(2), "色即是空C"),
        ]
    );
}

fn cite_fixture() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    init_schema(&conn).unwrap();
    let rows = [
        (
            "觀自在菩薩",
            "观自在菩萨",
            "sa",
            "āryāvalokiteśvaro",
            0.95,
            "T0251",
            "心經",
            1,
        ),
        (
            "色即是空",
            "色即是空",
            "sa",
            "rūpaṃ śūnyatā",
            0.91,
            "T0251",
            "心經",
            2,
        ),
        (
            "色即是空",
            "色即是空",
            "bo",
            "gzugs stong pa",
            0.88,
            "T0251",
            "心經",
            2,
        ),
        (
            "如是我聞",
            "如是我闻",
            "sa",
            "evaṃ mayā śrutam",
            0.90,
            "T0235",
            "金剛經",
            1,
        ),
    ];
    for (zt, zn, lang, f, c, cb, ti, j) in rows {
        conn.execute(
            "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![zt, zn, lang, f, c, cb, ti, j],
        ).unwrap();
    }
    for (from, to) in [("經", "经"), ("剛", "刚")] {
        conn.execute(
            "INSERT INTO norm_map(from_char,to_char) VALUES (?1,?2)",
            params![from, to],
        )
        .unwrap();
    }
    conn
}

#[test]
fn by_cbeta_lists_only_that_text_in_juan_order() {
    let conn = cite_fixture();
    let g = fojin_cli::query::by_cbeta(&conn, "T0251", None, None, 3).unwrap();
    assert_eq!(g.len(), 2);
    assert_eq!(g[0].zh_text, "觀自在菩薩");
    assert_eq!(g[0].juan_num, Some(1));
    assert_eq!(g[1].zh_text, "色即是空");
    assert_eq!(g[1].parallels.len(), 2, "sa+bo grouped");
}

#[test]
fn by_cbeta_filters_juan_and_is_case_insensitive() {
    let conn = cite_fixture();
    let g = fojin_cli::query::by_cbeta(&conn, "t0251", Some(2), None, 3).unwrap();
    assert_eq!(g.len(), 1);
    assert_eq!(g[0].juan_num, Some(2));
}

#[test]
fn by_cbeta_unknown_id_is_empty_not_error() {
    let conn = cite_fixture();
    assert!(fojin_cli::query::by_cbeta(&conn, "T9999", None, None, 3)
        .unwrap()
        .is_empty());
}

#[test]
fn texts_matching_folds_traditional_titles() {
    let conn = cite_fixture();
    let map = fojin_cli::normalize::load_norm_map(&conn).unwrap();
    let hits = fojin_cli::query::texts_matching(&conn, "心经", &map).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].cbeta_id, "T0251");
    assert_eq!(
        hits[0].title_zh, "心經",
        "displays original traditional title"
    );
}

#[test]
fn texts_matching_counts_per_lang() {
    let conn = cite_fixture();
    let map = fojin_cli::normalize::load_norm_map(&conn).unwrap();
    let hits = fojin_cli::query::texts_matching(&conn, "心经", &map).unwrap();
    let by_lang = &hits[0].by_lang;
    assert_eq!(by_lang.iter().find(|(l, _)| l == "sa").unwrap().1, 2);
    assert_eq!(by_lang.iter().find(|(l, _)| l == "bo").unwrap().1, 1);
}

#[test]
fn texts_matching_no_hit_is_empty() {
    let conn = cite_fixture();
    let map = fojin_cli::normalize::load_norm_map(&conn).unwrap();
    assert!(fojin_cli::query::texts_matching(&conn, "法华", &map)
        .unwrap()
        .is_empty());
}
