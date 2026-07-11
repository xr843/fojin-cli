use super::{download_notice, sibling_path, DataSource, Progress};
use anyhow::{anyhow, Context, Result};
use flate2::read::MultiGzDecoder;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant as StdInstant};

const MIB: u64 = 1024 * 1024;
const BUFFER_SIZE: usize = 64 * 1024;
static ARTIFACT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

// Ureq's socket adapter turns an already-expired zero timeout into one second
// because operating systems reject a zero socket timeout. TLS can also perform
// several lower-level reads for one HTTP-level read while reusing the original
// timeout. Recompute the global and connection-phase deadlines plus the
// per-read idle timeout at both transport layers so neither buffered protocol
// bytes nor a dribbling TLS record can extend a configured boundary. The
// transport API is unversioned, so ureq is pinned exactly in Cargo.toml.
#[derive(Clone, Copy, Debug)]
struct AbsoluteDeadlineConnector {
    deadline: StdInstant,
    connect_timeout: Duration,
    idle_read_timeout: Duration,
}

#[derive(Debug)]
struct AbsoluteDeadlineTransport<T> {
    inner: T,
    deadline: StdInstant,
    connect_timeout: Duration,
    connect_phase_deadline: StdInstant,
    connect_phase_active: bool,
    idle_read_timeout: Duration,
}

impl<T: ureq::unversioned::transport::Transport> ureq::unversioned::transport::Connector<T>
    for AbsoluteDeadlineConnector
{
    type Out = AbsoluteDeadlineTransport<T>;

    fn connect(
        &self,
        details: &ureq::unversioned::transport::ConnectionDetails<'_>,
        chained: Option<T>,
    ) -> std::result::Result<Option<Self::Out>, ureq::Error> {
        // ConnectionDetails::now is captured before the connector chain runs,
        // so this includes TCP and CONNECT-proxy work that preceded this
        // wrapper instead of restarting the budget after the socket opened.
        let connect_phase_started = match details.now {
            ureq::unversioned::transport::time::Instant::Exact(started) => started,
            _ => {
                return Err(ureq::Error::Io(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "HTTP transport did not provide an exact connection start time",
                )));
            }
        };
        let connect_phase_deadline = checked_deadline(connect_phase_started, self.connect_timeout)?;
        Ok(chained.map(|inner| AbsoluteDeadlineTransport {
            inner,
            deadline: self.deadline,
            connect_timeout: self.connect_timeout,
            connect_phase_deadline,
            connect_phase_active: true,
            idle_read_timeout: self.idle_read_timeout,
        }))
    }
}

impl<T> AbsoluteDeadlineTransport<T> {
    fn bound_timeout(
        &mut self,
        timeout: ureq::unversioned::transport::NextTimeout,
        idle_read_timeout: Option<Duration>,
    ) -> std::result::Result<ureq::unversioned::transport::NextTimeout, ureq::Error> {
        let now = StdInstant::now();
        let connect_deadline = if timeout.reason == ureq::Timeout::Connect {
            if !self.connect_phase_active {
                self.connect_phase_deadline = checked_deadline(now, self.connect_timeout)?;
                self.connect_phase_active = true;
            }
            Some(self.connect_phase_deadline)
        } else {
            self.connect_phase_active = false;
            None
        };
        Ok(bound_transport_timeout(
            timeout,
            self.deadline,
            connect_deadline,
            idle_read_timeout,
            now,
        ))
    }
}

impl<T: ureq::unversioned::transport::Transport> ureq::unversioned::transport::Transport
    for AbsoluteDeadlineTransport<T>
{
    fn buffers(&mut self) -> &mut dyn ureq::unversioned::transport::Buffers {
        self.inner.buffers()
    }

    fn transmit_output(
        &mut self,
        amount: usize,
        timeout: ureq::unversioned::transport::NextTimeout,
    ) -> std::result::Result<(), ureq::Error> {
        let timeout = self.bound_timeout(timeout, None)?;
        require_remaining_timeout(timeout)?;
        self.inner.transmit_output(amount, timeout)
    }

    fn maybe_await_input(
        &mut self,
        timeout: ureq::unversioned::transport::NextTimeout,
    ) -> std::result::Result<bool, ureq::Error> {
        let timeout = self.bound_timeout(timeout, Some(self.idle_read_timeout))?;
        require_remaining_timeout(timeout)?;
        self.inner.maybe_await_input(timeout)
    }

    fn await_input(
        &mut self,
        timeout: ureq::unversioned::transport::NextTimeout,
    ) -> std::result::Result<bool, ureq::Error> {
        let timeout = self.bound_timeout(timeout, Some(self.idle_read_timeout))?;
        require_remaining_timeout(timeout)?;
        self.inner.await_input(timeout)
    }

    fn is_open(&mut self) -> bool {
        self.inner.is_open()
    }

    fn is_tls(&self) -> bool {
        self.inner.is_tls()
    }
}

fn bound_transport_timeout(
    mut timeout: ureq::unversioned::transport::NextTimeout,
    deadline: StdInstant,
    connect_deadline: Option<StdInstant>,
    idle_read_timeout: Option<Duration>,
    now: StdInstant,
) -> ureq::unversioned::transport::NextTimeout {
    let remaining = deadline.saturating_duration_since(now);
    let global_after: ureq::unversioned::transport::time::Duration = remaining.into();
    if global_after < timeout.after {
        timeout = ureq::unversioned::transport::NextTimeout {
            after: global_after,
            reason: ureq::Timeout::Global,
        };
    }

    if let Some(connect_deadline) = connect_deadline {
        let remaining = connect_deadline.saturating_duration_since(now);
        let connect_after: ureq::unversioned::transport::time::Duration = remaining.into();
        if connect_after < timeout.after {
            timeout = ureq::unversioned::transport::NextTimeout {
                after: connect_after,
                reason: ureq::Timeout::Connect,
            };
        }
    }

    if let Some(idle_read_timeout) = idle_read_timeout {
        let idle_after: ureq::unversioned::transport::time::Duration = idle_read_timeout.into();
        if idle_after < timeout.after {
            timeout = ureq::unversioned::transport::NextTimeout {
                after: idle_after,
                reason: ureq::Timeout::RecvResponse,
            };
        }
    }

    timeout
}

