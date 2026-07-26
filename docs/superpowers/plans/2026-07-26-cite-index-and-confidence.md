# cite 索引与 confidence 显示 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给 `cite` 消除全表扫描(2.34 s → ~0 ms),并让恒为 1.0 的 `confidence` 不再污染每一行输出。

**Architecture:** 索引是对**已下载产物**的本地优化,不进 `schema.sql`(那是兼容性参照物,且被 Python 导出管线执行)。两条创建路径:安装/更新时在候选文件上建好;`cite` 发现缺失时懒建一次。任何失败都静默退回全表扫描。`confidence` 只改渲染,JSON 与排序逻辑一字不动。

**Tech Stack:** Rust 2021、rusqlite 0.40.1(bundled SQLite)、anyhow。**不新增依赖。**

**依据 spec:** `docs/superpowers/specs/2026-07-26-index-and-confidence-design.md`

## Global Constraints

- **MSRV 1.95。本机默认 stable 是 1.88,低于 MSRV,所有 cargo 命令必须写成 `rustup run 1.96.0 cargo ...`**,否则以 "requires rustc 1.95" 失败。MSRV 校验用 `rustup run 1.95.0 cargo test --all --locked`。
- 不新增 Cargo 依赖,不改 `Cargo.toml`。
- 所有面向用户的字符串用中文,与既有输出风格一致。
- 退出码契约不变:`0` 成功、`1` 运行期错误、`2` 用法错误。
- `--json` 时 stdout 必须是纯 JSON;提示走 stderr。
- **索引的存在与否绝不改变任何查询结果,只改变速度。**
- **索引构建的任何失败都不得让查询失败**,一律静默退回全表扫描。
- `cargo clippy --all-targets -- -D warnings` 无警告,`cargo fmt` 已格式化。
- 全量测试基线 **213 全绿**,只应变多不应变红。

## 文件结构

| 文件 | 本轮职责 |
| --- | --- |
| `src/data/operation_lock.rs` | 新增 `try_acquire`:单次尝试、不等待、不打印 |
| `src/data.rs` | 索引 DDL 常量、`ensure_cbeta_index`(懒建)、安装路径建索引 |
| `src/cli.rs` | `Cite` 分支调用 `ensure_cbeta_index` |
| `src/render.rs` | `conf_tag` 改为显示精度为 `1.00` 时不输出标签 |
| `README.md` / `CHANGELOG.md` | 首屏示例去掉 `[MITRA 1.00]`;说明首次 `cite` 的一次性停顿 |

---

### Task 1: `operation_lock::try_acquire`

**Files:**
- Modify: `src/data/operation_lock.rs`

**Interfaces:**
- Produces: `pub(super) fn try_acquire(data_path: &Path) -> Result<OperationLock>` —— 单次 `try_lock`,锁被占用时立即返回 `Err`,**不等待、不向 stderr 打印任何内容**。

现有的 `acquire` 会最长等待 20 分钟,并在首次等待时打印"检测到另一个 fojin 数据操作,正在等待..."。这两条对查询路径都是错的:用户只是想查一部经,不该因为后台在更新数据而卡住或看到无关提示。

- [ ] **Step 1: 写失败测试**

在 `src/data/operation_lock.rs` 的 `mod tests` 中追加:

```rust
    #[test]
    fn try_acquire_returns_immediately_when_held() {
        let directory = tempfile::tempdir().unwrap();
        let data = directory.path().join("data.sqlite");
        let held = acquire(&data, Duration::from_millis(100)).unwrap();

        let started = Instant::now();
        let result = try_acquire(&data);
        let waited = started.elapsed();

        assert!(result.is_err(), "must not acquire a held lock");
        assert!(
            waited < Duration::from_millis(50),
            "try_acquire must not wait; waited {waited:?}"
        );

        drop(held);
        assert!(try_acquire(&data).is_ok(), "must acquire once released");
    }
```

- [ ] **Step 2: 运行,确认失败**

Run: `rustup run 1.96.0 cargo test --lib operation_lock`
Expected: FAIL,`cannot find function try_acquire in this scope`。

- [ ] **Step 3: 实现**

把 `acquire` 中打开锁文件的那段抽成共享函数,并新增 `try_acquire`。将 `acquire` 的开头替换为调用共享函数:

```rust
fn open_lock_file(data_path: &Path) -> Result<(File, PathBuf)> {
    let lock_path = sibling_path(data_path, ".lock")?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("打开数据操作锁失败: {}", lock_path.display()))?;
    Ok((file, lock_path))
}

/// Single attempt, no waiting and no stderr output. The query path uses this:
/// a user running `cite` should never block on a background data operation,
/// nor see a message about one.
pub(super) fn try_acquire(data_path: &Path) -> Result<OperationLock> {
    let (file, lock_path) = open_lock_file(data_path)?;
    match file.try_lock() {
        Ok(()) => Ok(OperationLock { _file: file }),
        Err(TryLockError::WouldBlock) => {
            Err(anyhow!("数据操作锁被占用: {}", lock_path.display()))
        }
        Err(TryLockError::Error(error)) => Err(error)
            .with_context(|| format!("获取数据操作锁失败: {}", lock_path.display())),
    }
}

pub(super) fn acquire(data_path: &Path, timeout: Duration) -> Result<OperationLock> {
    let (file, lock_path) = open_lock_file(data_path)?;
    let started = Instant::now();
    let mut reported_wait = false;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(OperationLock { _file: file }),
            Err(TryLockError::WouldBlock) => {
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
            Err(TryLockError::Error(error)) => {
                return Err(error)
                    .with_context(|| format!("获取数据操作锁失败: {}", lock_path.display()));
            }
        }
    }
}
```

文件顶部的 `use` 需要 `PathBuf`:

```rust
use std::path::{Path, PathBuf};
```

- [ ] **Step 4: 运行,确认通过**

Run: `rustup run 1.96.0 cargo test --lib operation_lock`
Expected: 2 passed(既有的 `competing_lock_times_out_then_succeeds_after_drop` 与新增的)。

- [ ] **Step 5: 全量 + clippy,提交**

Run: `rustup run 1.96.0 cargo test && rustup run 1.96.0 cargo clippy --all-targets -- -D warnings`

```bash
git add src/data/operation_lock.rs
git commit -m "feat(data): 新增不等待、不打印的 try_acquire 供查询路径使用"
```

---

### Task 2: `cite` 缺失时懒建索引

**Files:**
- Modify: `src/data.rs`
- Modify: `src/cli.rs`
- Test: `tests/command.rs`(追加)

**Interfaces:**
- Consumes: `operation_lock::try_acquire`(Task 1)
- Produces:
  - `pub const CBETA_INDEX_NAME: &str = "idx_parallels_cbeta"`
  - `pub fn ensure_cbeta_index(conn: &rusqlite::Connection)` —— 尽力而为,无返回值,永不 panic、永不传播错误
  - `fn create_cbeta_index(conn: &rusqlite::Connection) -> rusqlite::Result<()>`(私有,Task 3 复用)

**关键实现顺序:先以读写方式打开,成功后才打印提示。** 打开读写是最可靠且最便宜的可写性判据(SQLite 需要**所在目录**可写以创建回滚日志,光看文件权限不够)。把打开放在打印之前,只读安装就**永远不会**看到任何提示,而能建索引的用户能看到那 1–2 秒停顿的解释。

- [ ] **Step 1: 写失败测试**

追加到 `tests/command.rs`:

