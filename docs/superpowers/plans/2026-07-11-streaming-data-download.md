# Streaming Data Download Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the unbounded in-memory data download with a bounded two-stage disk pipeline and serialize data mutations so concurrent first runs perform exactly one download.

**Architecture:** A private `data::transfer` module streams the authenticated gzip asset to a unique sibling and then streams a `MultiGzDecoder` into a unique SQLite candidate. A private `data::operation_lock` module provides a persistent, time-bounded OS file lock used by ensure, update, and clean; `data.rs` retains SQLite validation and platform-specific atomic publication.

**Tech Stack:** Rust 2021 on Rust 1.95, ureq 3.3.0, sha2 0.10, flate2 1.1.9, rusqlite 0.40.1, standard-library `File::try_lock`, local `TcpListener` fixtures, GitHub Actions.

## Global Constraints

- Keep the MSRV at Rust 1.95 and add no locking dependency.
- Production limits are connect 30 seconds, idle read 60 seconds, total HTTP 15 minutes, lock wait 20 minutes, compressed 256 MiB, decompressed 768 MiB, and buffer 64 KiB.
- Send `Accept-Encoding: identity`; SHA-256 covers the downloaded release-asset bytes.
- First install and update both run schema/version, quick-check, and FTS integrity validation before publish.
- Existing data remains byte-for-byte unchanged on every failure before atomic publish.
- Ordinary queries never wait for the long-running mutation lock.
- Temporary files are same-directory, `create_new` siblings reserved and
  cleaned by one cooperating operation; this is not an adversarial same-user
  filesystem boundary.
- Do not add retries, resume support, mirrors, data discovery, schema changes, a tag, or a GitHub Release.
- Use TDD, focused commits, independent review, PR checks, and merge to `master`.

---

## File Map

- Create `src/data/transfer.rs`: download policy, strict response metadata, incremental hashing, bounded disk transfer, multi-member gzip decoding, and owned artifact cleanup.
- Create `src/data/operation_lock.rs`: permanent lock-file acquisition with bounded `try_lock` polling.
- Modify `src/data.rs`: module wiring, unified candidate validation/publication, lock integration, and clean API.
- Modify `src/cli.rs`: delegate `data clean` to the locked data-layer API.
- Modify `tests/data.rs`: end-to-end failure invariants and process-level concurrent installation.
- Modify `tests/command.rs`: clean/lock-file CLI behavior.
- Modify `README.md`: memory, disk, timeout, size, and concurrency contract.
- Modify `CHANGELOG.md`: unreleased bounded-transfer and concurrency entries.

---

### Task 1: Build and adopt the bounded two-stage transfer pipeline

**Files:**
- Create: `src/data/transfer.rs`
- Modify: `src/data.rs:1-205`
- Modify: `src/data.rs:419-445`
- Test: `src/data/transfer.rs`
- Test: `tests/data.rs:108-166`
- Test: `tests/data.rs:430-570`

**Interfaces:**
- Consumes: `super::DataSource`, `super::Progress`, `super::download_notice`, `super::sibling_path`, and the existing replacement/verification functions in `data.rs`.
- Produces: `transfer::DownloadPolicy`, `transfer::PRODUCTION_POLICY`, `transfer::StagedCandidate`, and `transfer::stage_candidate(&Path, &DataSource, DownloadPolicy) -> Result<StagedCandidate>`.

- [ ] **Step 1: Add failing bounded-transfer tests**

