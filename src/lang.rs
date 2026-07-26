use anyhow::{bail, Result};

/// Language codes the renderer knows how to label. Validation uses this static
/// list rather than `SELECT DISTINCT foreign_lang`, which would full-scan
/// 908k rows (the column has no index).
pub const KNOWN_LANGS: [&str; 6] = ["sa", "pi", "bo", "en", "lzh", "zh"];

pub fn validate_langs(codes: &[String]) -> Result<()> {
    for code in codes {
        if !KNOWN_LANGS.contains(&code.as_str()) {
            bail!("未知语种 `{code}`;可用: {}", KNOWN_LANGS.join(", "));
        }
    }
    Ok(())
}