fn checked_deadline(
    started: StdInstant,
    timeout: Duration,
) -> std::result::Result<StdInstant, ureq::Error> {
    started.checked_add(timeout).ok_or_else(|| {
        ureq::Error::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "HTTP timeout exceeds the system clock range",
        ))
    })
}

fn require_remaining_timeout(
    timeout: ureq::unversioned::transport::NextTimeout,
) -> std::result::Result<(), ureq::Error> {
    if !timeout.after.is_not_happening() && timeout.after.is_zero() {
        Err(ureq::Error::Timeout(timeout.reason))
    } else {
        Ok(())
    }
}

fn require_absolute_deadline(deadline: StdInstant) -> std::result::Result<(), ureq::Error> {
    if StdInstant::now() >= deadline {
        Err(ureq::Error::Timeout(ureq::Timeout::Global))
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct DownloadPolicy {
    pub connect_timeout: Duration,
    pub idle_read_timeout: Duration,
    pub http_timeout: Duration,
    pub max_compressed: u64,
    pub max_uncompressed: u64,
}

pub(super) const PRODUCTION_POLICY: DownloadPolicy = DownloadPolicy {
    connect_timeout: Duration::from_secs(30),
    idle_read_timeout: Duration::from_secs(60),
    http_timeout: Duration::from_secs(15 * 60),
    max_compressed: 256 * MIB,
    max_uncompressed: 768 * MIB,
};

pub(super) struct StagedCandidate {
    path: PathBuf,
    armed: bool,
}

struct OwnedCompressed {
    path: PathBuf,
    armed: bool,
}

impl OwnedCompressed {
    fn remove_now(mut self) -> Result<()> {
        self.armed = false;
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => {
                Err(error).with_context(|| format!("删除压缩临时文件失败: {}", self.path.display()))
            }
        }
    }

    fn cleanup_with(mut self, error: anyhow::Error) -> anyhow::Error {
        self.armed = false;
        match std::fs::remove_file(&self.path) {
            Ok(()) => error,
            Err(cleanup) if cleanup.kind() == io::ErrorKind::NotFound => error,
            Err(cleanup) => error.context(format!(
                "清理压缩临时文件失败: {}: {cleanup}",
                self.path.display()
            )),
        }
    }
}

impl Drop for OwnedCompressed {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl StagedCandidate {
    #[cfg(test)]
    pub(super) fn for_test(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn publish_succeeded(mut self) {
        self.armed = false;
    }

    #[cfg(any(test, windows))]
    pub(super) fn preserve(mut self) {
        self.armed = false;
    }

    pub(super) fn cleanup_with(mut self, error: anyhow::Error) -> anyhow::Error {
        self.armed = false;
        match remove_artifact_family(&self.path) {
            Ok(()) => error,
            Err(cleanup) => error.context(format!("清理候选数据失败: {cleanup}")),
        }
    }
}

impl Drop for StagedCandidate {
    fn drop(&mut self) {
        if self.armed {
            let _ = remove_artifact_family(&self.path);
        }
    }
}

fn unique_path(live_path: &Path, role: &str, extension: &str) -> Result<PathBuf> {
    let sequence = ARTIFACT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    sibling_path(
        live_path,
        &format!(".{role}.{}.{sequence}{extension}", std::process::id()),
    )
}

fn create_compressed_artifact(live_path: &Path) -> Result<(OwnedCompressed, File)> {
    loop {
        let path = unique_path(live_path, "download", ".gz")?;
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Ok((OwnedCompressed { path, armed: true }, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("创建压缩临时文件失败: {}", path.display()));
            }
        }
    }
}

fn create_candidate_artifact(live_path: &Path) -> Result<(StagedCandidate, File)> {
    loop {
        let path = unique_path(live_path, "candidate", "")?;
        if let Some(candidate) = try_create_candidate_artifact(&path)? {
            return Ok(candidate);
        }
    }
}

fn try_create_candidate_artifact(path: &Path) -> Result<Option<(StagedCandidate, File)>> {
    try_create_candidate_artifact_with(path, path_entry_exists)
}

fn try_create_candidate_artifact_with<F>(
    path: &Path,
    mut entry_exists: F,
) -> Result<Option<(StagedCandidate, File)>>
where
    F: FnMut(&Path) -> Result<bool>,
{
    let sidecars = ["-journal", "-shm", "-wal"]
        .map(|suffix| sibling_path(path, suffix))
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    let family_occupied = sidecars
        .iter()
        .try_fold(entry_exists(path)?, |occupied, sidecar| {
            entry_exists(sidecar).map(|exists| occupied || exists)
        })?;
    if family_occupied {
        return Ok(None);
    }

    let file = match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("创建候选数据失败: {}", path.display()));
        }
    };

    // The main file reserves this generation for cooperating fojin processes.
    // Recheck the sidecars so a stale family is not claimed and later deleted
    // by this guard. After this point, the operation lock ensures cooperating
    // processes leave this unique family to SQLite. This naming protocol is
    // coordination, not isolation from a same-user process that can modify the
    // data directory concurrently.
    let sidecar_occupied = sidecars.iter().try_fold(false, |occupied, sidecar| {
        entry_exists(sidecar).map(|exists| occupied || exists)
    });
    let sidecar_occupied = match sidecar_occupied {
        Ok(occupied) => occupied,
        Err(error) => {
            drop(file);
            return Err(cleanup_owned_candidate_main(path, error));
        }
    };
    if sidecar_occupied {
        drop(file);
        std::fs::remove_file(path)
            .with_context(|| format!("释放冲突候选数据失败: {}", path.display()))?;
        return Ok(None);
    }

    Ok(Some((
        StagedCandidate {
            path: path.to_path_buf(),
            armed: true,
        },
        file,
    )))
}

fn cleanup_owned_candidate_main(path: &Path, error: anyhow::Error) -> anyhow::Error {
    match std::fs::remove_file(path) {
        Ok(()) => error,
        Err(cleanup) if cleanup.kind() == io::ErrorKind::NotFound => error,
        Err(cleanup) => error.context(format!(
            "清理候选数据主文件失败: {}: {cleanup}",
            path.display()
        )),
    }
}

