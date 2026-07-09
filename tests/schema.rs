use rusqlite::Connection;

#[test]
fn schema_creates_tables_and_fts_autopopulates() {
    let conn = Connection::open_in_memory().unwrap();
    fojin_cli::schema::init_schema(&conn).unwrap();

    conn.execute(
        "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
         VALUES ('色即是空','色即是空','sa','rūpaṃ śūnyatā',0.9,'T0251','心經',1)",
        [],
    ).unwrap();

    // FTS row auto-inserted by trigger; trigram MATCH finds the substring
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM parallels_fts WHERE parallels_fts MATCH '\"色即是空\"'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);

    // trigram-specific: a 3-char fragment from the MIDDLE of the stored value
    // must match. Under the default unicode61 tokenizer the CJK run is a single
    // token and this fragment query would NOT match — so this proves trigram.
    let sub: i64 = conn
        .query_row(
            "SELECT count(*) FROM parallels_fts WHERE parallels_fts MATCH '\"即是空\"'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        sub, 1,
        "trigram substring '即是空' must match stored '色即是空'"
    );
}
