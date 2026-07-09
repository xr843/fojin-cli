use fojin_cli::query::search;
use fojin_cli::schema::init_schema;
use rusqlite::{params, Connection};

fn fixture() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    init_schema(&conn).unwrap();
    let rows = [
        ("色即是空", "色即是空", "sa", "rūpaṃ śūnyatā", 0.91, "T0251", "心經", 1),
        ("色即是空", "色即是空", "bo", "gzugs stong pa", 0.88, "T0251", "心經", 1),
        ("受想行識", "受想行识", "sa", "vedanā saṃjñā", 0.70, "T0251", "心經", 1),
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
    assert_eq!(sa[1].text, "sa-mid", "second highest; lowest (sa-lo) dropped");
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
