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