```rust
fn write_cite_fixture(dir: &std::path::Path) {
    let db_path = dir.join("data.sqlite");
    let conn = Connection::open(db_path).unwrap();
    init_schema(&conn).unwrap();
    for (k, v) in [("version", "v1"), ("norm_ruleset", "t2s-char-1to1-v1")] {
        conn.execute(
            "INSERT INTO meta(key,value) VALUES (?1,?2)",
            rusqlite::params![k, v],
        )
        .unwrap();
    }
    for (zt, zn, lang, f, juan) in [
        ("色即是空", "色即是空", "sa", "rūpaṃ śūnyatā", 1),
        ("受想行識", "受想行识", "bo", "gzugs stong pa", 2),
    ] {
        conn.execute(
            "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
             VALUES (?1,?2,?3,?4,0.9,'T0251','心經',?5)",
            rusqlite::params![zt, zn, lang, f, juan],
        )
        .unwrap();
    }
}

fn index_exists(dir: &std::path::Path) -> bool {
    let conn = Connection::open(dir.join("data.sqlite")).unwrap();
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_parallels_cbeta'",
        [],
        |row| row.get::<_, i64>(0),
    )
    .unwrap()
        > 0
}

#[test]
fn cite_builds_the_missing_index_and_output_is_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    write_cite_fixture(dir.path());
    assert!(!index_exists(dir.path()), "fixture must start without the index");

    let first = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args(["cite", "T0251", "--offline", "--data-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(first.status.code(), Some(0));
    assert!(index_exists(dir.path()), "cite must have built the index");

    // Second run: index already present, so no rebuild and no notice. This is
    // also the idempotency check — `ensure_cbeta_index` returns before it even
    // opens the file read-write once the index exists.
    let second = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args(["cite", "T0251", "--offline", "--data-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(first.stdout).unwrap(),
        String::from_utf8(second.stdout).unwrap()
    );
    assert!(
        !String::from_utf8(second.stderr).unwrap().contains("建立索引"),
        "the notice must appear only when actually building"
    );
}

#[test]
fn a_held_lock_skips_the_build_and_leaves_results_identical() {
    // Holding the operation lock forces `cite` down the un-indexed path, which
    // is the only portable way to compare indexed vs un-indexed output — the
    // first run of the previous test already has the index by the time it
    // queries, so it cannot make this comparison.
    let indexed_dir = tempfile::tempdir().unwrap();
    write_cite_fixture(indexed_dir.path());
    let indexed = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args(["cite", "T0251", "--offline", "--data-dir"])
        .arg(indexed_dir.path())
        .output()
        .unwrap();
    assert!(index_exists(indexed_dir.path()));

    let plain_dir = tempfile::tempdir().unwrap();
    write_cite_fixture(plain_dir.path());
    let lock_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(plain_dir.path().join("data.sqlite.lock"))
        .unwrap();
    lock_file.try_lock().expect("test must own the lock");

    let plain = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args(["cite", "T0251", "--offline", "--data-dir"])
        .arg(plain_dir.path())
        .output()
        .unwrap();
    drop(lock_file);

    assert_eq!(plain.status.code(), Some(0), "a busy lock must not fail the query");
    assert!(
        !index_exists(plain_dir.path()),
        "a busy lock must skip the build rather than wait for it"
    );
    assert_eq!(
        String::from_utf8(indexed.stdout).unwrap(),
        String::from_utf8(plain.stdout).unwrap(),
        "the index changes speed only — never results"
    );
}

#[test]
fn parallel_does_not_build_the_cite_index() {
    let dir = tempfile::tempdir().unwrap();
    write_cite_fixture(dir.path());

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args(["parallel", "色即是空", "--offline", "--data-dir"])
        .arg(dir.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    assert!(
        !index_exists(dir.path()),
        "only the cite path needs this index; parallel must not pay for it"
    );
}

#[cfg(unix)]
#[test]
fn cite_degrades_gracefully_when_the_data_dir_is_read_only() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    write_cite_fixture(dir.path());
    let original = std::fs::metadata(dir.path()).unwrap().permissions();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args(["cite", "T0251", "--offline", "--data-dir"])
        .arg(dir.path())
        .output()
        .unwrap();

    // Restore before asserting so a failure still leaves a removable tempdir.
    std::fs::set_permissions(dir.path(), original).unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "a read-only data dir must not fail the query: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8(output.stdout).unwrap().contains("色即是空"));
}
```

- [ ] **Step 2: 运行,确认失败**

Run: `rustup run 1.96.0 cargo test --test command`
Expected: `cite_builds_the_missing_index_and_output_is_unchanged` FAIL —— 索引未被创建。另两条会通过(尚未有任何建索引逻辑)。

- [ ] **Step 3: 在 `src/data.rs` 实现**

在 `EXPECTED_NORM_RULESET` 常量之后加入:

```rust
/// Name of the local `cite` index. Public so tests and diagnostics can look it
/// up by the same string the builder uses.
pub const CBETA_INDEX_NAME: &str = "idx_parallels_cbeta";

/// `cite` filters on `cbeta_id = ?1 COLLATE NOCASE` and orders by
/// `juan_num, id`. The NOCASE collation on the leading column is required:
/// a BINARY index cannot serve that comparison as an equality seek and
/// degrades to an index scan.
///
/// Deliberately NOT in `schema.sql`: that file is the compatibility reference
/// (`validate_compatibility` checks against it) and is executed verbatim by
/// the Python export pipeline. Every already-downloaded dataset lacks this
/// index and must stay fully valid, so it can never be a compatibility
/// requirement.
const CBETA_INDEX_DDL: &str = "CREATE INDEX IF NOT EXISTS idx_parallels_cbeta \
     ON parallels(cbeta_id COLLATE NOCASE, juan_num, id)";

fn create_cbeta_index(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute_batch(CBETA_INDEX_DDL)
}

fn cbeta_index_exists(conn: &rusqlite::Connection) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1",
        [CBETA_INDEX_NAME],
        |_| Ok(true),
    )
    .optional()
    .map(|found| found.unwrap_or(false))
}

/// Best-effort local optimization for `cite`. Without this index the query
/// scans all 908,620 rows; with it, SQLite seeks directly.
///
/// Every failure degrades to that scan: a read-only filesystem, a busy data
/// operation, or any SQLite error leaves the query correct and merely slower.
/// Nothing here can fail the caller.
pub fn ensure_cbeta_index(conn: &rusqlite::Connection) {
    let Some(path) = conn.path().map(Path::to_path_buf) else {
        return; // no backing file (in-memory)
    };
    if path.as_os_str().is_empty() {
        return;
    }
    // On a read error, assume present rather than risk a pointless rebuild.
    if cbeta_index_exists(conn).unwrap_or(true) {
        return;
    }
    // Never wait: a background `data update`/`clean` means we simply scan.
    let Ok(_lock) = operation_lock::try_acquire(&path) else {
        return;
    };
    // Open read-write BEFORE announcing. Opening is the only reliable
    // writability signal — SQLite needs the containing directory writable for
    // its rollback journal, which file permissions alone do not tell us — and
    // doing it first keeps a read-only install completely silent.
    let Ok(writable) = rusqlite::Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_WRITE,
    ) else {
        return;
    };
    eprintln!("首次按经号查询,正在建立索引(一次性,约 1-2 秒,数据目录约 +17 MB)...");
    let _ = create_cbeta_index(&writable);
}
```

- [ ] **Step 4: 在 `src/cli.rs` 的 `Command::Cite` 分支接上**

在 `Command::Cite` 分支中,把

```rust
            let conn = open_ensured(data_dir, offline)?;
            let groups_all = query::by_cbeta(&conn, cbeta_id.trim(), juan, langs.as_deref(), top)?;
```

替换为

```rust
            let conn = open_ensured(data_dir, offline)?;
            data::ensure_cbeta_index(&conn);
            let groups_all = query::by_cbeta(&conn, cbeta_id.trim(), juan, langs.as_deref(), top)?;
```

只加在 `Cite` 分支。`parallel` 与 `texts` 用不上这个索引,不应为它付检测成本。

- [ ] **Step 5: 运行,确认通过**

Run: `rustup run 1.96.0 cargo test --test command --test golden`
Expected: 全部 PASS,包括三条新测试。

- [ ] **Step 6: 全量 + clippy,提交**

Run: `rustup run 1.96.0 cargo test && rustup run 1.96.0 cargo clippy --all-targets -- -D warnings`

```bash
git add src/data.rs src/cli.rs tests/command.rs
git commit -m "perf(cite): 缺失时懒建 cbeta_id 索引,失败静默退回扫描"
```

---

### Task 3: 安装与更新时建索引

**Files:**
- Modify: `src/data.rs`
- Test: `tests/data.rs`(追加)

**Interfaces:**
- Consumes: `create_cbeta_index`、`CBETA_INDEX_NAME`(Task 2)

在候选文件上建索引,让新装用户与 `data update` 之后的用户不必经过懒建路径。位置在 `verify_dataset_file` 通过之后、原子替换之前:失败时走既有的 `candidate.cleanup_with`,活跃数据不受影响。

- [ ] **Step 1: 写失败测试**

追加到 `tests/data.rs`。把 `CBETA_INDEX_NAME` 加进顶部的 `use fojin_cli::data::{...}` 列表。

这条测试沿用同文件 `ensure_data_downloads_verifies_and_unpacks` 的本地 HTTP server 模式(以及既有的 `gzip_bytes` / `replacement_database_bytes` / `sha256_hex` 三个 helper),走完整的下载 → 校验 → 解压 → 发布路径:

```rust
#[test]
fn install_leaves_the_cite_index_in_place() {
    let gz = gzip_bytes(&replacement_database_bytes());
    let sha = sha256_hex(&gz);

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let body = gz.clone();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut req = [0u8; 4096];
        let _ = std::io::Read::read(&mut stream, &mut req);
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/gzip\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(&body).unwrap();
    });

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.sqlite");
    let source = DataSource {
        url: &format!("http://127.0.0.1:{port}/data.gz"),
        sha256: &sha,
    };
    ensure_data(&path, false, &source).unwrap();
    server.join().unwrap();

    let conn = open_read_only_db(&path).unwrap();
    let present: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name=?1",
            [CBETA_INDEX_NAME],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(present, 1, "a fresh install must ship the cite index");

    // The index must not disturb the compatibility contract.
    verify_dataset_file(&path).unwrap();
}
```

- [ ] **Step 2: 运行,确认失败**

Run: `rustup run 1.96.0 cargo test --test data install_leaves_the_cite_index_in_place`
Expected: FAIL,`assertion \`left == right\` failed: a fresh install must ship the cite index`(left: 0)。

- [ ] **Step 3: 实现**

在 `src/data.rs` 的 `install_candidate` 中,`verify_dataset_file` 之后、同步之前插入建索引:

```rust
fn install_candidate(path: &Path, source: &DataSource<'_>) -> Result<()> {
    let candidate = transfer::stage_candidate(path, source, transfer::PRODUCTION_POLICY)?;
    if let Err(error) = verify_dataset_file(candidate.path()).map(|_| ()) {
        return Err(candidate.cleanup_with(error));
    }
    // Build the cite index on the candidate so a fresh install never has to
    // take the lazy path. ~1.6 s against a 183 MB download.
    if let Err(error) = rusqlite::Connection::open_with_flags(
        candidate.path(),
        OpenFlags::SQLITE_OPEN_READ_WRITE,
    )
    .map_err(anyhow::Error::from)
    .and_then(|conn| create_cbeta_index(&conn).map_err(anyhow::Error::from))
    .with_context(|| format!("为候选数据建立索引失败: {}", candidate.path().display()))
    {
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

- [ ] **Step 4: 运行,确认通过**

Run: `rustup run 1.96.0 cargo test --test data`
Expected: 全部 PASS。

- [ ] **Step 5: 全量 + clippy,提交**

Run: `rustup run 1.96.0 cargo test && rustup run 1.96.0 cargo clippy --all-targets -- -D warnings`

```bash
git add src/data.rs tests/data.rs
git commit -m "perf(data): 安装与更新时为候选数据建立 cite 索引"
```

---

### Task 4: `confidence` 为 1.00 时不显示标签

**Files:**
- Modify: `src/render.rs`
- Test: `tests/render.rs`(追加)

**Interfaces:**
- 无新公开签名。`conf_tag` 是 `render.rs` 私有函数,仅行为改变。

数据集全部 908,620 行的 `confidence` 都是 1.0,所以每行末尾的 `[MITRA 1.00]` 不携带任何信息。改为**按显示精度**判断:格式化成两位小数后若为 `1.00` 则不输出标签。按格式化结果而非原始值比较,避免 0.995 这类值既显示成 `[MITRA 1.00]` 又被判定为"有信息量"的自相矛盾。

JSON 的 `confidence` 字段与 `group_and_rank` / `cap_per_lang` 的排序逻辑**一律不动**:前者是 agent 契约,后者对未来有真实分数的数据是正确的。

- [ ] **Step 1: 写失败测试**

追加到 `tests/render.rs`:

```rust
#[test]
fn perfect_confidence_shows_no_tag() {
    let group = MatchGroup {
        zh_text: "色即是空".into(),
        cbeta_id: Some("T0251".into()),
        title_zh: Some("心經".into()),
        juan_num: Some(1),
        parallels: vec![Parallel {
            lang: "sa".into(),
            text: "rūpaṃ śūnyatā".into(),
            confidence: Some(1.0),
        }],
    };
    let out = render_human(&[group], None, 0);
    assert!(out.contains("梵  rūpaṃ śūnyatā"), "the parallel itself stays: {out}");
    assert!(
        !out.contains("MITRA"),
        "a uniform 1.00 carries no information and must not be printed: {out}"
    );
}

