use anyhow::{anyhow, Context, Result};
use rusqlite::{OpenFlags, OptionalExtension};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Connect timeout for the data download: fails fast if the release host is
/// unreachable rather than hanging forever.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Overall read ceiling for the data download. The artifact is ~100-200MB,
/// so this is generous for a slow-but-alive connection while still
/// guaranteeing the CLI can never hang indefinitely.
const READ_TIMEOUT: Duration = Duration::from_secs(900);
static CANDIDATE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
    let raw = download_and_unpack(path, source)?;
    write_atomic(path, &raw)
}

/// Download a replacement dataset to a sibling candidate, validate it, then
/// replace the live dataset. Failures before replacement leave `path` intact.
pub fn update_data(path: &Path, source: &DataSource) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("创建缓存目录失败")?;
    }

    let raw = download_and_unpack(path, source)?;
    let (candidate, mut candidate_file) = create_candidate(path)?;
    let write_result = candidate_file
        .write_all(&raw)
        .with_context(|| format!("写入候选数据失败: {}", candidate.display()));
    drop(candidate_file);
    if let Err(error) = write_result {
        return Err(cleanup_candidate_error(&candidate, error));
    }

    let validation_result = verify_dataset_file(&candidate).map(|_| ());
    if let Err(error) = validation_result {
        return Err(cleanup_candidate_error(&candidate, error));
    }

    finish_replacement(&candidate, replace_with_candidate(path, &candidate))
}

fn download_and_unpack(path: &Path, source: &DataSource) -> Result<Vec<u8>> {
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
    gunzip(&gz)
}

fn sibling_path(path: &Path, suffix: &str) -> Result<PathBuf> {
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow!("数据路径没有文件名: {}", path.display()))?;
    let mut sibling = file_name.to_os_string();
    sibling.push(suffix);
    Ok(path.with_file_name(sibling))
}

fn create_candidate(path: &Path) -> Result<(PathBuf, std::fs::File)> {
    loop {
        let sequence = CANDIDATE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let suffix = format!(".candidate.{}.{sequence}", std::process::id());
        let candidate = sibling_path(path, &suffix)?;
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(ref error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("创建候选数据失败: {}", candidate.display()))
            }
        }
    }
}

fn cleanup_candidate_error(candidate: &Path, error: anyhow::Error) -> anyhow::Error {
    match remove_candidate_artifacts(candidate) {
        Ok(()) => error,
        Err(cleanup_error) => error.context(format!("清理候选数据失败: {cleanup_error}")),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CandidateCleanupPolicy {
    Remove,
    #[cfg(any(test, windows))]
    Preserve,
}

#[derive(Debug)]
struct ReplacementFailure {
    error: anyhow::Error,
    cleanup: CandidateCleanupPolicy,
}

impl ReplacementFailure {
    fn remove(error: anyhow::Error) -> Self {
        Self {
            error,
            cleanup: CandidateCleanupPolicy::Remove,
        }
    }

    #[cfg(any(test, windows))]
    fn preserve(error: anyhow::Error) -> Self {
        Self {
            error,
            cleanup: CandidateCleanupPolicy::Preserve,
        }
    }
}

type ReplacementResult = std::result::Result<(), ReplacementFailure>;

fn finish_replacement(candidate: &Path, result: ReplacementResult) -> Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(failure) => match failure.cleanup {
            CandidateCleanupPolicy::Remove => {
                Err(cleanup_candidate_error(candidate, failure.error))
            }
            #[cfg(any(test, windows))]
            CandidateCleanupPolicy::Preserve => Err(failure.error.context(format!(
                "validated candidate preserved at `{}`",
                candidate.display()
            ))),
        },
    }
}

