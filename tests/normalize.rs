use fojin_cli::normalize::{load_norm_map, normalize, NormMap};
use rusqlite::Connection;
use std::fs;

fn load_fixture_map() -> NormMap {
    let mut m = NormMap::new();
    for line in fs::read_to_string("tests/fixtures/norm_map.tsv")
        .unwrap()
        .lines()
    {
        let (a, b) = line.split_once('\t').unwrap();
        m.insert(a.chars().next().unwrap(), b.chars().next().unwrap());
    }
    m
}

#[test]
fn golden_norm_cases() {
    let map = load_fixture_map();
    for line in fs::read_to_string("tests/fixtures/norm_cases.tsv")
        .unwrap()
        .lines()
    {
        let (inp, exp) = line.split_once('\t').unwrap();
        assert_eq!(normalize(inp, &map), exp, "input was {inp:?}");
    }
}

#[test]
fn load_norm_map_reads_rows() {
    let conn = Connection::open_in_memory().unwrap();
    fojin_cli::schema::init_schema(&conn).unwrap();
    conn.execute(
        "INSERT INTO norm_map(from_char,to_char) VALUES ('應','应')",
        [],
    )
    .unwrap();
    let m = load_norm_map(&conn).unwrap();
    assert_eq!(m.get(&'應'), Some(&'应'));
}