Create `src/data/transfer.rs`, wire it with `mod transfer;` in `src/data.rs`, and start with unit tests that specify exact-limit and over-limit behavior:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::{Cursor, Write};

    #[test]
    fn bounded_copy_accepts_exact_limit_and_rejects_one_more() {
        let mut exact = Vec::new();
        assert_eq!(copy_bounded(Cursor::new(b"1234"), &mut exact, 4, "test").unwrap(), 4);
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
        assert!(error.contains("解压") && error.contains("4"), "got: {error}");
    }
}
```

In `tests/data.rs`, change `ensure_data_downloads_verifies_and_unpacks` to
serve `gzip_bytes(&replacement_database_bytes())` and finish by calling
`verify_dataset_file(&path)`. Add `first_install_rejects_incompatible_database`
using the original `b"fake sqlite payload"`; its SHA is valid, but
`ensure_data` must return a dataset-incompatibility error, leave `path`
absent, and leave no owned candidate artifacts. This integration test fails
against the current weak first-install path.

- [ ] **Step 2: Run the new tests and confirm RED**

Run:

```bash
cargo +1.95.0 test --lib data::transfer::tests --locked
```

Expected: compilation fails because `copy_bounded` and `unpack_gzip` do not exist.

- [ ] **Step 3: Implement policy, bounded copy, gzip handling, and owned artifacts**

Add these concrete interfaces to `src/data/transfer.rs`:

```rust
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
            Err(error) => Err(error)
                .with_context(|| format!("删除压缩临时文件失败: {}", self.path.display())),
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
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn publish_succeeded(mut self) {
        self.armed = false;
    }

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
                return Err(error)
                    .with_context(|| format!("创建候选数据失败: {}", path.display()));
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
    let copied = io::copy(&mut limited, &mut writer)
        .with_context(|| format!("{label}失败"))?;
    if copied > maximum {
        return Err(anyhow!("{label}超过限制: 最大 {maximum} 字节"));
    }
    Ok(copied)
}

