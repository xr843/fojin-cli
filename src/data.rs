use anyhow::{anyhow, Context, Result};
use rusqlite::{OpenFlags, OptionalExtension};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Connect timeout for the data download: fails fast if the release host is
/// unreachable rather than hanging forever.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Overall read ceiling for the data download. The artifact is ~100-200MB,
/// so this is generous for a slow-but-alive connection while still
/// guaranteeing the CLI can never hang indefinitely.
const READ_TIMEOUT: Duration = Duration::from_secs(900);

pub const EXPECTED_DATA_VERSION: &str = "v1";
pub const EXPECTED_NORM_RULESET: &str = "t2s-char-1to1-v1";

pub struct DataSource<'a> {
    pub url: &'a str,
    pub sha256: &'a str,
}

const MB: u64 = 1024 * 1024;

/// Tracks download progress and yields a message each time a new 10% decile
/// is crossed (at most once per decile). Silent when total size is unknown.
pub struct Progress {
    total: Option<u64>,
    done: u64,
    last_decile: u64,
}

impl Progress {
    pub fn new(total: Option<u64>) -> Self {
        Self {
            total,
            done: 0,
            last_decile: 0,
        }
    }

    pub fn advance(&mut self, bytes: u64) -> Option<String> {
        self.done = self.done.saturating_add(bytes);
        let total = self.total.filter(|&t| t > 0)?;
        let decile = (self.done.saturating_mul(10) / total).min(10);
        if decile <= self.last_decile {
            return None;
        }
        self.last_decile = decile;
        Some(format!(
            "下载中... {}% ({}/{} MB)",
            decile * 10,
            self.done / MB,
            total / MB
        ))
    }
}

pub fn download_notice(total: Option<u64>) -> String {
    match total {
        Some(t) => format!(
            "首次运行:正在下载对齐数据 ({} MB),完成后即可完全离线使用...",
            t / MB
        ),
        None => "首次运行:正在下载对齐数据,完成后即可完全离线使用...".to_string(),
    }
}

pub fn resolve_data_path(data_dir: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(d) = data_dir {
        return Ok(d.join("data.sqlite"));
    }
    let dirs = directories::ProjectDirs::from("app", "fojin", "fojin")
        .ok_or_else(|| anyhow!("无法确定缓存目录"))?;
    Ok(dirs.cache_dir().join("data.sqlite"))
}

pub fn verify_sha256(bytes: &[u8], expected_hex: &str) -> bool {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    let got: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    got.eq_ignore_ascii_case(expected_hex)
}

pub fn gunzip(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut d = flate2::read::GzDecoder::new(bytes);
    let mut out = Vec::new();
    d.read_to_end(&mut out).context("解压 gzip 失败")?;
    Ok(out)
}

/// Write `bytes` to `path` atomically: write a temp sibling then rename.
/// Rename is atomic on the same filesystem, so a crash mid-write can never
/// leave a corrupt file at `path` that a later `ensure_data` run would trust.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).with_context(|| format!("写入临时文件失败: {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("原子替换数据文件失败: {}", path.display()))?;
    Ok(())
}

pub fn ensure_data(path: &Path, offline: bool, source: &DataSource) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    if offline {
        return Err(anyhow!(
            "本地数据不存在且处于 --offline (offline)。请手动下载:\n  {}\n解压后放到: {}",
            source.url,
            path.display()
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("创建缓存目录失败")?;
    }
    let gz = http_get(source.url).map_err(|e| {
        anyhow!(
            "{e:#}\n请手动下载:\n  {}\n解压后放到: {}",
            source.url,
            path.display()
        )
    })?;
    if !verify_sha256(&gz, source.sha256) {
        return Err(anyhow!(
            "下载校验失败(sha256 不符)。请重试或手动下载:\n  {}\n解压后放到: {}",
            source.url,
            path.display()
        ));
    }
    let raw = gunzip(&gz)?;
    write_atomic(path, &raw)?;
    Ok(())
}

fn http_get(url: &str) -> Result<Vec<u8>> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout_read(READ_TIMEOUT)
        .build();
    let resp = agent
        .get(url)
        .call()
        .with_context(|| format!("下载失败: {url}"))?;
    let total: Option<u64> = resp.header("Content-Length").and_then(|v| v.parse().ok());
    eprintln!("{}", download_notice(total));
    let mut progress = Progress::new(total);
    let mut reader = resp.into_reader();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut chunk).context("读取响应失败")?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(msg) = progress.advance(n as u64) {
            eprintln!("{msg}");
        }
    }
    Ok(buf)
}

pub fn open_db(path: &Path) -> Result<rusqlite::Connection> {
    rusqlite::Connection::open(path).with_context(|| format!("打开数据失败: {}", path.display()))
}

pub fn open_read_only_db(path: &Path) -> Result<rusqlite::Connection> {
    rusqlite::Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("打开数据失败: {}", path.display()))
}

