use anyhow::{anyhow, Context, Result};
use rusqlite::{OpenFlags, OptionalExtension};
use std::io::Read;
use std::path::{Path, PathBuf};

mod operation_lock;
mod transfer;

#[cfg(windows)]
static REPLACEMENT_BACKUP_SEQUENCE: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

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
    let _lock = operation_lock::acquire(path, operation_lock::LOCK_WAIT_TIMEOUT)?;
    if path.exists() {
        return Ok(());
    }
    install_candidate(path, source)
}

/// Download a replacement dataset to a sibling candidate, validate it, then
/// replace the live dataset. Failures before replacement leave `path` intact.
pub fn update_data(path: &Path, source: &DataSource) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("创建缓存目录失败")?;
    }

    let _lock = operation_lock::acquire(path, operation_lock::LOCK_WAIT_TIMEOUT)?;
    install_candidate(path, source)
}

pub fn clean_data(path: &Path) -> Result<Option<u64>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("创建缓存目录失败")?;
    }
    let _lock = operation_lock::acquire(path, operation_lock::LOCK_WAIT_TIMEOUT)?;
    transfer::remove_known_artifacts(path)?;
    let size = match std::fs::metadata(path) {
        Ok(metadata) => Some(metadata.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).context("读取数据文件状态失败"),
    };
    if size.is_some() {
        std::fs::remove_file(path).with_context(|| format!("删除数据失败: {}", path.display()))?;
    }
    Ok(size)
}

fn install_candidate(path: &Path, source: &DataSource<'_>) -> Result<()> {
    let candidate = transfer::stage_candidate(path, source, transfer::PRODUCTION_POLICY)?;
    if let Err(error) = verify_dataset_file(candidate.path()).map(|_| ()) {
        return Err(candidate.cleanup_with(error));
    }
    if let Err(error) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(candidate.path())
        .and_then(|file| file.sync_all())
        .with_context(|| format!("同步候选数据失败: {}", candidate.path().display()))
    {
        return Err(candidate.cleanup_with(error));
    }
    let candidate_path = candidate.path().to_path_buf();
    finish_replacement(candidate, replace_with_candidate(path, &candidate_path))
}

fn sibling_path(path: &Path, suffix: &str) -> Result<PathBuf> {
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow!("数据路径没有文件名: {}", path.display()))?;
    let mut sibling = file_name.to_os_string();
    sibling.push(suffix);
    Ok(path.with_file_name(sibling))
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