fn unpack_gzip(reader: impl Read, writer: impl Write, maximum: u64) -> Result<u64> {
    let decoder = MultiGzDecoder::new(reader);
    copy_bounded(decoder, writer, maximum, "解压 gzip")
}
```

The sequence is consumed before each open attempt; an `AlreadyExists` retry
therefore always receives a new name and cannot loop on a stale artifact.

- [ ] **Step 4: Implement streaming download and candidate staging**

Add `stage_candidate` and keep the HTTP reader/file loop explicit so SHA,
progress, and the compressed limit share one byte count:

```rust
pub(super) fn stage_candidate(
    live_path: &Path,
    source: &DataSource<'_>,
    policy: DownloadPolicy,
) -> Result<StagedCandidate> {
    let (compressed_guard, mut compressed_file) = create_compressed_artifact(live_path)?;
    let staged = stage_candidate_inner(
        live_path,
        source,
        policy,
        &mut compressed_file,
    );
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

    let (candidate_guard, mut candidate_file) =
        create_candidate_artifact(live_path)?;
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
```

Define the Task 1 metadata and digest helpers as:

```rust
fn basic_declared_length(response: &ureq::Response, maximum: u64) -> Result<Option<u64>> {
    let Some(value) = response.header("Content-Length") else {
        return Ok(None);
    };
    let parsed = value
        .parse::<u64>()
        .with_context(|| format!("无效 Content-Length: {value}"))?;
    if parsed > maximum {
        return Err(anyhow!(
            "Content-Length 超过下载限制: {parsed} > {maximum}"
        ));
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
```

Task 2 replaces the single-value lookup with strict duplicate and transfer-
encoding handling while retaining the latter two helpers unchanged.

- [ ] **Step 5: Replace the Vec pipeline with unified candidate publication**

In `src/data.rs`, delete production use of `download_and_unpack`, `http_get`,
and fixed-name `write_atomic`. Keep the public small-byte helpers for source
compatibility. Add:

```rust
mod transfer;

fn install_candidate(path: &Path, source: &DataSource<'_>) -> Result<()> {
    let candidate =
        transfer::stage_candidate(path, source, transfer::PRODUCTION_POLICY)?;
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
```

Change `finish_replacement` to consume `StagedCandidate`. On success call
`publish_succeeded`; on remove-policy failure call `cleanup_with`; on
preserve-policy failure call `preserve` and retain the exact path in context.
Make both `ensure_data` and `update_data` call `install_candidate`, so first
installation receives full SQLite/FTS verification.

- [ ] **Step 6: Run targeted and regression tests**

Run:

```bash
cargo +1.95.0 test --lib data::transfer::tests --locked
cargo +1.95.0 test --test data --locked
cargo +1.95.0 test --all --locked
```

Expected: all tests pass; the Rust total is at least 102 after the two new
bounded-transfer tests.

- [ ] **Step 7: Commit the bounded pipeline**

```bash
git add src/data.rs src/data/transfer.rs tests/data.rs
git commit -m "fix(data): stream verified downloads to disk"
```

---

### Task 2: Enforce strict HTTP metadata and timeout behavior

**Files:**
- Modify: `src/data/transfer.rs`
- Test: `src/data/transfer.rs`

**Interfaces:**
- Consumes: `DownloadPolicy` and `stage_candidate` from Task 1.
- Produces: strict `declared_length(&ureq::Response, u64) -> Result<Option<u64>>` and stable timeout/HTTP error contexts used by all transfers.

- [ ] **Step 1: Add failing raw-HTTP tests**

Add a reusable one-response `TcpListener` fixture inside the transfer test
module and tests with small injected policies for:

```rust
#[test]
fn duplicate_content_length_is_rejected() {
    let response = b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nContent-Length: 4\r\nConnection: close\r\n\r\ntest";
    let error = stage_from_raw_response(response, policy_for_tests())
        .unwrap_err()
        .to_string();
    assert!(error.contains("Content-Length") && error.contains("重复"), "got: {error}");
}

#[test]
fn declared_length_must_match_received_body() {
    let response = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\ntest";
    let error = stage_from_raw_response(response, policy_for_tests())
        .unwrap_err()
        .to_string();
    assert!(error.contains("长度") || error.contains("读取响应失败"), "got: {error}");
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
    let error = stage_from_dribbling_server(Duration::from_millis(50), policy)
        .unwrap_err()
        .to_string();
    assert!(error.contains("读取响应失败") || error.contains("timed out"), "got: {error}");
}
```

Define the test helpers in the same module so no external network is used:

```rust
fn policy_for_tests() -> DownloadPolicy {
    DownloadPolicy {
        connect_timeout: Duration::from_millis(200),
        idle_read_timeout: Duration::from_millis(200),
        http_timeout: Duration::from_secs(2),
        max_compressed: 1024,
        max_uncompressed: 4096,
    }
}

fn stage_from_raw_response(response: &[u8], policy: DownloadPolicy) -> Result<()> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let response = response.to_vec();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let _ = stream.read(&mut request);
        stream.write_all(&response).unwrap();
    });
    let directory = tempfile::tempdir().unwrap();
    let live = directory.path().join("data.sqlite");
    let url = format!("http://{address}/data.gz");
    let sha = "0".repeat(64);
    let result = stage_candidate(&live, &DataSource { url: &url, sha256: &sha }, policy)
        .map(drop);
    server.join().unwrap();
    result
}

fn stage_from_dribbling_server(interval: Duration, policy: DownloadPolicy) -> Result<()> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let _ = stream.read(&mut request);
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
    let result = stage_candidate(&live, &DataSource { url: &url, sha256: &sha }, policy)
        .map(drop);
    server.join().unwrap();
    result
}
```

Also add tests for an oversized declared length rejected before body reads,
missing length, chunked `max + 1`, an idle body pause, and capture the request
to assert `Accept-Encoding: identity`.

- [ ] **Step 2: Run the strict HTTP tests and confirm RED**

Run:

```bash
cargo +1.95.0 test --lib data::transfer::tests --locked
```

Expected: duplicate-length, chunked-limit, and timeout assertions fail against
the basic Task 1 metadata implementation.

- [ ] **Step 3: Implement strict response metadata**

Replace `basic_declared_length` with:

```rust
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
                return Err(anyhow!(
                    "Content-Length 超过下载限制: {parsed} > {maximum}"
                ));
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
```

Keep the actual compressed counter authoritative for missing/chunked lengths.
Map body `TimedOut` and `UnexpectedEof` without discarding their source error,
so tests and users can distinguish timeout from truncation.

- [ ] **Step 4: Add gzip integrity edge tests**

Add tests for two concatenated gzip members, a truncated trailer, modified
CRC, and trailing non-gzip bytes. Use `MultiGzDecoder`; exact-limit valid data
must pass because the reader probes `maximum + 1`, while each malformed input
must fail before a candidate can be published.

- [ ] **Step 5: Run transfer tests, Clippy, and formatting**

Run:

```bash
cargo +1.95.0 fmt --all -- --check
cargo +1.95.0 clippy --all-targets --locked -- -D warnings
cargo +1.95.0 test --lib data::transfer::tests --locked
cargo +1.95.0 test --test data --locked
```

Expected: all commands exit 0.

- [ ] **Step 6: Commit strict protocol handling**

```bash
git add src/data/transfer.rs
git commit -m "fix(data): bound HTTP metadata and deadlines"
```

---

### Task 3: Add the single-flight operation lock and locked clean path

**Files:**
- Create: `src/data/operation_lock.rs`
- Modify: `src/data.rs:1-150`
- Modify: `src/cli.rs:332-351`
- Test: `src/data/operation_lock.rs`
- Test: `tests/data.rs`
- Test: `tests/command.rs:240-283`

**Interfaces:**
- Consumes: `super::sibling_path`, `install_candidate`, and Task 1 artifact names.
- Produces: `operation_lock::acquire(&Path, Duration) -> Result<OperationLock>` and public `data::clean_data(&Path) -> Result<Option<u64>>`.

- [ ] **Step 1: Add failing lock lifecycle tests**

Create `src/data/operation_lock.rs`, wire `mod operation_lock;`, and add:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn competing_lock_times_out_then_succeeds_after_drop() {
        let directory = tempfile::tempdir().unwrap();
        let data = directory.path().join("data.sqlite");
        let first = acquire(&data, Duration::from_millis(100)).unwrap();
        let error = acquire(&data, Duration::from_millis(20))
            .unwrap_err()
            .to_string();
        assert!(error.contains("等待") && error.contains("超时"), "got: {error}");
        drop(first);
        acquire(&data, Duration::from_millis(100)).unwrap();
        assert!(data.with_file_name("data.sqlite.lock").exists());
    }
}
```

