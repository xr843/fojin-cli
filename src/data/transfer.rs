use super::{download_notice, sibling_path, DataSource, Progress};
use anyhow::{anyhow, Context, Result};
use flate2::read::MultiGzDecoder;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const MIB: u64 = 1024 * 1024;
const BUFFER_SIZE: usize = 64 * 1024;
static ARTIFACT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((StagedCandidate { path, armed: true }, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("创建候选数据失败: {}", path.display()));
            }
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
            compressed_guard.remove_now()?;
            Ok(candidate)
        }
        Err(error) => Err(compressed_guard.cleanup_with(error)),
    }
}

fn stage_candidate_inner(
    live_path: &Path,
    source: &DataSource<'_>,
    policy: DownloadPolicy,
    compressed_file: &mut File,
) -> Result<StagedCandidate> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(policy.connect_timeout)
        .timeout_read(policy.idle_read_timeout)
        .timeout(policy.http_timeout)
        .build();
    let response = agent
        .get(source.url)
        .set("Accept-Encoding", "identity")
        .call()
        .map_err(|error| {
            anyhow!(
                "下载失败: {}: {error}\n请手动下载:\n  {}\n解压后放到: {}",
                source.url,
                source.url,
                live_path.display()
            )
        })?;
    let declared = basic_declared_length(&response, policy.max_compressed)?;
    eprintln!("{}", download_notice(declared));

    let mut reader = response.into_reader();
    let mut progress = Progress::new(declared);
    let mut digest = Sha256::new();
    let mut received = 0_u64;
    let mut buffer = [0_u8; BUFFER_SIZE];
    loop {
        let count = reader.read(&mut buffer).context("读取响应失败")?;
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
    require_declared_length(declared, received)?;
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

fn basic_declared_length(response: &ureq::Response, maximum: u64) -> Result<Option<u64>> {
    let Some(value) = response.header("Content-Length") else {
        return Ok(None);
    };
    let parsed = value
        .parse::<u64>()
        .with_context(|| format!("无效 Content-Length: {value}"))?;
    if parsed > maximum {
        return Err(anyhow!("Content-Length 超过下载限制: {parsed} > {maximum}"));
    }
    Ok(Some(parsed))
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
    use std::io::{Cursor, Write};

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