fn path_entry_exists(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => {
            Err(error).with_context(|| format!("检查临时数据路径失败: {}", path.display()))
        }
    }
}

fn remove_artifact_family(path: &Path) -> Result<()> {
    for suffix in ["", "-journal", "-shm", "-wal"] {
        let artifact = if suffix.is_empty() {
            path.to_path_buf()
        } else {
            sibling_path(path, suffix)?
        };
        match std::fs::remove_file(&artifact) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("删除候选数据失败: {}", artifact.display()));
            }
        }
    }
    Ok(())
}

pub(super) fn remove_known_artifacts(live_path: &Path) -> Result<()> {
    let legacy = live_path.with_extension("tmp");
    match std::fs::remove_file(&legacy) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("删除旧临时数据失败: {}", legacy.display()));
        }
    }

    let directory = live_path
        .parent()
        .ok_or_else(|| anyhow!("数据路径没有父目录: {}", live_path.display()))?;
    let file_name = live_path
        .file_name()
        .ok_or_else(|| anyhow!("数据路径没有文件名: {}", live_path.display()))?
        .to_str()
        .ok_or_else(|| anyhow!("数据文件名不是有效 UTF-8: {}", live_path.display()))?;
    for entry in std::fs::read_dir(directory)
        .with_context(|| format!("读取数据目录失败: {}", directory.display()))?
    {
        let entry = entry.context("读取数据目录项失败")?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !is_owned_artifact_name(file_name, name) {
            continue;
        }
        if entry.file_type().context("读取临时数据类型失败")?.is_dir() {
            continue;
        }
        std::fs::remove_file(entry.path())
            .with_context(|| format!("删除临时数据失败: {}", entry.path().display()))?;
    }
    Ok(())
}

fn is_owned_artifact_name(live_name: &str, artifact_name: &str) -> bool {
    let Some(suffix) = artifact_name.strip_prefix(live_name) else {
        return false;
    };

    if let Some(generation) = suffix
        .strip_prefix(".download.")
        .and_then(|name| name.strip_suffix(".gz"))
    {
        return is_numeric_generation(generation);
    }

    let Some(candidate) = suffix.strip_prefix(".candidate.") else {
        return false;
    };
    let generation = ["-journal", "-shm", "-wal"]
        .into_iter()
        .find_map(|sidecar| candidate.strip_suffix(sidecar))
        .unwrap_or(candidate);
    is_numeric_generation(generation)
}

fn is_numeric_generation(value: &str) -> bool {
    let mut parts = value.split('.');
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(pid), Some(sequence), None)
            if !pid.is_empty()
                && !sequence.is_empty()
                && pid.bytes().all(|byte| byte.is_ascii_digit())
                && sequence.bytes().all(|byte| byte.is_ascii_digit())
    )
}

fn copy_bounded(
    mut reader: impl Read,
    mut writer: impl Write,
    maximum: u64,
    label: &str,
) -> Result<u64> {
    let mut limited = reader.by_ref().take(maximum.saturating_add(1));
    let copied = io::copy(&mut limited, &mut writer).with_context(|| format!("{label}失败"))?;
    if copied > maximum {
        return Err(anyhow!("{label}超过限制: 最大 {maximum} 字节"));
    }
    Ok(copied)
}

fn unpack_gzip(reader: impl Read, writer: impl Write, maximum: u64) -> Result<u64> {
    let decoder = MultiGzDecoder::new(reader);
    copy_bounded(decoder, writer, maximum, "解压 gzip")
}

pub(super) fn stage_candidate(
    live_path: &Path,
    source: &DataSource<'_>,
    policy: DownloadPolicy,
) -> Result<StagedCandidate> {
    let (compressed_guard, mut compressed_file) = create_compressed_artifact(live_path)?;
    let staged = stage_candidate_inner(live_path, source, policy, &mut compressed_file);
    match staged {
        Ok(candidate) => {
            finish_successful_stage_with(compressed_guard, candidate, OwnedCompressed::remove_now)
        }
        Err(error) => Err(compressed_guard.cleanup_with(error)),
    }
}

fn finish_successful_stage_with<RemoveCompressed>(
    compressed_guard: OwnedCompressed,
    candidate: StagedCandidate,
    remove_compressed: RemoveCompressed,
) -> Result<StagedCandidate>
where
    RemoveCompressed: FnOnce(OwnedCompressed) -> Result<()>,
{
    match remove_compressed(compressed_guard) {
        Ok(()) => Ok(candidate),
        Err(error) => Err(candidate.cleanup_with(error)),
    }
}