- [ ] **Step 2: Run the lock test and confirm RED**

Run:

```bash
cargo +1.95.0 test --lib data::operation_lock::tests --locked
```

Expected: compilation fails because `acquire` and `OperationLock` do not exist.

- [ ] **Step 3: Implement the permanent bounded-wait lock**

Implement `src/data/operation_lock.rs` as:

```rust
use super::sibling_path;
use anyhow::{anyhow, Context, Result};
use std::fs::{File, OpenOptions};
use std::io::ErrorKind;
use std::path::Path;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(100);
pub(super) const LOCK_WAIT_TIMEOUT: Duration = Duration::from_secs(20 * 60);

#[derive(Debug)]
pub(super) struct OperationLock {
    _file: File,
}

pub(super) fn acquire(data_path: &Path, timeout: Duration) -> Result<OperationLock> {
    let lock_path = sibling_path(data_path, ".lock")?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&lock_path)
        .with_context(|| format!("打开数据操作锁失败: {}", lock_path.display()))?;
    let started = Instant::now();
    let mut reported_wait = false;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(OperationLock { _file: file }),
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if started.elapsed() >= timeout {
                    return Err(anyhow!(
                        "等待其他 fojin 数据操作超时: {}",
                        lock_path.display()
                    ));
                }
                if !reported_wait {
                    eprintln!("检测到另一个 fojin 数据操作,正在等待...");
                    reported_wait = true;
                }
                let remaining = timeout.saturating_sub(started.elapsed());
                std::thread::sleep(POLL_INTERVAL.min(remaining));
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("获取数据操作锁失败: {}", lock_path.display()));
            }
        }
    }
}
```

Do not unlink the lock file. Dropping `OperationLock` drops `File` and releases
the OS lock after success, error, unwind, or process exit.

- [ ] **Step 4: Lock ensure, update, and clean with a second check**

Change `ensure_data` to preserve the no-op and offline fast paths, then acquire
the lock and check existence again:

```rust
pub fn ensure_data(path: &Path, offline: bool, source: &DataSource<'_>) -> Result<()> {
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
```

Acquire the same lock around all of `update_data`. Add:

```rust
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
        std::fs::remove_file(path)
            .with_context(|| format!("删除数据失败: {}", path.display()))?;
    }
    Ok(size)
}
```

Implement the locked sweep as:

```rust
pub(super) fn remove_known_artifacts(live_path: &Path) -> Result<()> {
    let legacy = live_path.with_extension("tmp");
    match std::fs::remove_file(&legacy) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("删除旧临时数据失败: {}", legacy.display()));
        }
    }

    let directory = live_path
        .parent()
        .ok_or_else(|| anyhow!("数据路径没有父目录: {}", live_path.display()))?;
    let file_name = live_path
        .file_name()
        .ok_or_else(|| anyhow!("数据路径没有文件名: {}", live_path.display()))?
        .to_string_lossy();
    let download_prefix = format!("{file_name}.download.");
    let candidate_prefix = format!("{file_name}.candidate.");
    for entry in std::fs::read_dir(directory)
        .with_context(|| format!("读取数据目录失败: {}", directory.display()))?
    {
        let entry = entry.context("读取数据目录项失败")?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(&download_prefix) && !name.starts_with(&candidate_prefix) {
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
```

