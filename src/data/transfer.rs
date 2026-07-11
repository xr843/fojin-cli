use super::{download_notice, sibling_path, DataSource, Progress};
use anyhow::{anyhow, Context, Result};
use flate2::read::MultiGzDecoder;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

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
    let deadline = Instant::now()
        .checked_add(policy.http_timeout)
        .ok_or_else(|| anyhow!("HTTP 总超时配置溢出"))?;
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(policy.connect_timeout)
        .timeout_read(policy.idle_read_timeout)
        .build();
    let response = agent
        .get(source.url)
        .set("Accept-Encoding", "identity")
        .call();
    require_http_deadline(deadline)?;
    let response = response.map_err(|error| {
        anyhow!(
            "下载失败: {}: {error}\n请手动下载:\n  {}\n解压后放到: {}",
            source.url,
            source.url,
            live_path.display()
        )
    })?;
    let declared = declared_length(&response, policy.max_compressed)?;
    eprintln!("{}", download_notice(declared));

    let mut reader = response.into_reader();
    let mut progress = Progress::new(declared);
    let mut digest = Sha256::new();
    let mut received = 0_u64;
    let mut buffer = [0_u8; BUFFER_SIZE];
    loop {
        require_http_deadline(deadline)?;
        let count = reader.read(&mut buffer).context("读取响应失败")?;
        require_http_deadline(deadline)?;
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

fn require_http_deadline(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "HTTP total deadline timed out",
        ))
        .context("读取响应失败");
    }
    Ok(())
}

fn declared_length(response: &ureq::Response, maximum: u64) -> Result<Option<u64>> {
    if !response.all("Transfer-Encoding").is_empty() {
        return Ok(None);
    }
    let values = response.all("Content-Length");
    match values.as_slice() {
        [] => Ok(None),
        [value] => {
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

    fn policy_for_tests() -> DownloadPolicy {
        DownloadPolicy {
            connect_timeout: Duration::from_millis(200),
            idle_read_timeout: Duration::from_millis(200),
            http_timeout: Duration::from_secs(2),
            max_compressed: 1024,
            max_uncompressed: 4096,
        }
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

    fn contains_io_kind(error: &anyhow::Error, expected: io::ErrorKind) -> bool {
        error.chain().any(|cause| {
            cause
                .downcast_ref::<io::Error>()
                .is_some_and(|error| error.kind() == expected)
        })
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
    }

    #[test]
    fn total_timeout_stops_a_non_idle_dribble() {
        let policy = DownloadPolicy {
            connect_timeout: Duration::from_millis(200),
            idle_read_timeout: Duration::from_millis(150),
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
        assert!(
            error.contains("无效 Content-Length") && error.contains("nope"),
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
