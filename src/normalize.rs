use anyhow::{bail, Result};
use std::collections::HashMap;

pub type NormMap = HashMap<char, char>;

/// Whitespace + punctuation removed during normalization.
/// MUST stay byte-for-byte identical to the Python export side.
pub const STRIP_CHARS: &str = " \t\r\n\u{3000}，。；：！？、,.;:!?“”‘’\"'()（）《》〈〉【】…—-·";
pub const MIN_QUERY_CHARS: usize = 2;

pub fn normalize(input: &str, map: &NormMap) -> String {
    input
        .chars()
        .filter(|c| !STRIP_CHARS.contains(*c))
        .map(|c| *map.get(&c).unwrap_or(&c))
        .collect()
}

pub fn validate_query_length(normalized: &str) -> Result<()> {
    let count = normalized.chars().count();
    if count == 1 {
        bail!("查询至少需要 2 个汉字；一字查询范围过大，请输入更具体的词句");
    }
    Ok(())
}

pub fn load_norm_map(conn: &rusqlite::Connection) -> rusqlite::Result<NormMap> {
    let mut stmt = conn.prepare("SELECT from_char, to_char FROM norm_map")?;
    let rows = stmt.query_map([], |r| {
        let from: String = r.get(0)?;
        let to: String = r.get(1)?;
        Ok((from, to))
    })?;
    let mut map = NormMap::new();
    for row in rows {
        let (from, to) = row?;
        if let (Some(f), Some(t)) = (from.chars().next(), to.chars().next()) {
            map.insert(f, t);
        }
    }
    Ok(map)
}