#[test]
fn imperfect_confidence_still_shows_the_tag() {
    let group = MatchGroup {
        zh_text: "色即是空".into(),
        cbeta_id: Some("T0251".into()),
        title_zh: Some("心經".into()),
        juan_num: Some(1),
        parallels: vec![Parallel {
            lang: "sa".into(),
            text: "rūpaṃ śūnyatā".into(),
            confidence: Some(0.87),
        }],
    };
    let out = render_human(&[group], None, 0);
    assert!(out.contains("梵  rūpaṃ śūnyatā  [MITRA 0.87]"), "got: {out}");
}

#[test]
fn confidence_rounding_to_one_hides_the_tag() {
    // 0.995 formats as "1.00"; printing `[MITRA 1.00]` while calling the value
    // informative would contradict itself, so the rule keys on what would be
    // displayed, not on the raw float.
    let group = MatchGroup {
        zh_text: "色即是空".into(),
        cbeta_id: Some("T0251".into()),
        title_zh: Some("心經".into()),
        juan_num: Some(1),
        parallels: vec![Parallel {
            lang: "sa".into(),
            text: "rūpaṃ śūnyatā".into(),
            confidence: Some(0.995),
        }],
    };
    let out = render_human(&[group], None, 0);
    assert!(!out.contains("MITRA"), "got: {out}");
}

#[test]
fn absent_confidence_shows_no_tag() {
    let group = MatchGroup {
        zh_text: "色即是空".into(),
        cbeta_id: Some("T0251".into()),
        title_zh: Some("心經".into()),
        juan_num: Some(1),
        parallels: vec![Parallel {
            lang: "sa".into(),
            text: "rūpaṃ śūnyatā".into(),
            confidence: None,
        }],
    };
    let out = render_human(&[group], None, 0);
    assert!(!out.contains("MITRA"), "got: {out}");
}

#[test]
fn json_still_carries_confidence_when_the_tag_is_hidden() {
    let group = MatchGroup {
        zh_text: "色即是空".into(),
        cbeta_id: Some("T0251".into()),
        title_zh: Some("心經".into()),
        juan_num: Some(1),
        parallels: vec![Parallel {
            lang: "sa".into(),
            text: "rūpaṃ śūnyatā".into(),
            confidence: Some(1.0),
        }],
    };
    let out = render_json(&[group], 1);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        v["groups"][0]["parallels"][0]["confidence"],
        serde_json::json!(1.0),
        "the JSON contract is unchanged; only the human tag is suppressed"
    );
}
```

- [ ] **Step 2: 运行,确认失败**

Run: `rustup run 1.96.0 cargo test --test render`
Expected: `perfect_confidence_shows_no_tag`、`confidence_rounding_to_one_hides_the_tag` FAIL(输出里仍有 `[MITRA 1.00]`);其余通过。

- [ ] **Step 3: 实现**

把 `src/render.rs` 的 `conf_tag` 替换为:

```rust
/// The tag appears only when it would show a number other than 1.00.
///
/// Keying on the formatted string rather than the raw float keeps the rule
/// self-consistent: a value like 0.995 would render as "1.00", so treating it
/// as informative would print a tag that says exactly what we suppress
/// elsewhere. Absence of the tag reads as "no caveat".
fn conf_tag(c: Option<f64>) -> String {
    match c {
        Some(v) => {
            let shown = format!("{v:.2}");
            if shown == "1.00" {
                String::new()
            } else {
                format!("  [MITRA {shown}]")
            }
        }
        None => String::new(),
    }
}
```

- [ ] **Step 4: 运行,确认通过**

Run: `rustup run 1.96.0 cargo test --test render --test golden --test cli`
Expected: 全部 PASS。既有断言用的都是 0.91 / 0.88 / 0.75,均 < 1.00,不受影响 —— 包括 `tests/golden.rs` 的逐字节守卫。

- [ ] **Step 5: 全量 + clippy,提交**

Run: `rustup run 1.96.0 cargo test && rustup run 1.96.0 cargo clippy --all-targets -- -D warnings`

```bash
git add src/render.rs tests/render.rs
git commit -m "feat(render): 置信度显示为 1.00 时不再输出标签"
```

---

### Task 5: 真实数据验证与文档

**Files:**
- Modify: `README.md`
- Modify: `CHANGELOG.md`

- [ ] **Step 1: 真实数据冒烟(必须实测,不得跳过)**

数据集是只读资源。**先复制到临时目录再测,不要直接对 `~/.cache/fojin/data.sqlite` 操作,也不要运行 `fojin data clean` / `data update`。**

```bash
SP=/tmp/claude-1000/-home-lqsxi-projects-fojin-cli/88b0dd32-1d6d-4140-a736-9dae215864d7/scratchpad/verify
mkdir -p "$SP" && cp ~/.cache/fojin/data.sqlite "$SP/"
rustup run 1.96.0 cargo build --release
B=./target/release/fojin

