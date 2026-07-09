pub const SCHEMA_SQL: &str = include_str!("../schema.sql");

pub fn init_schema(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA_SQL)
}