This code does not traverse directories or delete the `.lock` sibling.
Change `run_data(DataAction::Clean)` in `src/cli.rs` to print from
`data::clean_data` and leave `data.sqlite.lock` untouched.

- [ ] **Step 5: Add the process-level single-flight test**

In `tests/data.rs`, make one test act as either the parent or a re-executed
worker. This avoids adding a normal test that exits without assertions:

```rust
#[test]
fn concurrent_first_install_downloads_once() {
    if std::env::var_os("FOJIN_CONCURRENT_WORKER").is_some() {
        let path = PathBuf::from(std::env::var_os("FOJIN_WORKER_DATA").unwrap());
        let url = std::env::var("FOJIN_WORKER_URL").unwrap();
        let sha = std::env::var("FOJIN_WORKER_SHA256").unwrap();
        ensure_data(&path, false, &DataSource { url: &url, sha256: &sha }).unwrap();
        return;
    }
    use std::io::{Read, Write};
    use std::process::Command;
    use std::time::{Duration, Instant};

    let directory = tempfile::tempdir().unwrap();
    let data_path = directory.path().join("data.sqlite");
    let body = gzip_bytes(&replacement_database_bytes());
    let sha = sha256_hex(&body);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/data.gz", listener.local_addr().unwrap());
    let server_body = body.clone();
    let server = std::thread::spawn(move || {
        let mut requests = 0_usize;
        let (mut first, _) = listener.accept().unwrap();
        requests += 1;
        let mut request = [0_u8; 4096];
        let _ = first.read(&mut request);
        write!(
            first,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            server_body.len()
        )
        .unwrap();
        let midpoint = server_body.len() / 2;
        first.write_all(&server_body[..midpoint]).unwrap();
        first.flush().unwrap();
        std::thread::sleep(Duration::from_millis(200));
        first.write_all(&server_body[midpoint..]).unwrap();
        drop(first);

        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    requests += 1;
                    let _ = stream.read(&mut request);
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        server_body.len()
                    )
                    .unwrap();
                    stream.write_all(&server_body).unwrap();
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("accept failed: {error}"),
            }
        }
        requests
    });

    let spawn_worker = || {
        Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("concurrent_first_install_downloads_once")
            .arg("--nocapture")
            .env("FOJIN_CONCURRENT_WORKER", "1")
            .env("FOJIN_WORKER_DATA", &data_path)
            .env("FOJIN_WORKER_URL", &url)
            .env("FOJIN_WORKER_SHA256", &sha)
            .spawn()
            .unwrap()
    };
    let mut first = spawn_worker();
    let mut second = spawn_worker();
    assert!(first.wait().unwrap().success());
    assert!(second.wait().unwrap().success());
    assert_eq!(server.join().unwrap(), 1);
    verify_dataset_file(&data_path).unwrap();
    assert_no_owned_candidate_artifacts(&data_path);
}
```

- [ ] **Step 6: Add clean and lock regression tests**

In `tests/command.rs`, extend `data_clean_removes_file_and_is_idempotent` to
assert `data.sqlite.lock` remains a regular file after both clean calls. Add a
data-layer test showing an existing live file stays readable while an updater
is downloading; do not make query connections acquire the operation lock.

- [ ] **Step 7: Run all lock, data, command, and Windows-compilable tests**

Run:

```bash
cargo +1.95.0 fmt --all -- --check
cargo +1.95.0 clippy --all-targets --locked -- -D warnings
cargo +1.95.0 test --lib data::operation_lock::tests --locked
cargo +1.95.0 test --test data --locked
cargo +1.95.0 test --test command --locked
cargo +1.95.0 test --all --locked
```

Expected: all commands exit 0, the process-level server reports one request,
and no test waits for a production-scale timeout.

- [ ] **Step 8: Commit single-flight locking**

```bash
git add src/data.rs src/data/operation_lock.rs src/data/transfer.rs src/cli.rs tests/data.rs tests/command.rs
git commit -m "fix(data): serialize installation and cleanup"
```

---

### Task 4: Document the resource contract and run release-grade verification

**Files:**
- Modify: `README.md:155-190`
- Modify: `README.md:330-365`
- Modify: `CHANGELOG.md:5-20`
- Test: all repository test and release-contract files