fn finish_replacement(
    candidate: transfer::StagedCandidate,
    result: ReplacementResult,
) -> Result<()> {
    match result {
        Ok(()) => {
            candidate.publish_succeeded();
            Ok(())
        }
        Err(failure) => match failure.cleanup {
            CandidateCleanupPolicy::Remove => Err(candidate.cleanup_with(failure.error)),
            #[cfg(any(test, windows))]
            CandidateCleanupPolicy::Preserve => {
                let candidate_path = candidate.path().to_path_buf();
                candidate.preserve();
                Err(failure.error.context(format!(
                    "validated candidate preserved at `{}`",
                    candidate_path.display()
                )))
            }
        },
    }
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

    let mut backup = reserve_replacement_backup(path).map_err(ReplacementFailure::remove)?;
    let replaced = wide_path(path).map_err(ReplacementFailure::remove)?;
    let replacement = wide_path(candidate).map_err(ReplacementFailure::remove)?;
    let backup_name = wide_path(backup.path()).map_err(ReplacementFailure::remove)?;
    backup
        .prepare_for_replace()
        .map_err(ReplacementFailure::remove)?;
    // SAFETY: all path buffers are NUL-terminated UTF-16 and remain alive for
    // the call; the reserved pointers are null as ReplaceFileW requires.
    let replaced_ok = unsafe {
        ReplaceFileW(
            replaced.as_ptr(),
            replacement.as_ptr(),
            backup_name.as_ptr(),
            0,
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    if replaced_ok != 0 {
        cleanup_windows_backup_after_success(backup.path());
        return Ok(());
    }

    let native_error = std::io::Error::last_os_error();
    handle_windows_replace_failure(
        path,
        candidate,
        backup.path(),
        native_error,
        |from, to| std::fs::rename(from, to),
        std::thread::sleep,
    )
}

#[cfg(any(test, windows))]
#[derive(Debug)]
struct ReplacementBackupReservation {
    path: PathBuf,
    file: Option<std::fs::File>,
    armed: bool,
}

#[cfg(any(test, windows))]
impl ReplacementBackupReservation {
    #[cfg(windows)]
    fn path(&self) -> &Path {
        &self.path
    }

    #[cfg(windows)]
    fn prepare_for_replace(&mut self) -> Result<()> {
        drop(self.file.take());
        std::fs::remove_file(&self.path)
            .with_context(|| format!("释放替换备份路径失败: {}", self.path.display()))?;
        self.armed = false;
        Ok(())
    }
}

#[cfg(any(test, windows))]
impl Drop for ReplacementBackupReservation {
    fn drop(&mut self) {
        drop(self.file.take());
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(windows)]
fn reserve_replacement_backup(live_path: &Path) -> Result<ReplacementBackupReservation> {
    use std::sync::atomic::Ordering;

    // create_new avoids stale-name collisions between cooperating fojin
    // operations. The reservation is not a security boundary against another
    // same-user process that can alter this directory concurrently.
    loop {
        let sequence = REPLACEMENT_BACKUP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = sibling_path(
            live_path,
            &format!(".replace-backup.{}.{sequence}", std::process::id()),
        )?;
        if let Some(reservation) = try_reserve_replacement_backup(&path)? {
            return Ok(reservation);
        }
    }
}

#[cfg(any(test, windows))]
fn try_reserve_replacement_backup(path: &Path) -> Result<Option<ReplacementBackupReservation>> {
    match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => Ok(Some(ReplacementBackupReservation {
            path: path.to_path_buf(),
            file: Some(file),
            armed: true,
        })),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
        Err(error) => {
            Err(error).with_context(|| format!("保留替换备份路径失败: {}", path.display()))
        }
    }
}

#[cfg(any(test, windows))]
fn cleanup_windows_backup_after_success(backup: &Path) {
    match std::fs::remove_file(backup) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => eprintln!(
            "警告: 新数据已发布,但无法删除旧数据备份 `{}`: {error}",
            backup.display()
        ),
    }
}

#[cfg(any(test, windows))]
fn handle_windows_replace_failure<F, S>(
    path: &Path,
    candidate: &Path,
    backup: &Path,
    native_error: std::io::Error,
    mut recover: F,
    mut sleep: S,
) -> ReplacementResult
where
    F: FnMut(&Path, &Path) -> std::io::Result<()>,
    S: FnMut(std::time::Duration),
{
    let original = windows_replace_error(path, candidate, backup, &native_error);
    let live_exists = match replacement_entry_exists(path) {
        Ok(exists) => exists,
        Err(error) => {
            return Err(ReplacementFailure::preserve(original.context(format!(
                "could not inspect live path `{}` after replacement failure: {error}",
                path.display()
            ))));
        }
    };
    let backup_exists = match replacement_entry_exists(backup) {
        Ok(exists) => exists,
        Err(error) => {
            return Err(ReplacementFailure::preserve(original.context(format!(
                "could not inspect replacement backup `{}`: {error}",
                backup.display()
            ))));
        }
    };

    if live_exists {
        let error = if backup_exists {
            original.context(format!(
                "live path still exists; replacement backup preserved at `{}`",
                backup.display()
            ))
        } else {
            original
        };
        return Err(ReplacementFailure::remove(error));
    }

    if backup_exists {
        return match recover_with_retries(backup, path, &mut recover, &mut sleep) {
            Ok(()) => Err(ReplacementFailure::remove(original.context(format!(
                "old live data restored from replacement backup `{}`",
                backup.display()
            )))),
            Err(recovery_error) => Err(ReplacementFailure::preserve(anyhow!(
                "{original:#}; rollback `{}` -> `{}` failed with {}; validated candidate remains at `{}` and old live remains at `{}`. manual recovery required: restore the backup to the live path before retrying",
                backup.display(),
                path.display(),
                describe_windows_error(&recovery_error),
                candidate.display(),
                backup.display()
            ))),
        };
    }

    let candidate_exists = match replacement_entry_exists(candidate) {
        Ok(exists) => exists,
        Err(error) => {
            return Err(ReplacementFailure::preserve(original.context(format!(
                "could not inspect validated candidate `{}`: {error}",
                candidate.display()
            ))));
        }
    };
    if candidate_exists {
        return match recover_with_retries(candidate, path, &mut recover, &mut sleep) {
            Ok(()) => Err(ReplacementFailure::remove(original.context(format!(
                "replacement backup was missing; validated candidate restored to live path `{}`",
                path.display()
            )))),
            Err(recovery_error) => Err(ReplacementFailure::preserve(anyhow!(
                "{original:#}; both live `{}` and backup `{}` are missing; recovery rename `{}` -> `{}` failed with {}. manual recovery required",
                path.display(),
                backup.display(),
                candidate.display(),
                path.display(),
                describe_windows_error(&recovery_error)
            ))),
        };
    }

    Err(ReplacementFailure::preserve(original.context(format!(
        "live `{}`, backup `{}`, and candidate `{}` are all missing; manual recovery required",
        path.display(),
        backup.display(),
        candidate.display()
    ))))
}

#[cfg(any(test, windows))]
fn recover_with_retries<F, S>(
    from: &Path,
    to: &Path,
    recover: &mut F,
    sleep: &mut S,
) -> std::io::Result<()>
where
    F: FnMut(&Path, &Path) -> std::io::Result<()>,
    S: FnMut(std::time::Duration),
{
    let mut last_error = None;
    for attempt in 0..3 {
        match recover(from, to) {
            Ok(()) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
        if attempt < 2 {
            sleep(std::time::Duration::from_millis(50));
        }
    }
    Err(last_error.expect("recovery attempted at least once"))
}

#[cfg(any(test, windows))]
fn replacement_entry_exists(path: &Path) -> std::io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(any(test, windows))]
fn windows_replace_error(
    path: &Path,
    candidate: &Path,
    backup: &Path,
    native_error: &std::io::Error,
) -> anyhow::Error {
    anyhow!(
        "ReplaceFileW failed for replaced `{}`, replacement `{}`, and backup `{}` with {}",
        path.display(),
        candidate.display(),
        backup.display(),
        describe_windows_error(native_error)
    )
}

#[cfg(any(test, windows))]
fn describe_windows_error(error: &std::io::Error) -> String {
    match error.raw_os_error() {
        Some(code) => format!("Windows error {code}: {error}"),
        None => error.to_string(),
    }
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
    let count = |row: &rusqlite::Row<'_>, index: usize| -> rusqlite::Result<u64> {
        let value = row.get::<_, i64>(index)?;
        u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
    };
    let total = conn.query_row("SELECT COUNT(*) FROM parallels", [], |r| count(r, 0))?;
    let texts: u64 = conn.query_row(
        "SELECT COUNT(DISTINCT cbeta_id) FROM parallels WHERE cbeta_id IS NOT NULL",
        [],
        |r| count(r, 0),
    )?;
    let mut stmt = conn.prepare(
        "SELECT foreign_lang, COUNT(*) FROM parallels GROUP BY foreign_lang ORDER BY foreign_lang",
    )?;
    let by_lang = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, count(r, 1)?)))?
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
        let staged = transfer::StagedCandidate::for_test(candidate.clone());

        let error = finish_replacement(
            staged,
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

#[cfg(test)]
mod windows_replace_state_tests {
    use super::*;
    use std::io;

    #[test]
    fn errors_with_an_intact_live_path_preserve_old_content() {
        for code in [1175, 1176, 87] {
            let directory = tempfile::tempdir().unwrap();
            let live = directory.path().join("data.sqlite");
            let candidate = directory.path().join("data.sqlite.candidate.1.1");
            let backup = directory.path().join("data.sqlite.replace-backup.1.1");
            std::fs::write(&live, format!("old-{code}")).unwrap();
            std::fs::write(&candidate, b"validated replacement").unwrap();

            let staged = transfer::StagedCandidate::for_test(candidate.clone());
            let result = handle_windows_replace_failure(
                &live,
                &candidate,
                &backup,
                io::Error::from_raw_os_error(code),
                |from, to| std::fs::rename(from, to),
                |_| {},
            );
            let error = finish_replacement(staged, result).unwrap_err();

            assert_eq!(
                std::fs::read(&live).unwrap(),
                format!("old-{code}").as_bytes()
            );
            assert!(!candidate.exists());
            assert!(!backup.exists());
            assert!(format!("{error:#}").contains(&format!("Windows error {code}")));
        }
    }

    #[test]
    fn error_1177_restores_old_live_from_backup_and_rejects_update() {
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join("data.sqlite");
        let candidate = directory.path().join("data.sqlite.candidate.1.1");
        let backup = directory.path().join("data.sqlite.replace-backup.1.1");
        std::fs::write(&candidate, b"validated replacement").unwrap();
        std::fs::write(&backup, b"old live").unwrap();

        let staged = transfer::StagedCandidate::for_test(candidate.clone());
        let result = handle_windows_replace_failure(
            &live,
            &candidate,
            &backup,
            io::Error::from_raw_os_error(1177),
            |from, to| std::fs::rename(from, to),
            |_| {},
        );
        let error = finish_replacement(staged, result).unwrap_err();

        assert_eq!(std::fs::read(&live).unwrap(), b"old live");
        assert!(!backup.exists());
        assert!(!candidate.exists());
        let detail = format!("{error:#}");
        assert!(detail.contains("Windows error 1177"), "{detail}");
        assert!(detail.contains("restored"), "{detail}");
    }

    #[test]
    fn rollback_failure_preserves_backup_and_candidate_for_manual_recovery() {
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join("data.sqlite");
        let candidate = directory.path().join("data.sqlite.candidate.1.1");
        let backup = directory.path().join("data.sqlite.replace-backup.1.1");
        std::fs::write(&candidate, b"validated replacement").unwrap();
        std::fs::write(&backup, b"old live").unwrap();

        let staged = transfer::StagedCandidate::for_test(candidate.clone());
        let result = handle_windows_replace_failure(
            &live,
            &candidate,
            &backup,
            io::Error::from_raw_os_error(1177),
            |_, _| Err(io::Error::from_raw_os_error(5)),
            |_| {},
        );
        let error = finish_replacement(staged, result).unwrap_err();

        assert!(!live.exists());
        assert_eq!(std::fs::read(&backup).unwrap(), b"old live");
        assert_eq!(std::fs::read(&candidate).unwrap(), b"validated replacement");
        let detail = format!("{error:#}");
        for expected in [
            "Windows error 1177",
            "Windows error 5",
            "manual recovery",
            live.to_string_lossy().as_ref(),
            backup.to_string_lossy().as_ref(),
            candidate.to_string_lossy().as_ref(),
        ] {
            assert!(detail.contains(expected), "missing {expected:?}: {detail}");
        }
    }

    #[test]
    fn successful_replace_removes_backup_without_touching_new_live() {
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join("data.sqlite");
        let candidate = directory.path().join("data.sqlite.candidate.1.1");
        let backup = directory.path().join("data.sqlite.replace-backup.1.1");
        std::fs::write(&live, b"new live").unwrap();
        std::fs::write(&backup, b"old live").unwrap();

        cleanup_windows_backup_after_success(&backup);

        assert_eq!(std::fs::read(&live).unwrap(), b"new live");
        assert!(!candidate.exists());
        assert!(!backup.exists());
    }

    #[test]
    fn successful_replace_keeps_a_backup_that_cannot_be_cleaned() {
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join("data.sqlite");
        let backup = directory.path().join("data.sqlite.replace-backup.1.1");
        std::fs::write(&live, b"new live").unwrap();
        std::fs::create_dir(&backup).unwrap();

        cleanup_windows_backup_after_success(&backup);

        assert_eq!(std::fs::read(&live).unwrap(), b"new live");
        assert!(backup.is_dir());
    }

    #[test]
    fn backup_reservation_skips_a_collision_without_overwriting_it() {
        let directory = tempfile::tempdir().unwrap();
        let collision = directory.path().join("data.sqlite.replace-backup.1.1");
        let available = directory.path().join("data.sqlite.replace-backup.1.2");
        std::fs::write(&collision, b"foreign backup").unwrap();

        assert!(try_reserve_replacement_backup(&collision)
            .unwrap()
            .is_none());
        let reservation = try_reserve_replacement_backup(&available)
            .unwrap()
            .expect("next unique backup name should be reservable");

        assert_eq!(std::fs::read(&collision).unwrap(), b"foreign backup");
        assert!(available.is_file());
        drop(reservation);
        assert!(!available.exists());
    }

    #[cfg(windows)]
    #[test]
    fn native_replace_file_success_preserves_new_live_and_removes_backup() {
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join("data.sqlite");
        let candidate = directory.path().join("data.sqlite.candidate.1.1");
        std::fs::write(&live, b"old live").unwrap();
        std::fs::write(&candidate, b"new live").unwrap();

        let staged = transfer::StagedCandidate::for_test(candidate.clone());
        finish_replacement(staged, replace_with_candidate(&live, &candidate)).unwrap();

        assert_eq!(std::fs::read(&live).unwrap(), b"new live");
        assert!(!candidate.exists());
        let prefix = "data.sqlite.replace-backup.";
        assert!(std::fs::read_dir(directory.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(prefix)
        }));
    }
}