# 1) 首次 cite:应打印一次建索引提示,并明显变慢一次
time $B cite T0251 --limit 3 --offline --data-dir "$SP"

# 2) 再次 cite:无提示,应为毫秒级
time $B cite T0251 --limit 3 --offline --data-dir "$SP"

# 3) 另一部经,确认索引对所有 cbeta_id 生效
time $B cite T0220 --limit 2 --offline --data-dir "$SP"

# 4) --juan 走复合前缀
time $B cite T0251 --juan 1 --limit 2 --offline --data-dir "$SP"

# 5) confidence:真实数据全为 1.0,输出中不应再出现 MITRA 标签
$B parallel "色即是空" --offline --data-dir "$SP" | grep -c "MITRA" || echo "0 处 MITRA(符合预期)"

# 6) 索引不改变结果:与一份保持未建索引的副本对比同一查询。
#    不能拿 ~/.cache 那份做对照 —— cite 会就地把索引建起来,
#    而且那是用户的真实数据,不该被本次验证改动。
#    用只读目录强制建索引静默失败,从而保持无索引状态:
mkdir -p "$SP/plain" && cp ~/.cache/fojin/data.sqlite "$SP/plain/"
chmod 500 "$SP/plain"
$B cite T0251 --limit 3 --offline --data-dir "$SP/plain" > "$SP/no-index.txt"
chmod 700 "$SP/plain"
$B cite T0251 --limit 3 --offline --data-dir "$SP" > "$SP/with-index.txt"
diff "$SP/no-index.txt" "$SP/with-index.txt" && echo "有无索引输出完全一致"

rm -rf "$SP"
```

Expected:第 1 步打印提示且耗时约 1–3 s;第 2/3/4 步无提示且为毫秒级;第 5 步为 0 处;第 6 步 diff 无差异,且只读目录那次不打印任何提示、退出码为 0(顺带在真实数据上验证了降级路径)。**把实际输出与耗时记录到报告中。**

- [ ] **Step 2: 更新 README**

(a) 第 15–16 行的首屏示例输出去掉 `  [MITRA 1.00]`:

```markdown
梵  śūnyat'aiva rūpaṃ, rūpān na pṛthak śūnyatā …
藏  གཟུགས་ལས་སྟོང་པ་ཉིད་གཞན་མ་ཡིན༏ …
```

(b) 在「其他子命令」一节 `fojin cite` 的说明之后补一句:

```markdown
首次运行 `fojin cite` 会为按经号查询建立一次本地索引(约 1–2 秒,数据目录增加约 17 MB),
之后按经号查询为毫秒级。数据目录不可写时会跳过建索引,查询结果不受影响,只是较慢。
```

(c) 在「数据集」一节补一条,说明置信度:

```markdown
- 当前数据集(`data-v1`)所有对齐的置信度均为 1.00,不具区分度,因此人类可读输出不再逐行标注;
  `--json` 的 `confidence` 字段保持输出。未来数据若提供真实分数,低于 1.00 的会自动显示。
```

- [ ] **Step 3: 更新 CHANGELOG**

在 `## [0.3.0] - Unreleased` 的列表中追加:

```markdown
- 查询性能:`cite` 现在使用本地建立的 `cbeta_id` 索引,不再全表扫描;索引在安装/更新时建立,已有数据在首次 `cite` 时补建,数据目录不可写时静默退回扫描。
- 输出可读性:置信度显示为 `1.00` 时不再逐行标注 `[MITRA 1.00]`(当前数据集全部如此);`--json` 的 `confidence` 字段不变。
```

- [ ] **Step 4: 最终验证并提交**

Run:
```bash
rustup run 1.95.0 cargo test --all --locked
rustup run 1.96.0 cargo fmt --all --check
rustup run 1.96.0 cargo clippy --all-targets --locked -- -D warnings
```
Expected:全部通过,测试数应为 213 + 本轮新增。

```bash
git add README.md CHANGELOG.md
git commit -m "docs: 记录 cite 索引与置信度显示变更"
```