fn remove_candidate_artifacts(candidate: &Path) -> Result<()> {
    for suffix in ["", "-journal", "-shm", "-wal"] {
        let artifact = if suffix.is_empty() {
            candidate.to_path_buf()
        } else {
            sibling_path(candidate, suffix)?
        };
        match std::fs::remove_file(&artifact) {
            Ok(()) => {}
            Err(ref error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("删除候选数据失败: {}", artifact.display()))
            }
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_with_candidate(path: &Path, candidate: &Path) -> ReplacementResult {
    // The candidate is a sibling, so rename atomically replaces the live path
    // on Unix without first removing or moving it away.
    std::fs::rename(candidate, path)
        .with_context(|| {
            format!(
                "替换数据文件失败: {} -> {}",
                candidate.display(),
                path.display()
            )
        })
        .map_err(ReplacementFailure::remove)
}

#[cfg(windows)]
fn replace_with_candidate(path: &Path, candidate: &Path) -> ReplacementResult {
    match std::fs::metadata(path) {
        Ok(_) => {}
        Err(ref error) if error.kind() == std::io::ErrorKind::NotFound => {
            return std::fs::rename(candidate, path)
                .with_context(|| {
                    format!(
                        "替换数据文件失败: {} -> {}",
                        candidate.display(),
                        path.display()
                    )
                })
                .map_err(ReplacementFailure::remove)
        }
        Err(error) => {
            return Err(ReplacementFailure::remove(
                anyhow::Error::new(error)
                    .context(format!("检查现有数据文件失败: {}", path.display())),
            ))
        }
    }

    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    #[link(name = "Kernel32")]
    extern "system" {
        fn ReplaceFileW(
            replaced_file_name: *const u16,
            replacement_file_name: *const u16,
            backup_file_name: *const u16,
            replace_flags: u32,
            exclude: *mut c_void,
            reserved: *mut c_void,
        ) -> i32;
    }

    fn wide_path(path: &Path) -> Result<Vec<u16>> {
        let mut encoded: Vec<u16> = path.as_os_str().encode_wide().collect();
        if encoded.contains(&0) {
            return Err(anyhow!("数据路径包含空字符: {}", path.display()));
        }
        encoded.push(0);
        Ok(encoded)
    }

    let replaced = wide_path(path).map_err(ReplacementFailure::remove)?;
    let replacement = wide_path(candidate).map_err(ReplacementFailure::remove)?;
    // SAFETY: both path buffers are NUL-terminated UTF-16 and remain alive for
    // the call; optional and reserved pointers are null as ReplaceFileW requires.
    let replaced_ok = unsafe {
        ReplaceFileW(
            replaced.as_ptr(),
            replacement.as_ptr(),
            ptr::null(),
            0,
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    if replaced_ok != 0 {
        return Ok(());
    }

    let native_error = std::io::Error::last_os_error();
    handle_windows_replace_failure(path, candidate, native_error, std::fs::rename)
}

#[cfg(windows)]
fn handle_windows_replace_failure<F>(
    path: &Path,
    candidate: &Path,
    native_error: std::io::Error,
    recover: F,
) -> ReplacementResult
where
    F: FnOnce(&Path, &Path) -> std::io::Result<()>,
{
    const ERROR_UNABLE_TO_REMOVE_REPLACED: i32 = 1175;
    const ERROR_UNABLE_TO_MOVE_REPLACEMENT: i32 = 1176;
    const ERROR_UNABLE_TO_MOVE_REPLACEMENT_2: i32 = 1177;

    match native_error.raw_os_error() {
        Some(ERROR_UNABLE_TO_MOVE_REPLACEMENT | ERROR_UNABLE_TO_MOVE_REPLACEMENT_2) => {
            match recover(candidate, path) {
                Ok(()) => Ok(()),
                Err(recovery_error) => Err(ReplacementFailure::preserve(anyhow!(
                        "ReplaceFileW failed for replaced `{}` and replacement `{}` with {}; recovery rename `{} -> {}` failed with {}",
                        path.display(),
                        candidate.display(),
                        describe_windows_error(&native_error),
                        candidate.display(),
                        path.display(),
                        describe_windows_error(&recovery_error)
                    ))),
            }
        }
        Some(ERROR_UNABLE_TO_REMOVE_REPLACED) => Err(ReplacementFailure::remove(
            windows_replace_error(path, candidate, &native_error),
        )),
        _ => Err(ReplacementFailure::remove(windows_replace_error(
            path,
            candidate,
            &native_error,
        ))),
    }
}

#[cfg(windows)]
fn windows_replace_error(
    path: &Path,
    candidate: &Path,
    native_error: &std::io::Error,
) -> anyhow::Error {
    anyhow!(
        "ReplaceFileW failed for replaced `{}` and replacement `{}` with {}",
        path.display(),
        candidate.display(),
        describe_windows_error(native_error)
    )
}

#[cfg(windows)]
fn describe_windows_error(error: &std::io::Error) -> String {
    match error.raw_os_error() {
        Some(code) => format!("Windows error {code}: {error}"),
        None => error.to_string(),
    }
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
    require_trigram_fts(conn)?;
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

pub fn verify_dataset_file(path: &Path) -> Result<DatasetCompatibility> {
    let compatibility = {
        let conn = open_read_only_db(path)?;
        verify_dataset(&conn)?
    };
    let conn = rusqlite::Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|error| {
            anyhow!(
                "dataset incompatibility: could not open existing dataset for FTS5 integrity-check: {error}. Run `fojin data update`."
            )
        })?;
    verify_fts_content_integrity(&conn)?;
    Ok(compatibility)
}

fn verify_fts_content_integrity(conn: &rusqlite::Connection) -> Result<()> {
    const FTS_CONTENT_INTEGRITY_CHECK: &str =
        "INSERT INTO parallels_fts(parallels_fts, rank) VALUES('integrity-check', 1)";
    conn.execute(FTS_CONTENT_INTEGRITY_CHECK, [])
        .map(|_| ())
        .map_err(|error| {
            anyhow!(
                "dataset incompatibility: FTS5 integrity-check failed: {error}. Run `fojin data update`."
            )
        })
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

fn require_trigram_fts(conn: &rusqlite::Connection) -> Result<()> {
    let sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'parallels_fts'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| {
            anyhow!(
                "dataset incompatibility: required schema `parallels_fts` has no table declaration: {e}. Run `fojin data update`."
            )
        })?;
    let declaration: String = sql
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    const EXPECTED_DECLARATION: &str = "createvirtualtableparallels_ftsusingfts5(zh_norm,content='parallels',content_rowid='id',tokenize='trigram')";
    if declaration != EXPECTED_DECLARATION {
        return Err(anyhow!(
            "dataset incompatibility: required schema `parallels_fts` does not match the required FTS5 trigram declaration. Run `fojin data update`."
        ));
    }

    let _: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM parallels_fts WHERE parallels_fts MATCH ?1 LIMIT 1",
            ["x"],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| {
            anyhow!(
                "dataset incompatibility: required schema `parallels_fts` does not support FTS5 MATCH: {e}. Run `fojin data update`."
            )
        })?;
    Ok(())
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

#[cfg(test)]
mod replacement_cleanup_tests {
    use super::*;

    #[test]
    fn preserve_policy_keeps_validated_candidate_and_reports_path() {
        let dir = tempfile::tempdir().unwrap();
        let candidate = dir.path().join("data.sqlite.candidate.1.1");
        std::fs::write(&candidate, b"validated dataset").unwrap();

        let error = finish_replacement(
            &candidate,
            Err(ReplacementFailure::preserve(anyhow!(
                "replacement recovery failed"
            ))),
        )
        .unwrap_err();

        assert_eq!(std::fs::read(&candidate).unwrap(), b"validated dataset");
        let detail = format!("{error:#}");
        assert!(detail.contains("validated candidate preserved"), "{detail}");
        assert!(
            detail.contains(candidate.to_string_lossy().as_ref()),
            "{detail}"
        );
        assert!(detail.contains("replacement recovery failed"), "{detail}");
    }
}

#[cfg(all(test, windows))]
mod windows_replace_failure_tests {
    use super::*;
    use std::cell::Cell;
    use std::io;

    const LIVE: &str = r"C:\cache\data.sqlite";
    const CANDIDATE: &str = r"C:\cache\data.sqlite.candidate.1.1";

    #[test]
    fn error_1175_returns_native_failure_without_recovery() {
        assert_names_untouched_error_skips_recovery(1175);
    }

    #[test]
    fn other_error_returns_native_failure_without_recovery() {
        assert_names_untouched_error_skips_recovery(87);
    }

    #[test]
    fn error_1176_recovers_missing_live_path() {
        assert_partial_failure_recovers(1176);
    }

    #[test]
    fn error_1177_recovers_missing_live_path() {
        assert_partial_failure_recovers(1177);
    }

    #[test]
    fn error_1176_preserves_native_and_recovery_failures() {
        assert_partial_failure_preserves_both_errors(1176);
    }

    #[test]
    fn error_1177_preserves_native_and_recovery_failures() {
        assert_partial_failure_preserves_both_errors(1177);
    }

    fn assert_names_untouched_error_skips_recovery(code: i32) {
        let recovery_called = Cell::new(false);
        let failure = handle_windows_replace_failure(
            Path::new(LIVE),
            Path::new(CANDIDATE),
            io::Error::from_raw_os_error(code),
            |_, _| {
                recovery_called.set(true);
                Ok(())
            },
        )
        .unwrap_err();

        assert!(!recovery_called.get());
        assert_eq!(failure.cleanup, CandidateCleanupPolicy::Remove);
        let detail = format!("{:#}", failure.error);
        assert!(
            detail.contains(&format!("Windows error {code}")),
            "{detail}"
        );
    }

    fn assert_partial_failure_recovers(code: i32) {
        let recovery_called = Cell::new(false);
        handle_windows_replace_failure(
            Path::new(LIVE),
            Path::new(CANDIDATE),
            io::Error::from_raw_os_error(code),
            |from, to| {
                recovery_called.set(true);
                assert_eq!(from, Path::new(CANDIDATE));
                assert_eq!(to, Path::new(LIVE));
                Ok(())
            },
        )
        .unwrap();

        assert!(recovery_called.get());
    }

    fn assert_partial_failure_preserves_both_errors(code: i32) {
        let failure = handle_windows_replace_failure(
            Path::new(LIVE),
            Path::new(CANDIDATE),
            io::Error::from_raw_os_error(code),
            |_, _| Err(io::Error::from_raw_os_error(5)),
        )
        .unwrap_err();

        assert_eq!(failure.cleanup, CandidateCleanupPolicy::Preserve);
        let detail = format!("{:#}", failure.error);
        assert!(
            detail.contains(&format!("Windows error {code}")),
            "{detail}"
        );
        assert!(detail.contains("Windows error 5"), "{detail}");
        assert!(detail.contains(CANDIDATE), "{detail}");
        assert!(detail.contains(LIVE), "{detail}");
    }
}
