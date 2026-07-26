//! Byte-for-byte guard on the hit path. The search-capability work (reverse
//! lookup, sentence splitting, fallback) must only engage on ZERO hits; a
//! query that matches must render exactly as it did before that work landed.
use fojin_cli::cli::compute_output;
use fojin_cli::schema::init_schema;
use rusqlite::{params, Connection};

fn fixture() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    init_schema(&conn).unwrap();
    conn.execute(
        "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
         VALUES ('色即是空','色即是空','sa','rūpaṃ śūnyatā',0.91,'T0251','心經',1)",
        params![],
    )
    .unwrap();
    conn
}

#[test]
fn hit_path_human_output_is_byte_identical() {
    let conn = fixture();
    let out = compute_output(&conn, "色即是空", None, 3, None, false).unwrap();
    assert_eq!(
        out,
        "汉  色即是空  (《心經》T0251 卷1)\n\
         梵  rūpaṃ śūnyatā  [MITRA 0.91]\n\
         藏  (无对齐)\n\
         \n\
         完整上下文见 https://fojin.app  ·  数据 CC BY-SA(Dharmamitra + fojin)\n"
    );
}