**Interfaces:**
- Consumes: the final constants and behavior from Tasks 1-3.
- Produces: user-facing operational documentation and a verified merge-ready branch.

- [ ] **Step 1: Update the Chinese and English data documentation**

Document these exact facts in both README language sections:

- transfers no longer buffer the complete archive or database in memory;
- compressed responses are capped at 256 MiB and decompressed databases at
  768 MiB;
- updates can temporarily require the live database plus roughly 744 MiB of
  staging disk;
- connect, idle, and total HTTP timeouts are 30 seconds, 60 seconds, and 15
  minutes;
- concurrent operations on one data directory are single-flight and a waiter
  may wait up to 20 minutes;
- the permanent `.lock` file is harmless and intentionally survives clean;
- offline queries and the pinned data-v1 checksum contract are unchanged.

- [ ] **Step 2: Update CHANGELOG**

Under `[0.3.0] - Unreleased`, add bullets stating that the data path now uses a
bounded disk-streamed, checksum-first pipeline and that concurrent install,
update, and clean operations are serialized with full candidate validation.

- [ ] **Step 3: Verify no obsolete production path remains**

Run:

```bash
rg -n "download_and_unpack|fn http_get|let raw =" src
```

Expected: no matches. The small public `gunzip`, `verify_sha256`, and
`write_atomic` compatibility helpers may remain, but neither `ensure_data` nor
`update_data` may call them.

- [ ] **Step 4: Run the complete local verification matrix**

Run:

```bash
git diff --check
cargo +1.95.0 fmt --all -- --check
cargo +1.95.0 clippy --all-targets --locked -- -D warnings
cargo +1.95.0 test --all --locked
cargo +1.95.0 build --release --locked
cargo +1.95.0 package --locked
python3 -m pytest data-pipeline/tests -q
bash tests/release-scripts.sh
bash tests/install-script.sh
```

Expected: every command exits 0; Rust tests include the new transfer and
concurrency cases, Python reports one pass, and both shell suites report their
success messages.

- [ ] **Step 5: Run static shell/workflow validation when available**

Run ShellCheck 0.10.0 over `install.sh`, `scripts/*.sh`, and `tests/*.sh`, and
actionlint 1.7.7 over `.github/workflows/*.yml`. Expected: exit 0 with no
diagnostics. If the binaries are absent, use the same checksum-verified pinned
downloads established in the preceding v0.3.0 work.

- [ ] **Step 6: Commit documentation**

```bash
git add README.md CHANGELOG.md
git commit -m "docs: describe bounded data downloads"
```

---

### Task 5: Independent review, PR, CI, and merge

**Files:**
- Review: `origin/master...HEAD`
- GitHub: branch `agent/streaming-data-download`

**Interfaces:**
- Consumes: all implementation commits and fresh verification evidence.
- Produces: a merged GitHub PR and synchronized clean local `master`.

- [ ] **Step 1: Request independent spec and code-quality review**

Give reviewers the approved design, this plan, the commit range, and test
evidence. Require explicit Critical/Important/Minor findings and a ready-to-
merge verdict. Address substantive findings with TDD and focused fix commits.

- [ ] **Step 2: Re-run verification after the final review fix**

At minimum rerun `git diff --check`, fmt, Clippy, all locked Rust tests, Python,
release scripts, and installer scripts. Expected: all exit 0 on the final HEAD.

- [ ] **Step 3: Push and create a ready pull request**

```bash
git push -u origin agent/streaming-data-download
gh pr create --base master --head agent/streaming-data-download \
  --title "fix(data): stream and serialize dataset installation" \
  --body-file .superpowers/sdd/pr-body.md
```

The PR body summarizes bounded memory, limits/timeouts, full first-install
validation, single-flight concurrency, documentation, and exact test evidence.

- [ ] **Step 4: Wait for every GitHub check and fix failures systematically**

Run `gh pr checks <number> --watch --interval 10`. Do not merge with a pending,
cancelled, or failing check. For a failure, inspect the job logs, reproduce it,
apply one root-cause fix, and rerun local verification before pushing.

- [ ] **Step 5: Merge and verify master**

Use a merge commit, delete the remote feature branch, fast-forward the main
checkout, wait for the post-merge `master` CI run, rerun the core local tests,
and remove the isolated worktree/local feature branch. Verify no tag or GitHub
Release was created.
