use std::collections::HashMap;

pub type NormMap = HashMap<char, char>;

/// Whitespace + punctuation removed during normalization.
/// MUST stay byte-for-byte identical to the Python export side.
pub const STRIP_CHARS: &str =
    " \t\r\n\u{3000}，。；：！？、,.;:!?“”‘’\"'()（）《》〈〉【】…—-·";

pub fn normalize(input: &str, map: &NormMap) -> String {
    input
        .chars()
        .filter(|c| !STRIP_CHARS.contains(*c))
        .map(|c| *map.get(&c).unwrap_or(&c))
        .collect()
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