fn stage_candidate_inner(
    live_path: &Path,
    source: &DataSource<'_>,
    policy: DownloadPolicy,
    compressed_file: &mut File,
) -> Result<StagedCandidate> {
    let deadline = StdInstant::now()
        .checked_add(policy.http_timeout)
        .ok_or_else(|| anyhow!("HTTP 总超时超出系统时间范围"))?;
    let config = ureq::Agent::config_builder()
        .max_redirects(5)
        .max_redirects_will_error(true)
        .max_response_header_size(64 * 1024)
        .accept_encoding("identity")
        .timeout_global(Some(policy.http_timeout))
        .timeout_resolve(Some(policy.connect_timeout))
        .timeout_connect(Some(policy.connect_timeout))
        .timeout_recv_body(Some(policy.idle_read_timeout))
        .build();
    use ureq::unversioned::transport::Connector;
    let deadline_connector = AbsoluteDeadlineConnector {
        deadline,
        connect_timeout: policy.connect_timeout,
        idle_read_timeout: policy.idle_read_timeout,
    };
    let connector =
        ().chain(ureq::unversioned::transport::ConnectProxyConnector::default())
            .chain(ureq::unversioned::transport::TcpConnector::default())
            .chain(deadline_connector)
            .chain(ureq::unversioned::transport::RustlsConnector::default())
            .chain(deadline_connector);
    let agent = ureq::Agent::with_parts(
        config,
        connector,
        ureq::unversioned::resolver::DefaultResolver::default(),
    );
    let mut response = agent
        .get(source.url)
        .header("Accept-Encoding", "identity")
        .call()
        .map_err(normalize_ureq_error)
        .map_err(|error| download_error_context(error, "下载失败", live_path, source.url))?;
    let declared = declared_length(&response, policy.max_compressed)?;
    eprintln!("{}", download_notice(declared));

    let mut reader = response
        .body_mut()
        .with_config()
        .limit(policy.max_compressed.saturating_add(1))
        .reader();
    let mut progress = Progress::new(declared);
    let mut digest = Sha256::new();
    let mut received = 0_u64;
    let mut buffer = [0_u8; BUFFER_SIZE];
    loop {
        require_absolute_deadline(deadline)
            .map_err(normalize_ureq_error)
            .map_err(|error| {
                download_error_context(error, "读取响应失败", live_path, source.url)
            })?;
        let count = match reader.read(&mut buffer) {
            Ok(count) => {
                require_absolute_deadline(deadline)
                    .map_err(normalize_ureq_error)
                    .map_err(|error| {
                        download_error_context(error, "读取响应失败", live_path, source.url)
                    })?;
                count
            }
            Err(error) => {
                return Err(download_error_context(
                    anyhow::Error::new(normalize_body_read_error(error)),
                    "读取响应失败",
                    live_path,
                    source.url,
                ));
            }
        };
        if count == 0 {
            break;
        }
        received = received
            .checked_add(count as u64)
            .ok_or_else(|| anyhow!("下载大小溢出"))?;
        if received > policy.max_compressed {
            return Err(anyhow!(
                "下载数据超过限制: 最大 {} 字节",
                policy.max_compressed
            ));
        }
        digest.update(&buffer[..count]);
        compressed_file
            .write_all(&buffer[..count])
            .context("写入压缩临时文件失败")?;
        if let Some(message) = progress.advance(count as u64) {
            eprintln!("{message}");
        }
    }
    require_declared_length(declared, received)
        .map_err(|error| download_error_context(error, "读取响应失败", live_path, source.url))?;
    let actual = digest.finalize();
    require_digest(actual.as_ref(), source.sha256).map_err(|error| {
        anyhow!(
            "{error}\n请手动下载:\n  {}\n解压后放到: {}",
            source.url,
            live_path.display()
        )
    })?;
    compressed_file.flush().context("刷新压缩临时文件失败")?;
    compressed_file.rewind().context("重置压缩临时文件失败")?;

    let (candidate_guard, mut candidate_file) = create_candidate_artifact(live_path)?;
    let unpacked = (|| -> Result<()> {
        unpack_gzip(
            &mut *compressed_file,
            &mut candidate_file,
            policy.max_uncompressed,
        )?;
        candidate_file.flush().context("刷新候选数据失败")?;
        candidate_file.sync_all().context("同步候选数据失败")?;
        Ok(())
    })();
    drop(candidate_file);
    match unpacked {
        Ok(()) => Ok(candidate_guard),
        Err(error) => Err(candidate_guard.cleanup_with(error)),
    }
}

fn download_error_context(
    error: anyhow::Error,
    action: &str,
    live_path: &Path,
    url: &str,
) -> anyhow::Error {
    let cause = error.to_string();
    error.context(format!(
        "{action}: {url}: {cause}\n请手动下载:\n  {url}\n解压后放到: {}",
        live_path.display()
    ))
}

fn normalize_ureq_error(error: ureq::Error) -> anyhow::Error {
    if matches!(error, ureq::Error::Timeout(_)) {
        anyhow::Error::new(io::Error::new(io::ErrorKind::TimedOut, error))
    } else {
        anyhow::Error::new(error)
    }
}

fn normalize_body_read_error(error: io::Error) -> io::Error {
    let is_timeout = error
        .get_ref()
        .and_then(|source| source.downcast_ref::<ureq::Error>())
        .is_some_and(|source| matches!(source, ureq::Error::Timeout(_)));
    if is_timeout {
        io::Error::new(io::ErrorKind::TimedOut, error)
    } else {
        error
    }
}

fn declared_length(
    response: &ureq::http::Response<ureq::Body>,
    maximum: u64,
) -> Result<Option<u64>> {
    if response
        .headers()
        .get_all(ureq::http::header::TRANSFER_ENCODING)
        .iter()
        .next()
        .is_some()
    {
        return Ok(None);
    }
    let values: Vec<_> = response
        .headers()
        .get_all(ureq::http::header::CONTENT_LENGTH)
        .iter()
        .collect();
    match values.as_slice() {
        [] => Ok(None),
        [value] => {
            let value = value.to_str().context("Content-Length 不是有效 ASCII")?;
            let parsed = value
                .parse::<u64>()
                .with_context(|| format!("无效 Content-Length: {value}"))?;
            if parsed > maximum {
                return Err(anyhow!("Content-Length 超过下载限制: {parsed} > {maximum}"));
            }
            Ok(Some(parsed))
        }
        _ => Err(anyhow!("响应包含重复 Content-Length")),
    }
}

fn require_declared_length(declared: Option<u64>, received: u64) -> Result<()> {
    if let Some(expected) = declared {
        if expected != received {
            return Err(anyhow!(
                "响应长度不符: Content-Length={expected}, received={received}"
            ));
        }
    }
    Ok(())
}