#[derive(Debug, serde::Serialize)]
pub struct DatasetCompatibility {
    pub version: String,
    pub norm_ruleset: String,
}

pub fn validate_compatibility(conn: &rusqlite::Connection) -> Result<DatasetCompatibility> {
    require_schema(conn, "meta", "SELECT key, value FROM meta LIMIT 0")?;
    require_schema(
        conn,
        "parallels",
        "SELECT id, zh_text, zh_norm, foreign_lang, foreign_text, confidence, cbeta_id, title_zh, \
         juan_num FROM parallels LIMIT 0",
    )?;
    require_schema(
        conn,
        "parallels_fts",
        "SELECT rowid, zh_norm FROM parallels_fts LIMIT 0",
    )?;
    require_schema(
        conn,
        "norm_map",
        "SELECT from_char, to_char FROM norm_map LIMIT 0",
    )?;

    Ok(DatasetCompatibility {
        version: require_expected_meta(conn, "version", EXPECTED_DATA_VERSION)?,
        norm_ruleset: require_expected_meta(conn, "norm_ruleset", EXPECTED_NORM_RULESET)?,
    })
}

pub fn verify_dataset(conn: &rusqlite::Connection) -> Result<DatasetCompatibility> {
    let compatibility = validate_compatibility(conn)?;
    let mut stmt = conn.prepare("PRAGMA quick_check").map_err(|e| {
        anyhow!(
            "dataset incompatibility: could not run PRAGMA quick_check: {e}. Run `fojin data update`."
        )
    })?;
    let diagnostics = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| {
            anyhow!(
                "dataset incompatibility: could not read PRAGMA quick_check output: {e}. Run `fojin data update`."
            )
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| {
            anyhow!(
                "dataset incompatibility: could not collect PRAGMA quick_check output: {e}. Run `fojin data update`."
            )
        })?;

    if diagnostics.as_slice() == ["ok"] {
        return Ok(compatibility);
    }

    let summary = if diagnostics.is_empty() {
        "no diagnostics returned".to_string()
    } else {
        diagnostics.join("; ")
    };
    Err(anyhow!(
        "dataset incompatibility: PRAGMA quick_check failed: {summary}. Run `fojin data update`."
    ))
}

pub fn open_compatible_db(path: &Path) -> Result<rusqlite::Connection> {
    let conn = open_read_only_db(path)?;
    validate_compatibility(&conn)?;
    Ok(conn)
}

#[derive(Debug, serde::Serialize)]
pub struct DatasetStats {
    pub version: Option<String>,
    pub license: Option<String>,
    pub attribution: Option<String>,
    pub total: u64,
    /// (lang, count) sorted by lang code
    pub by_lang: Vec<(String, u64)>,
    /// distinct cbeta_id count
    pub texts: u64,
}

pub fn dataset_stats(conn: &rusqlite::Connection) -> Result<DatasetStats> {
    let meta_get = |key: &str| -> Result<Option<String>> {
        Ok(conn
            .query_row("SELECT value FROM meta WHERE key=?1", [key], |r| r.get(0))
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?)
    };
    let total: u64 = conn.query_row("SELECT COUNT(*) FROM parallels", [], |r| r.get(0))?;
    let texts: u64 = conn.query_row(
        "SELECT COUNT(DISTINCT cbeta_id) FROM parallels WHERE cbeta_id IS NOT NULL",
        [],
        |r| r.get(0),
    )?;
    let mut stmt = conn.prepare(
        "SELECT foreign_lang, COUNT(*) FROM parallels GROUP BY foreign_lang ORDER BY foreign_lang",
    )?;
    let by_lang = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(DatasetStats {
        version: meta_get("version")?,
        license: meta_get("license")?,
        attribution: meta_get("attribution")?,
        total,
        by_lang,
        texts,
    })
}

fn require_schema(conn: &rusqlite::Connection, name: &str, sql: &str) -> Result<()> {
    conn.prepare(sql).map(|_| ()).map_err(|e| {
        anyhow!(
            "dataset incompatibility: required schema `{name}` is missing or invalid: {e}. Run `fojin data update`."
        )
    })
}

fn require_expected_meta(conn: &rusqlite::Connection, key: &str, expected: &str) -> Result<String> {
    let got = conn
        .query_row("SELECT value FROM meta WHERE key=?1", [key], |row| row.get::<_, String>(0))
        .optional()
        .map_err(|e| {
            anyhow!(
                "dataset incompatibility: could not read meta `{key}`: {e}. Run `fojin data update`."
            )
        })?
        .ok_or_else(|| {
            anyhow!(
                "dataset incompatibility: required meta `{key}` is missing. Run `fojin data update`."
            )
        })?;

    if got != expected {
        return Err(anyhow!(
            "dataset incompatibility: meta `{key}` expected `{expected}` but found `{got}`. Run `fojin data update`."
        ));
    }

    Ok(got)
}
