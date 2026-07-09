use fojin_cli::cli::compute_output;
use fojin_cli::schema::init_schema;
use rusqlite::{params, Connection};

fn fixture() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    init_schema(&conn).unwrap();
    conn.execute(
        "INSERT INTO norm_map(from_char,to_char) VALUES ('應','应')",
        [],
    )
    .unwrap();
    // Also '無'->'无': the traditional query used in compute_applies_normalization
    // ('應無所住') differs from its stored zh_norm ('应无所住') in two characters,
    // not just '應'. Without this row, normalize() only folds '應', producing
    // "应無所住", which is not a substring of the stored zh_norm under FTS5's
    // trigram phrase MATCH — so the intended normalization-match test would fail
    // for a fixture-data reason unrelated to compute_output's own correctness.
    conn.execute(
        "INSERT INTO norm_map(from_char,to_char) VALUES ('無','无')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
         VALUES ('色即是空','色即是空','sa','rūpaṃ śūnyatā',0.91,'T0251','心經',1)",
        params![],
    ).unwrap();
    conn
}

#[test]
fn compute_human_output_matches() {
    let conn = fixture();
    let out = compute_output(&conn, "  色即是空  ", None, 3, None, false).unwrap();
    assert!(out.contains("梵  rūpaṃ śūnyatā  [MITRA 0.91]"));
    assert!(out.contains("巴  (无对齐)"));
}

#[test]
fn compute_json_output_matches() {
    let conn = fixture();
    let out = compute_output(&conn, "色即是空", None, 3, None, true).unwrap();
    assert!(out.contains("\"matched\": true"));
}

#[test]
fn compute_applies_normalization() {
    // '應' folds to '应' via norm_map; the stored zh_norm has '应'
    let conn = fixture();
    conn.execute(
        "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
         VALUES ('應無所住','应无所住','sa','apratiṣṭhita',0.8,'T0235','金剛經',1)",
        params![],
    ).unwrap();
    let out = compute_output(&conn, "應無所住", None, 3, None, false).unwrap();
    assert!(
        out.contains("apratiṣṭhita"),
        "traditional query should match folded zh_norm"
    );
}

#[test]
fn compute_limit_caps_groups_and_reports_hidden() {
    let conn = fixture();
    // Insert 3 distinct match-groups: same zh_norm substring ("色即是空"), but
    // different (zh_text, cbeta_id, juan_num) so query::search groups them
    // separately. The base fixture() already inserts one group; add two more.
    conn.execute(
        "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
         VALUES ('色即是空義','色即是空义','sa','rupam2',0.5,'T0252','心經略疏',1)",
        params![],
    ).unwrap();
    conn.execute(
        "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
         VALUES ('色即是空論','色即是空论','sa','rupam3',0.4,'T0253','心經釋論',1)",
        params![],
    ).unwrap();

    let out = compute_output(&conn, "色即是空", None, 3, Some(2), false).unwrap();
    assert_eq!(
        out.matches("汉  ").count(),
        2,
        "must show exactly 2 groups' 汉 lines"
    );
    assert!(out.contains("还有 1 组"));
    assert!(out.contains("--all"));
}