fn require_digest(actual: &[u8], expected_hex: &str) -> Result<()> {
    if expected_hex.len() != 64 || !expected_hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(anyhow!("配置的 SHA-256 格式无效"));
    }
    let actual_hex: String = actual.iter().map(|byte| format!("{byte:02x}")).collect();
    if !actual_hex.eq_ignore_ascii_case(expected_hex) {
        return Err(anyhow!("下载校验失败(sha256 不符)"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::{Cursor, Read, Write};
    use std::time::Instant;

    fn policy_for_tests() -> DownloadPolicy {
        DownloadPolicy {
            connect_timeout: Duration::from_millis(200),
            idle_read_timeout: Duration::from_millis(200),
            http_timeout: Duration::from_secs(2),
            max_compressed: 1024,
            max_uncompressed: 4096,
        }
    }

    #[test]
    fn candidate_reservation_rejects_a_preexisting_sidecar_family() {
        let directory = tempfile::tempdir().unwrap();
        let candidate = directory.path().join("data.sqlite.candidate.123.4");
        let wal = sibling_path(&candidate, "-wal").unwrap();
        std::fs::write(&wal, b"foreign wal").unwrap();

        assert!(try_create_candidate_artifact(&candidate).unwrap().is_none());
        assert!(!candidate.exists());
        assert_eq!(std::fs::read(wal).unwrap(), b"foreign wal");
    }

    #[test]
    fn candidate_reservation_cleans_its_main_file_when_postcheck_fails() {
        let directory = tempfile::tempdir().unwrap();
        let candidate = directory.path().join("data.sqlite.candidate.123.4");
        let checks = std::cell::Cell::new(0_usize);

        let result = try_create_candidate_artifact_with(&candidate, |_| {
            let next = checks.get() + 1;
            checks.set(next);
            if next == 5 {
                Err(anyhow!("injected sidecar inspection failure"))
            } else {
                Ok(false)
            }
        });
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("post-reservation inspection failure was ignored"),
        };

        assert!(
            format!("{error:#}").contains("injected sidecar inspection failure"),
            "got: {error:#}"
        );
        assert!(!candidate.exists(), "owned candidate main leaked");
    }

    #[test]
    fn compressed_removal_failure_attaches_candidate_cleanup_error() {
        let directory = tempfile::tempdir().unwrap();
        let compressed_path = directory.path().join("data.sqlite.download.123.4.gz");
        std::fs::write(&compressed_path, b"compressed").unwrap();
        let compressed_guard = OwnedCompressed {
            path: compressed_path,
            armed: true,
        };
        let candidate_path = directory.path().join("data.sqlite.candidate.123.4");
        std::fs::create_dir(&candidate_path).unwrap();
        let candidate = StagedCandidate::for_test(candidate_path);

        let result = finish_successful_stage_with(compressed_guard, candidate, |_| {
            Err(anyhow!("injected compressed removal failure"))
        });
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("compressed removal failure was ignored"),
        };
        let detail = format!("{error:#}");

        assert!(
            detail.contains("injected compressed removal failure"),
            "got: {detail}"
        );
        assert!(detail.contains("清理候选数据失败"), "got: {detail}");
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn gzip(body: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(body).unwrap();
        encoder.finish().unwrap()
    }

    fn read_request(stream: &mut std::net::TcpStream) -> Vec<u8> {
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = stream.read(&mut buffer).unwrap();
            if count == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..count]);
        }
        request
    }

    fn stage_from_raw_response_with_sha(
        response: &[u8],
        sha256: &str,
        policy: DownloadPolicy,
    ) -> (Result<()>, Vec<u8>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let response = response.to_vec();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            stream.write_all(&response).unwrap();
            request
        });
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join("data.sqlite");
        let url = format!("http://{address}/data.gz");
        let result = stage_candidate(&live, &DataSource { url: &url, sha256 }, policy).map(drop);
        let request = server.join().unwrap();
        (result, request)
    }

    fn stage_from_raw_response(response: &[u8], policy: DownloadPolicy) -> Result<()> {
        stage_from_raw_response_with_sha(response, &"0".repeat(64), policy).0
    }

    fn stage_from_delayed_raw_response(
        response: &[u8],
        delay: Duration,
        policy: DownloadPolicy,
    ) -> Result<()> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let response = response.to_vec();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_request(&mut stream);
            std::thread::sleep(delay);
            stream.write_all(&response).unwrap();
        });
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join("data.sqlite");
        let url = format!("http://{address}/data.gz");
        let sha = "0".repeat(64);
        let result = stage_candidate(
            &live,
            &DataSource {
                url: &url,
                sha256: &sha,
            },
            policy,
        )
        .map(drop);
        server.join().unwrap();
        result
    }

    fn stage_from_dribbling_server(interval: Duration, policy: DownloadPolicy) -> Result<()> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_request(&mut stream);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
                .unwrap();
            for _ in 0..20 {
                if stream.write_all(b"x").is_err() {
                    break;
                }
                if stream.flush().is_err() {
                    break;
                }
                std::thread::sleep(interval);
            }
        });
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join("data.sqlite");
        let url = format!("http://{address}/data.gz");
        let sha = "0".repeat(64);
        let result = stage_candidate(
            &live,
            &DataSource {
                url: &url,
                sha256: &sha,
            },
            policy,
        )
        .map(drop);
        server.join().unwrap();
        result
    }

    fn stage_from_pausing_server(pause: Duration, policy: DownloadPolicy) -> Result<()> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_request(&mut stream);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nx")
                .unwrap();
            stream.flush().unwrap();
            std::thread::sleep(pause);
            let _ = stream.write_all(b"x");
        });
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join("data.sqlite");
        let url = format!("http://{address}/data.gz");
        let sha = "0".repeat(64);
        let result = stage_candidate(
            &live,
            &DataSource {
                url: &url,
                sha256: &sha,
            },
            policy,
        )
        .map(drop);
        server.join().unwrap();
        result
    }

    fn stage_from_pausing_headers(pause: Duration, policy: DownloadPolicy) -> Result<()> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_request(&mut stream);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n")
                .unwrap();
            stream.flush().unwrap();
            std::thread::sleep(pause);
            let _ = stream.write_all(b"\r\n");
        });
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join("data.sqlite");
        let url = format!("http://{address}/data.gz");
        let sha = "0".repeat(64);
        let result = stage_candidate(
            &live,
            &DataSource {
                url: &url,
                sha256: &sha,
            },
            policy,
        )
        .map(drop);
        server.join().unwrap();
        result
    }

    fn stage_from_protocol_dribble(
        prefix: &'static [u8],
        dribble: u8,
        count: usize,
        suffix: &'static [u8],
        interval: Duration,
        policy: DownloadPolicy,
    ) -> (Result<()>, Duration) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_request(&mut stream);
            if stream.write_all(prefix).is_err() {
                return;
            }
            for _ in 0..count {
                if stream.write_all(&[dribble]).is_err() || stream.flush().is_err() {
                    return;
                }
                std::thread::sleep(interval);
            }
            let _ = stream.write_all(suffix);
        });
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join("data.sqlite");
        let url = format!("http://{address}/data.gz");
        let sha = "0".repeat(64);
        let started = Instant::now();
        let result = stage_candidate(
            &live,
            &DataSource {
                url: &url,
                sha256: &sha,
            },
            policy,
        )
        .map(drop);
        let elapsed = started.elapsed();
        server.join().unwrap();
        (result, elapsed)
    }

    fn contains_io_kind(error: &anyhow::Error, expected: io::ErrorKind) -> bool {
        error.chain().any(|cause| {
            cause
                .downcast_ref::<io::Error>()
                .is_some_and(|error| error.kind() == expected)
        })
    }

    fn contains_ureq_timeout(error: &anyhow::Error, expected: ureq::Timeout) -> bool {
        fn source_contains_timeout(
            source: &(dyn std::error::Error + 'static),
            expected: ureq::Timeout,
        ) -> bool {
            if let Some(source) = source.downcast_ref::<ureq::Error>() {
                return match source {
                    ureq::Error::Timeout(reason) => *reason == expected,
                    ureq::Error::Io(source) => source_contains_timeout(source, expected),
                    _ => false,
                };
            }
            source
                .downcast_ref::<io::Error>()
                .and_then(io::Error::get_ref)
                .is_some_and(|source| source_contains_timeout(source, expected))
        }

        error
            .chain()
            .any(|cause| source_contains_timeout(cause, expected))
    }

    fn assert_no_owned_transfer_artifacts(live: &Path) {
        let live_name = live.file_name().unwrap().to_string_lossy();
        let owned: Vec<_> = std::fs::read_dir(live.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| {
                is_owned_artifact_name(live_name.as_ref(), name.to_string_lossy().as_ref())
            })
            .collect();
        assert!(
            owned.is_empty(),
            "owned transfer artifacts remain: {owned:?}"
        );
    }

    fn chunked_response(body: &[u8], extra_headers: &str) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n{extra_headers}Connection: close\r\n\r\n{:x}\r\n",
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(body);
        response.extend_from_slice(b"\r\n0\r\n\r\n");
        response
    }

    fn content_length_response(body: &[u8]) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(body);
        response
    }

    fn stage_compressed(compressed: &[u8], policy: DownloadPolicy) -> Result<()> {
        let sha = sha256_hex(compressed);
        let response = content_length_response(compressed);
        stage_from_raw_response_with_sha(&response, &sha, policy).0
    }

    #[test]
    fn duplicate_content_length_is_rejected() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nContent-Length: 4\r\nConnection: close\r\n\r\ntest";
        let error = stage_from_raw_response(response, policy_for_tests())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("Content-Length") && error.contains("重复"),
            "got: {error}"
        );
    }

    #[test]
    fn declared_length_must_match_received_body() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\ntest";
        let error = stage_from_raw_response(response, policy_for_tests()).unwrap_err();
        assert!(
            error.to_string().contains("长度") || error.to_string().contains("读取响应失败"),
            "got: {error:#}"
        );
        assert!(
            contains_io_kind(&error, io::ErrorKind::UnexpectedEof),
            "got: {error:#}"
        );
        assert!(
            format!("{error:#}").contains("请手动下载"),
            "got: {error:#}"
        );
    }

    #[test]
    fn total_timeout_stops_a_non_idle_dribble() {
        let policy = DownloadPolicy {
            connect_timeout: Duration::from_secs(1),
            idle_read_timeout: Duration::from_millis(600),
            http_timeout: Duration::from_millis(300),
            max_compressed: 1024,
            max_uncompressed: 4096,
        };
        let error = stage_from_dribbling_server(Duration::from_millis(50), policy).unwrap_err();
        assert!(
            error.to_string().contains("读取响应失败") || error.to_string().contains("timed out"),
            "got: {error:#}"
        );
        assert!(
            contains_io_kind(&error, io::ErrorKind::TimedOut),
            "got: {error:#}"
        );
        assert!(
            contains_ureq_timeout(&error, ureq::Timeout::Global),
            "got: {error:#}"
        );
    }

    #[test]
    fn slow_progressing_valid_body_can_outlive_the_idle_window() {
        let compressed = gzip(b"slow valid body");
        let sha = sha256_hex(&compressed);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_body = compressed.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_nodelay(true).unwrap();
            let _ = read_request(&mut stream);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                server_body.len()
            )
            .unwrap();
            for (index, byte) in server_body.iter().enumerate() {
                stream.write_all(std::slice::from_ref(byte)).unwrap();
                stream.flush().unwrap();
                if index + 1 < server_body.len() {
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        });
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join("data.sqlite");
        let url = format!("http://{address}/data.gz");
        let policy = DownloadPolicy {
            connect_timeout: Duration::from_millis(500),
            idle_read_timeout: Duration::from_millis(250),
            http_timeout: Duration::from_secs(2),
            max_compressed: 1024,
            max_uncompressed: 4096,
        };

        let candidate = stage_candidate(
            &live,
            &DataSource {
                url: &url,
                sha256: &sha,
            },
            policy,
        )
        .expect("continuously progressing body should not hit the idle timeout");
        server.join().unwrap();

        assert_eq!(std::fs::read(candidate.path()).unwrap(), b"slow valid body");
    }

    #[test]
    fn global_deadline_interrupts_a_dribbling_tls_record() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_nodelay(true).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut client_hello = [0_u8; 16 * 1024];
            let _ = stream.read(&mut client_hello).unwrap();
            stream.write_all(&[0x16, 0x03, 0x03, 0x40, 0x00]).unwrap();
            stream.flush().unwrap();
            for _ in 0..24 {
                if stream.write_all(&[0]).is_err() || stream.flush().is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join("data.sqlite");
        let url = format!("https://{address}/data.gz");
        let sha = "0".repeat(64);
        let policy = DownloadPolicy {
            connect_timeout: Duration::from_secs(1),
            idle_read_timeout: Duration::from_secs(1),
            http_timeout: Duration::from_millis(250),
            max_compressed: 1024,
            max_uncompressed: 4096,
        };

        let started = Instant::now();
        let error = stage_candidate(
            &live,
            &DataSource {
                url: &url,
                sha256: &sha,
            },
            policy,
        )
        .map(drop)
        .unwrap_err();
        let elapsed = started.elapsed();
        server.join().unwrap();

        assert!(elapsed < Duration::from_millis(700), "elapsed: {elapsed:?}");
        assert!(
            contains_ureq_timeout(&error, ureq::Timeout::Global),
            "got: {error:#}"
        );
        assert_no_owned_transfer_artifacts(&live);
    }

    #[test]
    fn connect_deadline_interrupts_a_dribbling_tls_record() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_nodelay(true).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut client_hello = [0_u8; 16 * 1024];
            let _ = stream.read(&mut client_hello).unwrap();
            stream.write_all(&[0x16, 0x03, 0x03, 0x40, 0x00]).unwrap();
            stream.flush().unwrap();
            for _ in 0..24 {
                if stream.write_all(&[0]).is_err() || stream.flush().is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join("data.sqlite");
        let url = format!("https://{address}/data.gz");
        let sha = "0".repeat(64);
        let policy = DownloadPolicy {
            connect_timeout: Duration::from_millis(250),
            idle_read_timeout: Duration::from_secs(1),
            http_timeout: Duration::from_secs(2),
            max_compressed: 1024,
            max_uncompressed: 4096,
        };

        let started = Instant::now();
        let error = stage_candidate(
            &live,
            &DataSource {
                url: &url,
                sha256: &sha,
            },
            policy,
        )
        .map(drop)
        .unwrap_err();
        let elapsed = started.elapsed();
        server.join().unwrap();

        assert!(elapsed < Duration::from_millis(700), "elapsed: {elapsed:?}");
        assert!(
            contains_ureq_timeout(&error, ureq::Timeout::Connect),
            "got: {error:#}"
        );
        assert_no_owned_transfer_artifacts(&live);
    }

    #[test]
    fn global_timeout_interrupts_continuously_dribbling_response_headers() {
        let policy = DownloadPolicy {
            connect_timeout: Duration::from_secs(1),
            idle_read_timeout: Duration::from_millis(500),
            http_timeout: Duration::from_millis(250),
            max_compressed: 1024,
            max_uncompressed: 4096,
        };
        let (result, elapsed) = stage_from_protocol_dribble(
            b"HTTP/1.1 200 OK\r\nX-Slow: ",
            b'a',
            40,
            b"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            Duration::from_millis(50),
            policy,
        );
        let error = result.unwrap_err();
        assert!(elapsed < Duration::from_secs(1), "elapsed: {elapsed:?}");
        assert!(
            contains_io_kind(&error, io::ErrorKind::TimedOut),
            "got: {error:#}"
        );
        assert!(
            contains_ureq_timeout(&error, ureq::Timeout::Global),
            "got: {error:#}"
        );
        assert!(
            format!("{error:#}").contains("请手动下载"),
            "got: {error:#}"
        );
    }

    #[test]
    fn global_timeout_interrupts_continuously_dribbling_chunk_framing() {
        let policy = DownloadPolicy {
            connect_timeout: Duration::from_secs(1),
            idle_read_timeout: Duration::from_millis(500),
            http_timeout: Duration::from_millis(250),
            max_compressed: 1024,
            max_uncompressed: 4096,
        };
        let (result, elapsed) = stage_from_protocol_dribble(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n1;",
            b'a',
            40,
            b"\r\nx\r\n0\r\n\r\n",
            Duration::from_millis(50),
            policy,
        );
        let error = result.unwrap_err();
        assert!(elapsed < Duration::from_secs(1), "elapsed: {elapsed:?}");
        assert!(
            contains_io_kind(&error, io::ErrorKind::TimedOut),
            "got: {error:#}"
        );
        assert!(
            contains_ureq_timeout(&error, ureq::Timeout::Global),
            "got: {error:#}"
        );
        assert!(
            format!("{error:#}").contains("请手动下载"),
            "got: {error:#}"
        );
    }

    #[test]
    fn oversized_declared_length_is_rejected_before_body_reads() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 1025\r\nConnection: close\r\n\r\n";
        let error = stage_from_raw_response(response, policy_for_tests())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("Content-Length") && error.contains("1025") && error.contains("1024"),
            "got: {error}"
        );
        assert!(!error.contains("读取响应失败"), "got: {error}");
    }

    #[test]
    fn invalid_content_length_is_rejected_before_body_reads() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: nope\r\nConnection: close\r\n\r\n";
        let error = stage_from_raw_response(response, policy_for_tests())
            .unwrap_err()
            .to_string();
        let lower = error.to_ascii_lowercase();
        assert!(
            lower.contains("content-length")
                && (error.contains("无效") || lower.contains("not a number")),
            "got: {error}"
        );
        assert!(!error.contains("读取响应失败"), "got: {error}");
    }

    #[test]
    fn missing_content_length_uses_received_body_size() {
        let compressed = gzip(b"test");
        let sha = sha256_hex(&compressed);
        let mut response = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
        response.extend_from_slice(&compressed);
        stage_from_raw_response_with_sha(&response, &sha, policy_for_tests())
            .0
            .unwrap();
    }

    #[test]
    fn transfer_encoding_makes_content_length_non_authoritative() {
        let compressed = gzip(b"test");
        let sha = sha256_hex(&compressed);
        let response = chunked_response(&compressed, "Content-Length: 1\r\n");
        stage_from_raw_response_with_sha(&response, &sha, policy_for_tests())
            .0
            .unwrap();
    }

    #[test]
    fn chunked_body_is_rejected_at_compressed_limit_plus_one() {
        let body = vec![b'x'; 1025];
        let response = chunked_response(&body, "");
        let error = stage_from_raw_response(&response, policy_for_tests())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("下载数据超过限制") && error.contains("1024"),
            "got: {error}"
        );
    }

    #[test]
    fn idle_body_pause_reports_timeout_with_io_source() {
        let policy = DownloadPolicy {
            idle_read_timeout: Duration::from_millis(100),
            ..policy_for_tests()
        };
        let error = stage_from_pausing_server(Duration::from_millis(300), policy).unwrap_err();
        assert!(error.to_string().contains("读取响应失败"), "got: {error:#}");
        assert!(
            contains_io_kind(&error, io::ErrorKind::TimedOut),
            "got: {error:#}"
        );
        assert!(
            contains_ureq_timeout(&error, ureq::Timeout::RecvBody),
            "got: {error:#}"
        );
        let detail = format!("{error:#}");
        assert!(detail.contains("http://"), "got: {detail}");
        assert!(detail.contains("data.sqlite"), "got: {detail}");
        assert!(detail.contains("请手动下载"), "got: {detail}");
    }

    #[test]
    fn request_disables_transport_content_decoding() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        let (_, request) =
            stage_from_raw_response_with_sha(response, &"0".repeat(64), policy_for_tests());
        let request = String::from_utf8(request).unwrap();
        assert!(
            request
                .lines()
                .any(|line| line.eq_ignore_ascii_case("Accept-Encoding: identity")),
            "got request: {request}"
        );
    }

    #[test]
    fn content_encoding_cannot_bypass_wire_length_limit() {
        let encoded = gzip(b"x");
        let maximum = encoded.len() as u64 - 1;
        let mut response = format!(
            "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            encoded.len()
        )
        .into_bytes();
        response.extend_from_slice(&encoded);
        let policy = DownloadPolicy {
            max_compressed: maximum,
            ..policy_for_tests()
        };
        let error = stage_from_raw_response(&response, policy)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("Content-Length") && error.contains("超过"),
            "got: {error}"
        );
    }

    #[test]
    fn http_status_error_has_stable_download_context() {
        let response =
            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        let error = stage_from_raw_response(response, policy_for_tests())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("下载失败") && error.contains("请手动下载"),
            "got: {error}"
        );
    }

    #[test]
    fn total_deadline_is_checked_after_http_status_response() {
        let policy = DownloadPolicy {
            idle_read_timeout: Duration::from_millis(300),
            http_timeout: Duration::from_millis(100),
            ..policy_for_tests()
        };
        let response =
            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        let error = stage_from_delayed_raw_response(response, Duration::from_millis(150), policy)
            .unwrap_err();
        assert!(
            contains_io_kind(&error, io::ErrorKind::TimedOut),
            "got: {error:#}"
        );
    }

    #[test]
    fn header_idle_timeout_preserves_io_source() {
        let policy = DownloadPolicy {
            idle_read_timeout: Duration::from_millis(100),
            ..policy_for_tests()
        };
        let error = stage_from_pausing_headers(Duration::from_millis(300), policy).unwrap_err();
        assert!(error.to_string().contains("下载失败"), "got: {error:#}");
        assert!(
            contains_io_kind(&error, io::ErrorKind::TimedOut),
            "got: {error:#}"
        );
        assert!(
            contains_ureq_timeout(&error, ureq::Timeout::RecvResponse),
            "got: {error:#}"
        );
    }

    #[test]
    fn unpack_reads_all_concatenated_gzip_members() {
        let mut compressed = gzip(b"one");
        compressed.extend_from_slice(&gzip(b"two"));
        let mut unpacked = Vec::new();
        assert_eq!(
            unpack_gzip(Cursor::new(compressed), &mut unpacked, 6).unwrap(),
            6
        );
        assert_eq!(unpacked, b"onetwo");
    }

    #[test]
    fn stage_accepts_gzip_at_exact_uncompressed_limit() {
        let policy = DownloadPolicy {
            max_uncompressed: 4,
            ..policy_for_tests()
        };
        stage_compressed(&gzip(b"test"), policy).unwrap();
    }

    #[test]
    fn truncated_gzip_trailer_is_rejected_during_staging() {
        let mut compressed = gzip(b"test");
        compressed.truncate(compressed.len() - 4);
        let error = stage_compressed(&compressed, policy_for_tests())
            .unwrap_err()
            .to_string();
        assert!(error.contains("解压 gzip失败"), "got: {error}");
    }

    #[test]
    fn modified_gzip_crc_is_rejected_during_staging() {
        let mut compressed = gzip(b"test");
        let crc = compressed.len() - 8;
        compressed[crc] ^= 0xff;
        let error = stage_compressed(&compressed, policy_for_tests())
            .unwrap_err()
            .to_string();
        assert!(error.contains("解压 gzip失败"), "got: {error}");
    }

    #[test]
    fn trailing_non_gzip_bytes_are_rejected_during_staging() {
        let mut compressed = gzip(b"test");
        compressed.extend_from_slice(b"not-gzip");
        let error = stage_compressed(&compressed, policy_for_tests())
            .unwrap_err()
            .to_string();
        assert!(error.contains("解压 gzip失败"), "got: {error}");
    }

    #[test]
    fn bounded_copy_accepts_exact_limit_and_rejects_one_more() {
        let mut exact = Vec::new();
        assert_eq!(
            copy_bounded(Cursor::new(b"1234"), &mut exact, 4, "test").unwrap(),
            4
        );
        assert_eq!(exact, b"1234");

        let mut oversized = Vec::new();
        let error = copy_bounded(Cursor::new(b"12345"), &mut oversized, 4, "test")
            .unwrap_err()
            .to_string();
        assert!(error.contains("4"), "got: {error}");
    }

    #[test]
    fn unpack_accepts_exact_limit_and_rejects_expansion() {
        let gzip = |body: &[u8]| {
            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(body).unwrap();
            encoder.finish().unwrap()
        };

        let mut exact = Vec::new();
        unpack_gzip(Cursor::new(gzip(b"1234")), &mut exact, 4).unwrap();
        assert_eq!(exact, b"1234");

        let mut oversized = Vec::new();
        let error = unpack_gzip(Cursor::new(gzip(b"12345")), &mut oversized, 4)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("解压") && error.contains("4"),
            "got: {error}"
        );
    }
}
