# 检索能力三项(反查 / 切分 / 回退)实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给 `fojin parallel` 加上三项检索能力——梵/藏反向查询、零命中时按句自动切分重查、仍零命中时给出最长可命中子串。

**Architecture:** 抽出 `src/search/` 编排层,把"整串 → 切句 → 回退"的策略从 `cli.rs` 下沉。切分与回退是不碰数据库的纯函数(命中判定由闭包注入),反查是 `query.rs` 里的两趟流式扫描。三项功能全部只在**零命中时**介入,有命中的查询路径逐字节不变。

**Tech Stack:** Rust 2021、rusqlite 0.40.1(bundled SQLite,FTS5 trigram)、clap 4.5.53 derive、serde_json、anyhow。**不新增任何依赖。**

**依据 spec:** `docs/superpowers/specs/2026-07-25-search-capabilities-design.md`

## Global Constraints

- **MSRV 1.95。本机默认 stable 是 1.88,低于 MSRV,所有 cargo 命令必须写成 `rustup run 1.96.0 cargo ...`**,否则会以 "requires rustc 1.95" 失败。
- 不新增 Cargo 依赖。
- 所有面向用户的字符串用中文,与现有输出风格一致(全角括号、两个空格缩进)。
- 退出码契约不变:`0` 成功(含"未找到对齐")、`1` 运行期错误、`2` 用法错误。
- `--json` 时 stdout 必须是纯 JSON;进度与提示走 stderr。
- 有命中的查询,人类可读输出必须与改动前**逐字节一致**(Task 1 的 golden 测试是守卫)。
- `cargo clippy` 无警告,`cargo fmt` 已格式化。
- 每个 Task 结束时提交一次。

## 文件结构

| 文件 | 职责 | 归属 |
| --- | --- | --- |
| `src/search/mod.rs` | 策略编排;`SearchOutcome` 等共享类型 | Task 1 建骨架,Task 6 填逻辑 |
| `src/search/split.rs` | 纯函数:原文 → 分句 | Task 5 |
| `src/search/fallback.rs` | 纯函数:查询串 + 探测闭包 → 最长可命中子串 | Task 4 |
| `src/lang.rs` | 语种白名单与校验 | Task 2 |
| `src/query.rs` | 新增 `search_foreign`;排序逻辑参数化 | Task 3 |
| `src/render.rs` | 新增分句/回退渲染与 JSON 组装 | Task 7 |
| `src/cli.rs` | 新增 `--from` / `--no-split`;编排下沉 | Task 1 接线,Task 2 校验 |

## 并行执行编排

Task 1 由主会话**串行**完成,它固定所有跨任务签名。之后三路并行,各自独立 git worktree:

| agent | 任务 | 独占文件 |
| --- | --- | --- |
| agent-1 · 反查 | Task 2 → Task 3 | `src/lang.rs`、`src/query.rs`、`tests/lang.rs`、`tests/query.rs` |
| agent-2 · 切分 | Task 5 → Task 6 → Task 7 | `src/search/split.rs`、`src/search/mod.rs`、`src/render.rs`、`tests/split.rs`、`tests/search.rs`、`tests/render.rs` |
| agent-3 · 回退 | Task 4 | `src/search/fallback.rs`、`tests/fallback.rs` |

合并顺序 **Task 1 → agent-1 → agent-3 → agent-2**。agent-2 的 Task 6 按 Task 1 固定的签名调用 `fallback::longest_matching`,骨架里的桩返回 `Ok(None)`,所以它不必等 agent-3 就能编译和写测试;端到端行为在合并后由 Task 8 验证。

**唯一的已知冲突点:** `src/search/mod.rs` 的 `run` 函数被 Task 3(agent-1 加 `from` 分支)和 Task 6(agent-2 重写全部三条路径)同时改到。Task 6 给出的 `run` 代码**已经包含** Task 3 的 `from` 分支,所以合并 agent-2 时对这个函数直接取 Task 6 的版本即可,不要手工拼接。除此之外三路无重叠文件。

Task 8 由主会话串行收尾。

---

### Task 1: 骨架 —— 共享类型、模块接线、惰性 flag、golden 回归守卫

**Files:**
- Create: `src/search/mod.rs`
- Create: `src/search/split.rs`
- Create: `src/search/fallback.rs`
- Create: `src/lang.rs`
- Create: `tests/golden.rs`
- Modify: `src/lib.rs`
- Modify: `src/cli.rs`

**Interfaces:**
- Produces:
  - `search::SearchRequest<'a> { raw: &'a str, langs: Option<&'a [String]>, top: usize, limit: Option<usize>, from: Option<&'a str>, no_split: bool }`
  - `search::SearchOutcome { groups: Vec<MatchGroup>, total: usize, segments: Option<Vec<SegmentResult>>, fallback: Option<FallbackInfo>, truncated_segments: usize }`
  - `search::SegmentResult { text: String, matched: bool, total: usize, groups: Vec<MatchGroup>, fallback: Option<FallbackInfo> }`
  - `search::FallbackInfo { matched_substring: String, char_len: usize }`
  - `search::run(conn: &Connection, req: &SearchRequest) -> anyhow::Result<SearchOutcome>`
  - `search::split::split_sentences(raw: &str, keep: impl Fn(&str) -> bool) -> SplitOutcome`
  - `search::split::SplitOutcome { segments: Vec<String>, truncated: usize }`
  - `search::fallback::longest_matching(norm_query: &str, probe: impl Fn(&str) -> anyhow::Result<bool>) -> anyhow::Result<Option<FallbackInfo>>`
  - `lang::KNOWN_LANGS: [&str; 6]`
  - `lang::validate_langs(codes: &[String]) -> anyhow::Result<()>`
  - `cli::compute_search(conn: &Connection, req: &SearchRequest, json: bool) -> anyhow::Result<String>`
  - 常量 `search::SCHEMA_VERSION = 1`、`search::SEGMENT_GROUP_CAP = 3`、`split::MAX_SEGMENTS = 20`、`fallback::MAX_FALLBACK_CHARS = 60`

- [ ] **Step 1: 写 golden 回归测试(此刻必须通过 —— 它记录的是改动前的行为)**

创建 `tests/golden.rs`:

```rust
//! Byte-for-byte guard on the hit path. The search-capability work (reverse
//! lookup, sentence splitting, fallback) must only engage on ZERO hits; a
//! query that matches must render exactly as it did before that work landed.
use fojin_cli::cli::compute_output;
use fojin_cli::schema::init_schema;
use rusqlite::{params, Connection};

fn fixture() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    init_schema(&conn).unwrap();
    conn.execute(
        "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
         VALUES ('色即是空','色即是空','sa','rūpaṃ śūnyatā',0.91,'T0251','心經',1)",
        params![],
    )
    .unwrap();
    conn
}

#[test]
fn hit_path_human_output_is_byte_identical() {
    let conn = fixture();
    let out = compute_output(&conn, "色即是空", None, 3, None, false).unwrap();
    assert_eq!(
        out,
        "汉  色即是空  (《心經》T0251 卷1)\n\
         梵  rūpaṃ śūnyatā  [MITRA 0.91]\n\
         藏  (无对齐)\n\
         \n\
         完整上下文见 https://fojin.app  ·  数据 CC BY-SA(Dharmamitra + fojin)\n"
    );
}
```

- [ ] **Step 2: 运行,确认它此刻就通过**

Run: `rustup run 1.96.0 cargo test --test golden`
Expected: PASS(1 passed)。若失败,说明字面量与当前 `render::render_human` 输出不符,以**实际输出**为准修正字面量后再继续——这条测试记录现状,不是改变现状。

- [ ] **Step 3: 建 `src/lang.rs`**

```rust
use anyhow::{bail, Result};

/// Language codes the renderer knows how to label. Validation uses this static
/// list rather than `SELECT DISTINCT foreign_lang`, which would full-scan
/// 908k rows (the column has no index).
pub const KNOWN_LANGS: [&str; 6] = ["sa", "pi", "bo", "en", "lzh", "zh"];

pub fn validate_langs(codes: &[String]) -> Result<()> {
    for code in codes {
        if !KNOWN_LANGS.contains(&code.as_str()) {
            bail!(
                "未知语种 `{code}`;可用: {}",
                KNOWN_LANGS.join(", ")
            );
        }
    }
    Ok(())
}
```

- [ ] **Step 4: 建 `src/search/mod.rs` 骨架**

```rust
use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

use crate::model::MatchGroup;
use crate::{normalize, query};

pub mod fallback;
pub mod split;

/// Bumped when the `--json` shape of `parallel` changes incompatibly.
pub const SCHEMA_VERSION: u32 = 1;
/// Per-segment display cap in split mode; `--all` lifts it.
pub const SEGMENT_GROUP_CAP: usize = 3;

#[derive(Debug, Clone, Serialize)]
pub struct FallbackInfo {
    pub matched_substring: String,
    pub char_len: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SegmentResult {
    pub text: String,
    pub matched: bool,
    pub total: usize,
    pub groups: Vec<MatchGroup>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<FallbackInfo>,
}

#[derive(Debug)]
pub struct SearchOutcome {
    /// Deduplicated hits across every path, already capped by `--limit`.
    pub groups: Vec<MatchGroup>,
    /// Hit count before the `--limit` cap.
    pub total: usize,
    pub segments: Option<Vec<SegmentResult>>,
    pub fallback: Option<FallbackInfo>,
    /// Segments dropped by the MAX_SEGMENTS cap; rendered, never silent.
    pub truncated_segments: usize,
}

impl SearchOutcome {
    /// The plain hit path: no splitting, no fallback.
    fn plain(groups: Vec<MatchGroup>, limit: Option<usize>) -> Self {
        let total = groups.len();
        let shown = match limit {
            Some(n) => n.min(total),
            None => total,
        };
        let mut groups = groups;
        groups.truncate(shown);
        SearchOutcome {
            groups,
            total,
            segments: None,
            fallback: None,
            truncated_segments: 0,
        }
    }
}

pub struct SearchRequest<'a> {
    pub raw: &'a str,
    pub langs: Option<&'a [String]>,
    pub top: usize,
    pub limit: Option<usize>,
    pub from: Option<&'a str>,
    pub no_split: bool,
}

/// Skeleton: whole-string query only. Task 3 adds the `from` branch; Task 6
/// adds splitting and fallback.
pub fn run(conn: &Connection, req: &SearchRequest) -> Result<SearchOutcome> {
    let map = normalize::load_norm_map(conn)?;
    let norm = normalize::normalize(req.raw.trim(), &map);
    normalize::validate_query_length(&norm)?;
    let groups = query::search(conn, &norm, req.langs, req.top)?;
    Ok(SearchOutcome::plain(groups, req.limit))
}
```

- [ ] **Step 5: 建 `src/search/split.rs` 与 `src/search/fallback.rs` 的桩**

`src/search/split.rs`:

```rust
/// Max sentences processed when auto-splitting; the excess is reported, never
/// silently dropped.
pub const MAX_SEGMENTS: usize = 20;

pub struct SplitOutcome {
    pub segments: Vec<String>,
    pub truncated: usize,
}

/// Task 5 implements this. `keep` decides whether a raw segment survives
/// (callers pass a normalized-length predicate), keeping this file DB-free.
pub fn split_sentences(_raw: &str, _keep: impl Fn(&str) -> bool) -> SplitOutcome {
    SplitOutcome {
        segments: Vec::new(),
        truncated: 0,
    }
}
```

`src/search/fallback.rs`:

```rust
use anyhow::Result;

use super::FallbackInfo;

/// Beyond this length the O(n log n) probe budget is not worth spending.
pub const MAX_FALLBACK_CHARS: usize = 60;

/// Task 4 implements this. `probe` reports whether a candidate substring has
/// any alignment, so this file needs no database.
pub fn longest_matching(
    _norm_query: &str,
    _probe: impl Fn(&str) -> Result<bool>,
) -> Result<Option<FallbackInfo>> {
    Ok(None)
}
```

- [ ] **Step 6: 接线 `src/lib.rs`**

在模块列表中加入两行(保持字母序):

```rust
pub mod cli;
pub mod data;
pub mod lang;
pub mod model;
pub mod normalize;
pub mod query;
pub mod render;
pub mod schema;
pub mod search;
```

- [ ] **Step 7: 在 `src/cli.rs` 声明两个惰性 flag 并加 `compute_search`**

在 `Command::Parallel` 的 `offline` 字段**之前**插入两个字段:

```rust
        /// 反向查询:用该语种的原文查汉文对应(如 --from sa)
        #[arg(long)]
        from: Option<String>,
        /// 零命中时不自动按句切分重查
        #[arg(long)]
        no_split: bool,
```

在 `compute_output` 之后加入新函数,并把 `compute_output` 改为委托(签名一字不改,现有测试与 golden 守卫因此继续有效):

```rust
pub fn compute_search(
    conn: &Connection,
    req: &search::SearchRequest,
    json: bool,
) -> Result<String> {
    let outcome = search::run(conn, req)?;
    Ok(if json {
        render::render_outcome_json(&outcome)
    } else {
        render::render_outcome_human(&outcome, req.langs)
    })
}

pub fn compute_output(
    conn: &Connection,
    raw: &str,
    langs: Option<&[String]>,
    top: usize,
    limit: Option<usize>,
    json: bool,
) -> Result<String> {
    compute_search(
        conn,
        &search::SearchRequest {
            raw,
            langs,
            top,
            limit,
            from: None,
            no_split: false,
        },
        json,
    )
}
```

顶部 `use` 增加 `search`:

```rust
use crate::{data, normalize, query, render, search};
```

`Command::Parallel` 的处理分支中,把 `let out = compute_output(...)` 换成:

```rust
            let out = compute_search(
                &conn,
                &search::SearchRequest {
                    raw: &raw,
                    langs: langs.as_deref(),
                    top,
                    limit,
                    from: from.as_deref(),
                    no_split,
                },
                json,
            )?;
```

并在 `match cli.command` 的 `Command::Parallel { ... }` 解构里补上 `from,` 与 `no_split,` 两个绑定。

- [ ] **Step 8: 在 `src/render.rs` 加两个 outcome 渲染入口(此刻只是转发)**

```rust
use crate::search::SearchOutcome;

pub fn render_outcome_human(outcome: &SearchOutcome, langs: Option<&[String]>) -> String {
    let hidden = outcome.total - outcome.groups.len();
    render_human(&outcome.groups, langs, hidden)
}

pub fn render_outcome_json(outcome: &SearchOutcome) -> String {
    render_json(&outcome.groups, outcome.total)
}
```

- [ ] **Step 9: 全量测试 + clippy**

Run: `rustup run 1.96.0 cargo test && rustup run 1.96.0 cargo clippy --all-targets -- -D warnings`
Expected: 全部 PASS,clippy 无警告。golden 测试仍通过 —— 编排下沉没有改变任何输出。

- [ ] **Step 10: 提交**

```bash
git add src/search src/lang.rs src/lib.rs src/cli.rs src/render.rs tests/golden.rs
git commit -m "refactor(search): 抽出 search 编排层与共享类型,加 golden 回归守卫"
```

---

### Task 2: 语种白名单校验(agent-1)

**Files:**
- Modify: `src/cli.rs`
- Test: `tests/lang.rs`(创建)、`tests/command.rs`(追加)

**Interfaces:**
- Consumes: `lang::KNOWN_LANGS`、`lang::validate_langs`(Task 1 已建)
- Produces: 无新签名;`--lang` 与 `--from` 在参数解析后、开库前完成校验

- [ ] **Step 1: 写失败测试**

创建 `tests/lang.rs`:

```rust
use fojin_cli::lang::{validate_langs, KNOWN_LANGS};
use fojin_cli::render::lang_label;

#[test]
fn accepts_known_codes() {
    let codes = vec!["sa".to_string(), "bo".to_string()];
    assert!(validate_langs(&codes).is_ok());
}

#[test]
fn rejects_unknown_code_and_lists_alternatives() {
    let codes = vec!["sk".to_string()];
    let err = validate_langs(&codes).unwrap_err().to_string();
    assert!(err.contains("未知语种 `sk`"), "got: {err}");
    assert!(err.contains("sa"), "error must list usable codes: {err}");
}

#[test]
fn every_known_lang_has_a_real_label() {
    // Guards KNOWN_LANGS against render::lang_label drifting apart: a code with
    // no label falls through to the `other => other` arm and returns itself.
    for code in KNOWN_LANGS {
        assert_ne!(
            lang_label(code),
            code,
            "`{code}` is in KNOWN_LANGS but has no label in render::lang_label"
        );
    }
}
```

追加到 `tests/command.rs`:

```rust
#[test]
fn unknown_from_lang_is_a_usage_error_before_touching_data() {
    let dir = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args(["parallel", "śūnyatā", "--from", "sk", "--offline", "--data-dir"])
        .arg(dir.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("未知语种 `sk`"), "got: {stderr}");
    assert!(
        !stderr.contains("本地数据不存在"),
        "validation must precede data access: {stderr}"
    );
}

#[test]
fn unknown_display_lang_is_a_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args(["parallel", "色即是空", "--lang", "sk", "--offline", "--data-dir"])
        .arg(dir.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8(output.stderr).unwrap().contains("未知语种 `sk`"));
}

#[test]
fn from_with_no_split_is_a_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args([
            "parallel", "śūnyatā", "--from", "sa", "--no-split", "--offline", "--data-dir",
        ])
        .arg(dir.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("--from 不做切分"));
}
```

- [ ] **Step 2: 运行,确认失败**

Run: `rustup run 1.96.0 cargo test --test lang --test command`
Expected: `tests/lang.rs` 通过(Task 1 已实现 `validate_langs`),`tests/command.rs` 三条新测试 FAIL —— 退出码是 0 或 1 而非 2。

- [ ] **Step 3: 在 `src/cli.rs` 的 `Command::Parallel` 分支接入校验**

在 `if raw.trim().is_empty()` 判空**之后**、`open_ensured` **之前**插入:

```rust
            if from.is_some() && no_split {
                eprintln!("--from 不做切分,不能与 --no-split 同用");
                return Ok(2);
            }
            if let Some(code) = from.as_deref() {
                if let Err(e) = crate::lang::validate_langs(&[code.to_string()]) {
                    eprintln!("{e}");
                    return Ok(2);
                }
            }
```

`langs` 解析出来之后立即校验。把现有的 `let langs: Option<Vec<String>> = lang.map(...)` 这一段整体移到 `open_ensured` **之前**,并在其后插入:

```rust
            if let Some(codes) = langs.as_deref() {
                if let Err(e) = crate::lang::validate_langs(codes) {
                    eprintln!("{e}");
                    return Ok(2);
                }
            }
```

对 `Command::Cite` 分支做同样的 `langs` 校验(同一段代码,插在它的 `open_ensured` 之前),保证两个子命令行为一致。

- [ ] **Step 4: 运行,确认通过**

Run: `rustup run 1.96.0 cargo test --test lang --test command`
Expected: 全部 PASS。

- [ ] **Step 5: 提交**

```bash
git add src/cli.rs tests/lang.rs tests/command.rs
git commit -m "feat(cli): 校验 --lang/--from 语种代码,未知值报用法错误"
```

---

### Task 3: 反向查询 `query::search_foreign`(agent-1)

**Files:**
- Modify: `src/query.rs`
- Modify: `src/search/mod.rs`(仅 `run` 里加 `from` 分支)
- Test: `tests/query.rs`(追加)

**Interfaces:**
- Consumes: `query::Row`(私有)、`query::cap_per_lang`(私有)、`model::MatchGroup`
- Produces:
  - `query::MIN_FOREIGN_QUERY_CHARS: usize = 3`
  - `query::search_foreign(conn: &Connection, from_lang: &str, raw_query: &str, langs: Option<&[String]>, top: usize) -> rusqlite::Result<Vec<MatchGroup>>`

**为什么是两趟扫描:** 命中行只属于 `from_lang`,但输出承诺展示该组的**全部**平行(用梵文找到,也要看见藏文)。所以第一趟只在 `from_lang` 的行里定位命中的分组键,第二趟按键把各语种的行都取回来。第一趟无命中时**直接返回,不做第二趟**——零命中的反查只付一趟的代价。

**为什么在 Rust 侧折叠大小写:** SQLite 的 `instr` 大小写敏感,`LOWER()` 只处理 ASCII,折叠不了 `Ś`→`ś`。数据里句首大写常见(`Tasmāc Chāriputra ...`),用户输入 `tasmāc` 会一条都查不到。

- [ ] **Step 1: 写失败测试**

追加到 `tests/query.rs`:

```rust
use fojin_cli::query::search_foreign;

fn bilingual_fixture() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    init_schema(&conn).unwrap();
    for (zt, zn, lang, f) in [
        ("色即是空", "色即是空", "sa", "Rūpaṃ śūnyatā"),
        ("色即是空", "色即是空", "bo", "gzugs stong pa"),
        ("受想行識", "受想行识", "sa", "vedanā saṃjñā"),
    ] {
        conn.execute(
            "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
             VALUES (?1,?2,?3,?4,1.0,'T0251','心經',1)",
            params![zt, zn, lang, f],
        )
        .unwrap();
    }
    conn
}

#[test]
fn foreign_search_finds_group_and_shows_all_langs() {
    let conn = bilingual_fixture();
    let g = search_foreign(&conn, "sa", "śūnyatā", None, 3).unwrap();
    assert_eq!(g.len(), 1);
    assert_eq!(g[0].zh_text, "色即是空");
    assert_eq!(
        g[0].parallels.len(),
        2,
        "a Sanskrit hit must still surface the group's Tibetan parallel"
    );
}

#[test]
fn foreign_search_folds_case_and_diacritic_capitals() {
    let conn = bilingual_fixture();
    // stored as "Rūpaṃ śūnyatā"; the query is all-lowercase
    let g = search_foreign(&conn, "sa", "rūpaṃ", None, 3).unwrap();
    assert_eq!(g.len(), 1);
    assert_eq!(g[0].zh_text, "色即是空");
}

#[test]
fn foreign_search_respects_from_lang() {
    let conn = bilingual_fixture();
    // "gzugs" only exists in the bo rows, so searching sa must not find it
    let g = search_foreign(&conn, "sa", "gzugs", None, 3).unwrap();
    assert!(g.is_empty());
}

#[test]
fn foreign_search_display_lang_filter_applies() {
    let conn = bilingual_fixture();
    let langs = vec!["bo".to_string()];
    let g = search_foreign(&conn, "sa", "śūnyatā", Some(&langs), 3).unwrap();
    assert_eq!(g.len(), 1, "found via Sanskrit");
    assert_eq!(g[0].parallels.len(), 1);
    assert_eq!(
        g[0].parallels[0].lang, "bo",
        "--from sa --lang bo means: find by Sanskrit, display Tibetan"
    );
}

#[test]
fn foreign_search_no_hit_is_empty_not_error() {
    let conn = bilingual_fixture();
    let g = search_foreign(&conn, "sa", "nirvāṇa", None, 3).unwrap();
    assert!(g.is_empty());
}

#[test]
fn known_lang_absent_from_dataset_answers_empty_not_error() {
    // `pi` is a valid code with zero rows in data-v1. It must answer honestly
    // rather than error — the spec's "已知但本数据集无行 → 退出码 0" row.
    let conn = bilingual_fixture();
    let g = search_foreign(&conn, "pi", "dukkha", None, 3).unwrap();
    assert!(g.is_empty());
}
```

- [ ] **Step 2: 运行,确认失败**

Run: `rustup run 1.96.0 cargo test --test query`
Expected: FAIL,`cannot find function search_foreign in module`。

- [ ] **Step 3: 实现 `search_foreign`**

在 `src/query.rs` 顶部 `use` 加入 `std::collections::HashSet`,并在 `search` 之后加入:

```rust
/// Minimum characters for a reverse query. IAST bigrams like `ka` match tens of
/// thousands of rows and carry no parallel-reading value; this mirrors the
/// 2-character floor on the Chinese side.
pub const MIN_FOREIGN_QUERY_CHARS: usize = 3;

/// Reverse lookup: find Chinese segments whose `from_lang` parallel contains
/// `raw_query`. Matching happens in Rust with full Unicode case folding —
/// SQLite's `instr` is case-sensitive and its `LOWER()` is ASCII-only, so
/// `tasmāc` would never find the stored `Tasmāc`.
pub fn search_foreign(
    conn: &Connection,
    from_lang: &str,
    raw_query: &str,
    langs: Option<&[String]>,
    top: usize,
) -> rusqlite::Result<Vec<MatchGroup>> {
    let needle = raw_query.trim().to_lowercase();
    if needle.is_empty() {
        return Ok(vec![]);
    }

    // Pass 1: locate hit groups, scanning only the source language's rows.
    let mut stmt = conn.prepare(
        "SELECT zh_text, cbeta_id, juan_num, foreign_text \
         FROM parallels WHERE foreign_lang = ?1",
    )?;
    let mut hits: HashSet<(String, Option<String>, Option<i64>)> = HashSet::new();
    let mut rows = stmt.query([from_lang])?;
    while let Some(row) = rows.next()? {
        let foreign_text: String = row.get(3)?;
        if !foreign_text.to_lowercase().contains(&needle) {
            continue;
        }
        hits.insert((row.get(0)?, row.get(1)?, row.get(2)?));
    }
    if hits.is_empty() {
        return Ok(vec![]);
    }

    // Pass 2: pull every language's rows for the hit groups.
    let mut stmt = conn.prepare(
        "SELECT zh_text,zh_norm,foreign_lang,foreign_text,confidence,\
                cbeta_id,title_zh,juan_num FROM parallels",
    )?;
    let mut collected: Vec<Row> = Vec::new();
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let row = Row {
            zh_text: r.get(0)?,
            zh_norm: r.get(1)?,
            foreign_lang: r.get(2)?,
            foreign_text: r.get(3)?,
            confidence: r.get(4)?,
            cbeta_id: r.get(5)?,
            title_zh: r.get(6)?,
            juan_num: r.get(7)?,
        };
        let key = (row.zh_text.clone(), row.cbeta_id.clone(), row.juan_num);
        if hits.contains(&key) {
            collected.push(row);
        }
    }

    Ok(group_and_rank(
        collected,
        &needle,
        langs,
        top,
        MatchColumn::ForeignText,
    ))
}
```

- [ ] **Step 4: 把 `group_and_rank` 的贴合度来源参数化**

在 `src/query.rs` 中 `struct Acc` 之前加入:

```rust
/// Which column the query string is measured against when ranking groups.
#[derive(Clone, Copy)]
enum MatchColumn {
    ZhNorm,
    ForeignText,
}
```

把 `group_and_rank` 的签名与取值改为:

```rust
fn group_and_rank(
    rows: Vec<Row>,
    needle: &str,
    langs: Option<&[String]>,
    top: usize,
    column: MatchColumn,
) -> Vec<MatchGroup> {
    let mut accs: Vec<Acc> = Vec::new();
    let mut acc_idx: HashMap<GroupKey, usize> = HashMap::new();
    let query_chars = needle.chars().count();
    for row in rows {
        if let Some(filter) = langs {
            if !filter.iter().any(|l| l == &row.foreign_lang) {
                continue;
            }
        }
        let hay = match column {
            MatchColumn::ZhNorm => row.zh_norm.clone(),
            MatchColumn::ForeignText => row.foreign_text.to_lowercase(),
        };
        let contains = hay.contains(needle);
        let exact = hay == needle;
        let excess_chars = if contains {
            hay.chars().count().saturating_sub(query_chars)
        } else {
            hay.chars().count()
        };
        // ... 其余循环体一字不改 ...
```

`search` 里的调用点相应改为:

```rust
    Ok(group_and_rank(rows, norm_query, langs, top, MatchColumn::ZhNorm))
```

- [ ] **Step 5: 运行,确认通过**

Run: `rustup run 1.96.0 cargo test --test query --test golden`
Expected: 全部 PASS。golden 必须仍通过 —— `MatchColumn::ZhNorm` 分支与改动前逐行等价。

- [ ] **Step 6: 在 `search::run` 里接上 `from` 分支**

在 `src/search/mod.rs` 的 `run` 中,把函数体改为:

```rust
pub fn run(conn: &Connection, req: &SearchRequest) -> Result<SearchOutcome> {
    if let Some(from_lang) = req.from {
        let needle = req.raw.trim();
        if needle.chars().count() < query::MIN_FOREIGN_QUERY_CHARS {
            anyhow::bail!(
                "反向查询至少需要 {} 个字符;更短的串会命中过多,无对读价值",
                query::MIN_FOREIGN_QUERY_CHARS
            );
        }
        let groups = query::search_foreign(conn, from_lang, needle, req.langs, req.top)?;
        return Ok(SearchOutcome::plain(groups, req.limit));
    }

    let map = normalize::load_norm_map(conn)?;
    let norm = normalize::normalize(req.raw.trim(), &map);
    normalize::validate_query_length(&norm)?;
    let groups = query::search(conn, &norm, req.langs, req.top)?;
    Ok(SearchOutcome::plain(groups, req.limit))
}
```

注意反查**不做**汉文归一化,也不走 `validate_query_length`(那是 2 个汉字的规则)。

- [ ] **Step 7: 补一条最小长度的端到端测试**

追加到 `tests/command.rs`:

```rust
#[test]
fn short_reverse_query_is_a_runtime_error() {
    let dir = tempfile::tempdir().unwrap();
    write_offline_db(dir.path());

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args(["parallel", "ka", "--from", "sa", "--offline", "--data-dir"])
        .arg(dir.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("至少需要 3 个字符"));
}
```

- [ ] **Step 8: 全量测试 + clippy**

Run: `rustup run 1.96.0 cargo test && rustup run 1.96.0 cargo clippy --all-targets -- -D warnings`
Expected: 全部 PASS,无警告。

- [ ] **Step 9: 提交**

```bash
git add src/query.rs src/search/mod.rs tests/query.rs tests/command.rs
git commit -m "feat(query): 反向查询 --from,Rust 侧 Unicode 折叠 + 两趟扫描"
```

---

### Task 4: 最长可命中子串回退(agent-3)

**Files:**
- Modify: `src/search/fallback.rs`
- Test: `tests/fallback.rs`(创建)

**Interfaces:**
- Consumes: `search::FallbackInfo`(Task 1 已定义)
- Produces: `fallback::longest_matching` 的**实现**(签名 Task 1 已固定,不得更改)

**算法依据(单调性):** 若长度 L 的某子串能命中,则它的任意长度 L−1 子串也能命中——子串的子串仍是子串。所以"存在长度 L 的命中子串"关于 L 单调递减,可以对 L 二分。每轮对该长度的所有起点按顺序探测,首个命中即返回,因此同长度下天然取最靠前的起点。

- [ ] **Step 1: 写失败测试**

创建 `tests/fallback.rs`:

```rust
use fojin_cli::search::fallback::longest_matching;

#[test]
fn finds_longest_present_substring() {
    let corpus = "色不異空，空不異色";
    let probe = |c: &str| Ok(corpus.contains(c));
    let fb = longest_matching("舍利子色不異空義", probe).unwrap().unwrap();
    assert_eq!(fb.matched_substring, "色不異空");
    assert_eq!(fb.char_len, 4);
}

#[test]
fn returns_none_when_nothing_matches() {
    let probe = |_: &str| Ok(false);
    assert!(longest_matching("涅槃寂靜", probe).unwrap().is_none());
}

#[test]
fn prefers_earliest_start_among_equal_lengths() {
    let probe = |c: &str| Ok(c == "甲乙" || c == "丙丁");
    let fb = longest_matching("甲乙丙丁", probe).unwrap().unwrap();
    assert_eq!(fb.matched_substring, "甲乙");
    assert_eq!(fb.char_len, 2);
}

#[test]
fn never_returns_the_whole_query() {
    // The whole string already failed upstream; probing it again is wasted work
    // and would report a "fallback" identical to the failed query.
    let probe = |_: &str| Ok(true);
    let fb = longest_matching("色即是空", probe).unwrap().unwrap();
    assert_eq!(fb.char_len, 3);
}

#[test]
fn declines_queries_shorter_than_three_chars() {
    let probe = |_: &str| Ok(true);
    assert!(longest_matching("色空", probe).unwrap().is_none());
}

#[test]
fn skips_pathologically_long_input() {
    let long = "空".repeat(61);
    let probe = |_: &str| Ok(true);
    assert!(longest_matching(&long, probe).unwrap().is_none());
}

#[test]
fn probes_are_bounded_for_a_long_query() {
    use std::cell::Cell;
    let calls = Cell::new(0usize);
    let probe = |_: &str| {
        calls.set(calls.get() + 1);
        Ok(false)
    };
    let _ = longest_matching(&"空".repeat(40), probe).unwrap();
    assert!(
        calls.get() < 400,
        "binary search over lengths must stay well under the O(n^2) naive scan, got {}",
        calls.get()
    );
}

#[test]
fn propagates_probe_error() {
    let probe = |_: &str| Err(anyhow::anyhow!("db exploded"));
    assert!(longest_matching("色即是空", probe).is_err());
}
```

`anyhow` 已在 `[dependencies]`,集成测试可直接使用,**不要**往 `[dev-dependencies]` 重复添加(已实证确认)。本任务不需要改 `Cargo.toml`。

- [ ] **Step 2: 运行,确认失败**

Run: `rustup run 1.96.0 cargo test --test fallback`
Expected: 除 `returns_none_when_nothing_matches`、`declines_queries_shorter_than_three_chars`、`skips_pathologically_long_input`、`probes_are_bounded_for_a_long_query` 外 FAIL —— 桩恒返回 `Ok(None)`。

- [ ] **Step 3: 实现**

把 `src/search/fallback.rs` 的函数体替换为:

```rust
pub fn longest_matching(
    norm_query: &str,
    probe: impl Fn(&str) -> Result<bool>,
) -> Result<Option<FallbackInfo>> {
    let chars: Vec<char> = norm_query.chars().collect();
    let n = chars.len();
    // n < 3 leaves no proper substring of length >= 2 to try.
    if n < 3 || n > MAX_FALLBACK_CHARS {
        return Ok(None);
    }

    let substring = |start: usize, len: usize| -> String {
        chars[start..start + len].iter().collect::<String>()
    };
    // Scans starts left to right, so the earliest start wins at a given length.
    let first_hit = |len: usize| -> Result<Option<usize>> {
        for start in 0..=(n - len) {
            if probe(&substring(start, len))? {
                return Ok(Some(start));
            }
        }
        Ok(None)
    };

    // "Some substring of length L matches" is monotonically decreasing in L,
    // because every substring of a matching substring also matches. Binary
    // search the largest feasible L instead of scanning every length.
    let mut best: Option<(usize, usize)> = None;
    let (mut lo, mut hi) = (2usize, n - 1);
    while lo <= hi {
        let mid = lo + (hi - lo) / 2;
        match first_hit(mid)? {
            Some(start) => {
                best = Some((start, mid));
                lo = mid + 1;
            }
            None => {
                hi = mid - 1;
            }
        }
    }

    Ok(best.map(|(start, len)| FallbackInfo {
        matched_substring: substring(start, len),
        char_len: len,
    }))
}
```

`hi = mid - 1` 不会下溢:`lo` 起始为 2,所以 `mid >= 2`。

- [ ] **Step 4: 运行,确认通过**

Run: `rustup run 1.96.0 cargo test --test fallback`
Expected: 8 passed。

- [ ] **Step 5: clippy + 提交**

Run: `rustup run 1.96.0 cargo clippy --all-targets -- -D warnings`

```bash
git add src/search/fallback.rs tests/fallback.rs
git commit -m "feat(search): 最长可命中子串回退,按长度二分探测"
```

---

### Task 5: 句读切分(agent-2)

**Files:**
- Modify: `src/search/split.rs`
- Test: `tests/split.rs`(创建)

**Interfaces:**
- Produces: `split::split_sentences` 的**实现**(签名 Task 1 已固定)、`split::SPLIT_CHARS`

- [ ] **Step 1: 写失败测试**

创建 `tests/split.rs`:

```rust
use fojin_cli::search::split::{split_sentences, MAX_SEGMENTS};

/// Stand-in for the real predicate (normalized length >= 2).
fn keep_two_chars(s: &str) -> bool {
    s.chars().count() >= 2
}

#[test]
fn splits_on_sentence_punctuation() {
    let out = split_sentences("觀自在菩薩，行深般若波羅蜜多時。照見五蘊皆空", keep_two_chars);
    assert_eq!(
        out.segments,
        vec!["觀自在菩薩", "行深般若波羅蜜多時", "照見五蘊皆空"]
    );
    assert_eq!(out.truncated, 0);
}

#[test]
fn consecutive_punctuation_yields_no_empty_segments() {
    let out = split_sentences("色即是空。。。受想行識", keep_two_chars);
    assert_eq!(out.segments, vec!["色即是空", "受想行識"]);
}

#[test]
fn punctuation_only_input_yields_nothing() {
    let out = split_sentences("，。！？", keep_two_chars);
    assert!(out.segments.is_empty());
}

#[test]
fn drops_segments_the_predicate_rejects() {
    let out = split_sentences("色即是空，也。受想行識", keep_two_chars);
    assert_eq!(
        out.segments,
        vec!["色即是空", "受想行識"],
        "the single-character 也 must be dropped"
    );
}

#[test]
fn does_not_split_on_in_sentence_marks() {
    // Book-title brackets, quotes, parens and the interpunct appear inside a
    // sentence; splitting there would cut phrases in half.
    let out = split_sentences("如《心經》所說「色即是空」（略）·如是", keep_two_chars);
    assert_eq!(out.segments.len(), 1, "got: {:?}", out.segments);
}

#[test]
fn splits_on_newlines_and_ascii_punctuation() {
    let out = split_sentences("色即是空\n受想行識,無眼耳鼻舌身意", keep_two_chars);
    assert_eq!(out.segments, vec!["色即是空", "受想行識", "無眼耳鼻舌身意"]);
}

#[test]
fn trims_surrounding_whitespace() {
    let out = split_sentences("  色即是空 ，  受想行識  ", keep_two_chars);
    assert_eq!(out.segments, vec!["色即是空", "受想行識"]);
}

#[test]
fn caps_segments_and_reports_the_overflow() {
    let raw = (0..MAX_SEGMENTS + 3)
        .map(|i| format!("第{i}句子"))
        .collect::<Vec<_>>()
        .join("。");
    let out = split_sentences(&raw, keep_two_chars);
    assert_eq!(out.segments.len(), MAX_SEGMENTS);
    assert_eq!(out.truncated, 3, "the overflow count must be reported");
}
```

- [ ] **Step 2: 运行,确认失败**

Run: `rustup run 1.96.0 cargo test --test split`
Expected: FAIL —— 桩返回空 `segments`。

- [ ] **Step 3: 实现**

把 `src/search/split.rs` 替换为:

```rust
/// Max sentences processed when auto-splitting; the excess is reported, never
/// silently dropped.
pub const MAX_SEGMENTS: usize = 20;

/// Sentence-level punctuation only. Book-title brackets, quotes, parentheses
/// and the interpunct are deliberately absent: they occur mid-sentence, and
/// breaking there would cut phrases in half.
pub const SPLIT_CHARS: &str = "，。；：！？、,.;:!?\n\r";

pub struct SplitOutcome {
    pub segments: Vec<String>,
    pub truncated: usize,
}

/// Split raw (un-normalized) input into sentences. Splitting must happen before
/// `normalize()`, which strips the very punctuation this relies on.
///
/// `keep` decides whether a trimmed segment survives; callers pass a
/// normalized-length predicate, which keeps this function database-free.
pub fn split_sentences(raw: &str, keep: impl Fn(&str) -> bool) -> SplitOutcome {
    let kept: Vec<String> = raw
        .split(|c: char| SPLIT_CHARS.contains(c))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter(|s| keep(s))
        .map(str::to_string)
        .collect();
    let truncated = kept.len().saturating_sub(MAX_SEGMENTS);
    let mut segments = kept;
    segments.truncate(MAX_SEGMENTS);
    SplitOutcome {
        segments,
        truncated,
    }
}
```

- [ ] **Step 4: 运行,确认通过**

Run: `rustup run 1.96.0 cargo test --test split`
Expected: 8 passed。

- [ ] **Step 5: 提交**

```bash
git add src/search/split.rs tests/split.rs
git commit -m "feat(search): 按句读切分长输入"
```

---

### Task 6: 编排 —— 零命中时切分重查并逐段回退(agent-2)

**Files:**
- Modify: `src/search/mod.rs`
- Test: `tests/search.rs`(创建)

**Interfaces:**
- Consumes: `split::split_sentences`、`fallback::longest_matching`、`query::search`、`normalize::{normalize, load_norm_map, MIN_QUERY_CHARS}`
- Produces: `search::run` 的完整三路径行为

- [ ] **Step 1: 写失败测试**

创建 `tests/search.rs`:

```rust
use fojin_cli::schema::init_schema;
use fojin_cli::search::{run, SearchRequest};
use rusqlite::{params, Connection};

fn fixture() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    init_schema(&conn).unwrap();
    for (zt, zn, f) in [
        ("觀自在菩薩", "观自在菩萨", "avalokiteśvara"),
        ("照見五蘊皆空", "照见五蕴皆空", "pañcaskandhāḥ śūnyāḥ"),
    ] {
        conn.execute(
            "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
             VALUES (?1,?2,'sa',?3,1.0,'T0251','心經',1)",
            params![zt, zn, f],
        )
        .unwrap();
    }
    for (from, to) in [("觀", "观"), ("薩", "萨"), ("見", "见"), ("蘊", "蕴")] {
        conn.execute(
            "INSERT INTO norm_map(from_char,to_char) VALUES (?1,?2)",
            params![from, to],
        )
        .unwrap();
    }
    conn
}

fn req<'a>(raw: &'a str, no_split: bool) -> SearchRequest<'a> {
    SearchRequest {
        raw,
        langs: None,
        top: 3,
        limit: Some(10),
        from: None,
        no_split,
    }
}

#[test]
fn hit_path_sets_no_segments_and_no_fallback() {
    let conn = fixture();
    let out = run(&conn, &req("觀自在菩薩", false)).unwrap();
    assert_eq!(out.total, 1);
    assert!(out.segments.is_none(), "a hit must not trigger splitting");
    assert!(out.fallback.is_none());
}

#[test]
fn zero_hit_long_input_splits_and_merges_segment_hits() {
    let conn = fixture();
    // Neither half spans one stored segment, so the whole string finds nothing;
    // each sentence on its own does.
    let out = run(&conn, &req("觀自在菩薩，照見五蘊皆空", false)).unwrap();
    let segments = out.segments.expect("split must have engaged");
    assert_eq!(segments.len(), 2);
    assert!(segments.iter().all(|s| s.matched));
    assert_eq!(out.total, 2, "top-level groups merge both segments' hits");
}

#[test]
fn no_split_flag_suppresses_splitting() {
    let conn = fixture();
    let out = run(&conn, &req("觀自在菩薩，照見五蘊皆空", true)).unwrap();
    assert!(out.segments.is_none());
    assert_eq!(out.total, 0);
}

#[test]
fn merged_groups_are_deduplicated() {
    let conn = fixture();
    // Both sentences hit the same stored group, which must appear once.
    let out = run(&conn, &req("觀自在，自在菩薩", false)).unwrap();
    assert_eq!(out.total, 1, "same group reached twice must dedupe");
}

#[test]
fn empty_segment_gets_its_own_fallback() {
    let conn = fixture();
    // Segment 2 matches nothing whole, but 觀自在菩薩 (normalized 观自在菩萨)
    // sits inside it and IS in the fixture — that is what fallback must find.
    let out = run(&conn, &req("涅槃寂靜，觀自在菩薩摩訶薩", false)).unwrap();
    let segments = out.segments.unwrap();
    let miss = segments
        .iter()
        .find(|s| s.text == "觀自在菩薩摩訶薩")
        .unwrap();
    assert!(!miss.matched);
    let fb = miss
        .fallback
        .as_ref()
        .expect("a segment with a matching substring must carry a fallback");
    assert_eq!(fb.matched_substring, "观自在菩萨");
    assert_eq!(fb.char_len, 5);
}

#[test]
fn segment_with_no_matching_substring_has_no_fallback() {
    let conn = fixture();
    let out = run(&conn, &req("觀自在菩薩，涅槃寂靜無為", false)).unwrap();
    let segments = out.segments.unwrap();
    let miss = segments.iter().find(|s| !s.matched).unwrap();
    assert_eq!(miss.text, "涅槃寂靜無為");
    assert!(
        miss.fallback.is_none(),
        "nothing in this fixture matches any substring of 涅槃寂靜無為"
    );
}

#[test]
fn unsplittable_zero_hit_query_falls_back_on_the_whole_string() {
    let conn = fixture();
    // No sentence punctuation, so there is nothing to split; the whole string
    // gets the substring fallback, and 觀自在菩薩 is present in the fixture.
    let out = run(&conn, &req("觀自在菩薩摩訶薩", false)).unwrap();
    assert!(out.segments.is_none());
    let fb = out.fallback.expect("whole-string fallback must engage");
    assert_eq!(fb.matched_substring, "观自在菩萨");
    assert_eq!(fb.char_len, 5);
}

#[test]
fn fallback_substring_is_reported_in_normalized_form() {
    let conn = fixture();
    let out = run(&conn, &req("觀自在菩薩摩訶薩", false)).unwrap();
    let fb = out.fallback.unwrap();
    assert!(
        fb.matched_substring.contains('观'),
        "fallback reports the normalized (folded) form: {}",
        fb.matched_substring
    );
}

#[test]
fn segment_display_is_capped_but_total_is_honest() {
    let conn = fixture();
    for i in 0..5 {
        conn.execute(
            "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
             VALUES (?1,?2,'sa','x',1.0,?3,'注疏',1)",
            params![
                format!("觀自在菩薩釋{i}"),
                format!("观自在菩萨释{i}"),
                format!("T{:04}", 900 + i)
            ],
        )
        .unwrap();
    }
    let out = run(&conn, &req("觀自在菩薩，涅槃寂靜無為", false)).unwrap();
    let segments = out.segments.unwrap();
    let hit = segments.iter().find(|s| s.matched).unwrap();
    assert_eq!(hit.groups.len(), 3, "per-segment display cap is 3");
    assert_eq!(hit.total, 6, "but the true count is reported");
}
```

- [ ] **Step 2: 运行,确认失败**

Run: `rustup run 1.96.0 cargo test --test search`
Expected: 除 `hit_path_sets_no_segments_and_no_fallback` 与 `no_split_flag_suppresses_splitting` 外全部 FAIL。

- [ ] **Step 3: 实现编排**

把 `src/search/mod.rs` 的 `run` 替换为下面这一组函数(`from` 分支保持 Task 3 落地的样子):

```rust
type GroupKey = (String, Option<String>, Option<i64>);

fn group_key(g: &MatchGroup) -> GroupKey {
    (g.zh_text.clone(), g.cbeta_id.clone(), g.juan_num)
}

pub fn run(conn: &Connection, req: &SearchRequest) -> Result<SearchOutcome> {
    if let Some(from_lang) = req.from {
        let needle = req.raw.trim();
        if needle.chars().count() < query::MIN_FOREIGN_QUERY_CHARS {
            anyhow::bail!(
                "反向查询至少需要 {} 个字符;更短的串会命中过多,无对读价值",
                query::MIN_FOREIGN_QUERY_CHARS
            );
        }
        let groups = query::search_foreign(conn, from_lang, needle, req.langs, req.top)?;
        return Ok(SearchOutcome::plain(groups, req.limit));
    }

    let map = normalize::load_norm_map(conn)?;
    let raw = req.raw.trim();
    let norm = normalize::normalize(raw, &map);
    normalize::validate_query_length(&norm)?;

    let groups = query::search(conn, &norm, req.langs, req.top)?;
    if !groups.is_empty() {
        return Ok(SearchOutcome::plain(groups, req.limit));
    }

    let probe = |candidate: &str| -> Result<bool> {
        Ok(!query::search(conn, candidate, req.langs, 1)?.is_empty())
    };

    if !req.no_split {
        let keep = |seg: &str| {
            normalize::normalize(seg, &map).chars().count() >= normalize::MIN_QUERY_CHARS
        };
        let split = split::split_sentences(raw, keep);
        if split.segments.len() >= 2 {
            return split_search(conn, req, &map, &split, &probe);
        }
    }

    Ok(SearchOutcome {
        groups: Vec::new(),
        total: 0,
        segments: None,
        fallback: fallback::longest_matching(&norm, &probe)?,
        truncated_segments: 0,
    })
}

fn split_search(
    conn: &Connection,
    req: &SearchRequest,
    map: &normalize::NormMap,
    split: &split::SplitOutcome,
    probe: &impl Fn(&str) -> Result<bool>,
) -> Result<SearchOutcome> {
    let mut merged: Vec<MatchGroup> = Vec::new();
    let mut seen: std::collections::HashSet<GroupKey> = std::collections::HashSet::new();
    let mut segments: Vec<SegmentResult> = Vec::new();

    for text in &split.segments {
        let seg_norm = normalize::normalize(text, map);
        let seg_groups = query::search(conn, &seg_norm, req.langs, req.top)?;
        let total = seg_groups.len();

        // Stable append-then-dedupe: segment order decides position, so output
        // never drifts with the number of segments.
        for g in &seg_groups {
            if seen.insert(group_key(g)) {
                merged.push(g.clone());
            }
        }

        let fb = if total == 0 {
            fallback::longest_matching(&seg_norm, probe)?
        } else {
            None
        };
        // `--all` (limit None) lifts the per-segment cap.
        let shown = match req.limit {
            Some(n) => n.min(SEGMENT_GROUP_CAP).min(total),
            None => total,
        };
        let mut groups = seg_groups;
        groups.truncate(shown);

        segments.push(SegmentResult {
            text: text.clone(),
            matched: total > 0,
            total,
            groups,
            fallback: fb,
        });
    }

    let total = merged.len();
    let shown = match req.limit {
        Some(n) => n.min(total),
        None => total,
    };
    merged.truncate(shown);

    Ok(SearchOutcome {
        groups: merged,
        total,
        segments: Some(segments),
        fallback: None,
        truncated_segments: split.truncated,
    })
}
```

- [ ] **Step 4: 运行,确认通过**

Run: `rustup run 1.96.0 cargo test --test search --test golden`
Expected: 全部 PASS。golden 必须仍通过 —— 有命中时第 3 步就返回了。

- [ ] **Step 5: clippy + 提交**

Run: `rustup run 1.96.0 cargo clippy --all-targets -- -D warnings`

```bash
git add src/search/mod.rs tests/search.rs
git commit -m "feat(search): 零命中时按句切分重查,逐段回退"
```

---

### Task 7: 分句与回退的渲染 + JSON 契约(agent-2)

**Files:**
- Modify: `src/render.rs`
- Test: `tests/render.rs`(追加)

**Interfaces:**
- Consumes: `search::{SearchOutcome, SegmentResult, FallbackInfo, SCHEMA_VERSION}`
- Produces: `render::render_outcome_human`、`render::render_outcome_json` 的完整实现

**回退的呈现方式:** 只给一行提示,**不**再查一次把结果铺开。`FallbackInfo` 只带子串和字数,渲染成"可单独查询"的引导,避免为一个降级结果付第二轮查询代价。

**范围说明:** `schema_version` 只加在 `parallel` 的输出(`render_outcome_json`)。`cite` 仍走 `render_json`,输出不变——改它超出本 spec 范围,留作后续。

- [ ] **Step 1: 写失败测试**

追加到 `tests/render.rs`:

```rust
use fojin_cli::model::{MatchGroup, Parallel};
use fojin_cli::render::{render_outcome_human, render_outcome_json};
use fojin_cli::search::{FallbackInfo, SearchOutcome, SegmentResult};

fn outcome_with_segments() -> SearchOutcome {
    SearchOutcome {
        groups: vec![heart()],
        total: 1,
        segments: Some(vec![
            SegmentResult {
                text: "色即是空".into(),
                matched: true,
                total: 1,
                groups: vec![heart()],
                fallback: None,
            },
            SegmentResult {
                text: "度一切苦厄".into(),
                matched: false,
                total: 0,
                groups: vec![],
                fallback: Some(FallbackInfo {
                    matched_substring: "一切苦".into(),
                    char_len: 3,
                }),
            },
        ]),
        fallback: None,
        truncated_segments: 0,
    }
}

#[test]
fn human_split_output_labels_each_segment() {
    let out = render_outcome_human(&outcome_with_segments(), None);
    assert!(out.contains("已按句切分查询"), "got: {out}");
    assert!(out.contains("--no-split"));
    assert!(out.contains("【色即是空】"));
    assert!(out.contains("【度一切苦厄】"));
    assert!(out.contains("梵  rūpaṃ śūnyatā  [MITRA 0.91]"));
}

#[test]
fn human_segment_fallback_points_at_the_matching_substring() {
    let out = render_outcome_human(&outcome_with_segments(), None);
    assert!(out.contains("其中「一切苦」"), "got: {out}");
    assert!(out.contains("可单独查询"));
}

#[test]
fn human_reports_truncated_segments_instead_of_dropping_silently() {
    let mut outcome = outcome_with_segments();
    outcome.truncated_segments = 4;
    let out = render_outcome_human(&outcome, None);
    assert!(out.contains("还有 4 句未处理"), "got: {out}");
}

#[test]
fn human_whole_string_fallback_is_a_single_hint() {
    let outcome = SearchOutcome {
        groups: vec![],
        total: 0,
        segments: None,
        fallback: Some(FallbackInfo {
            matched_substring: "色不异空".into(),
            char_len: 4,
        }),
        truncated_segments: 0,
    };
    let out = render_outcome_human(&outcome, None);
    assert!(out.contains("未找到对齐"));
    assert!(out.contains("其中「色不异空」(4 字) 有对齐"), "got: {out}");
}

#[test]
fn json_keeps_the_existing_top_level_contract() {
    let out = render_outcome_json(&outcome_with_segments());
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["matched"], serde_json::json!(true));
    assert_eq!(v["total"], serde_json::json!(1));
    assert_eq!(v["shown"], serde_json::json!(1));
    assert!(v["groups"].is_array());
    assert_eq!(v["schema_version"], serde_json::json!(1));
}

#[test]
fn json_segments_appear_only_when_splitting_happened() {
    let with = render_outcome_json(&outcome_with_segments());
    let v: serde_json::Value = serde_json::from_str(&with).unwrap();
    assert!(v["segments"].is_array());
    assert_eq!(v["segments"][1]["fallback"]["matched_substring"], serde_json::json!("一切苦"));
    assert_eq!(v["segments"][1]["total"], serde_json::json!(0));

    let plain = SearchOutcome {
        groups: vec![heart()],
        total: 1,
        segments: None,
        fallback: None,
        truncated_segments: 0,
    };
    let v: serde_json::Value = serde_json::from_str(&render_outcome_json(&plain)).unwrap();
    assert!(v.get("segments").is_none(), "no segments key on the hit path");
    assert!(v.get("fallback").is_none());
}
```

- [ ] **Step 2: 运行,确认失败**

Run: `rustup run 1.96.0 cargo test --test render`
Expected: 新增测试 FAIL —— 现有转发实现不产出分句小节,JSON 里也没有 `schema_version`。

- [ ] **Step 3: 实现渲染**

把 `src/render.rs` 里 Task 1 加的两个转发函数替换为:

```rust
use crate::search::{SearchOutcome, SCHEMA_VERSION};

fn render_fallback_hint(fallback: &crate::search::FallbackInfo) -> String {
    format!(
        "其中「{}」({} 字) 有对齐,可单独查询\n",
        fallback.matched_substring, fallback.char_len
    )
}

pub fn render_outcome_human(outcome: &SearchOutcome, langs: Option<&[String]>) -> String {
    let Some(segments) = &outcome.segments else {
        if let Some(fallback) = &outcome.fallback {
            let mut out = String::from("未找到对齐;");
            out.push_str(&render_fallback_hint(fallback));
            out.push_str(&format!("\n{FOOTER}\n"));
            return out;
        }
        let hidden = outcome.total - outcome.groups.len();
        return render_human(&outcome.groups, langs, hidden);
    };

    let mut out = String::from("整串未找到对齐,已按句切分查询(加 --no-split 关闭):\n");
    for segment in segments {
        out.push_str(&format!("\n【{}】", segment.text));
        if segment.matched {
            let hidden = segment.total - segment.groups.len();
            let suffix = if hidden > 0 {
                format!("{} 组(另有 {hidden} 组,加 --all 查看)\n", segment.total)
            } else {
                format!("{} 组\n", segment.total)
            };
            out.push_str(&suffix);
            out.push_str(&render_groups(&segment.groups, langs));
        } else {
            out.push_str("未找到对齐");
            match &segment.fallback {
                Some(fallback) => {
                    out.push(';');
                    out.push_str(&render_fallback_hint(fallback));
                }
                None => out.push('\n'),
            }
        }
    }
    if outcome.truncated_segments > 0 {
        out.push_str(&format!(
            "\n(超出 {} 句上限,还有 {} 句未处理)\n",
            crate::search::split::MAX_SEGMENTS,
            outcome.truncated_segments
        ));
    }
    out.push_str(&format!("\n{FOOTER}\n"));
    out
}

pub fn render_outcome_json(outcome: &SearchOutcome) -> String {
    let mut v = serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "matched": outcome.total > 0,
        "total": outcome.total,
        "shown": outcome.groups.len(),
        "groups": outcome.groups,
    });
    if let Some(segments) = &outcome.segments {
        v["segments"] = serde_json::json!(segments);
    }
    if let Some(fallback) = &outcome.fallback {
        v["fallback"] = serde_json::json!(fallback);
    }
    if outcome.truncated_segments > 0 {
        v["truncated_segments"] = serde_json::json!(outcome.truncated_segments);
    }
    serde_json::to_string_pretty(&v).unwrap()
}
```

- [ ] **Step 4: 从 `render_human` 抽出可复用的分组渲染**

`render_outcome_human` 需要在不带页脚的情况下渲染一批分组。把 `render_human` 中从 `for (gi, g) in groups.iter().enumerate()` 到该循环结束的整段搬进新函数,`render_human` 改为调用它:

```rust
/// Renders groups only — no footer, no "还有 N 组" line. Shared by the plain
/// and the split renderers.
fn render_groups(groups: &[MatchGroup], langs: Option<&[String]>) -> String {
    let display: Vec<String> = match langs {
        Some(filter) if !filter.is_empty() => filter.to_vec(),
        _ => DISPLAY_LANGS.iter().map(|s| s.to_string()).collect(),
    };
    let mut out = String::new();
    for (gi, g) in groups.iter().enumerate() {
        if gi > 0 {
            out.push('\n');
        }
        let src = match (&g.title_zh, &g.cbeta_id, g.juan_num) {
            (Some(t), Some(c), Some(j)) => format!("  (《{t}》{c} 卷{j})"),
            (Some(t), Some(c), None) => format!("  (《{t}》{c})"),
            _ => String::new(),
        };
        out.push_str(&format!("汉  {}{}\n", g.zh_text, src));

        for code in &display {
            let items: Vec<_> = g.parallels.iter().filter(|p| &p.lang == code).collect();
            if items.is_empty() {
                out.push_str(&format!("{}  (无对齐)\n", lang_label(code)));
            } else {
                for p in items {
                    out.push_str(&format!(
                        "{}  {}{}\n",
                        lang_label(code),
                        p.text,
                        conf_tag(p.confidence)
                    ));
                }
            }
        }
        if langs.is_none() {
            for p in &g.parallels {
                if !display.iter().any(|d| d == &p.lang) {
                    out.push_str(&format!(
                        "{}  {}{}\n",
                        lang_label(&p.lang),
                        p.text,
                        conf_tag(p.confidence)
                    ));
                }
            }
        }
    }
    out
}

pub fn render_human(groups: &[MatchGroup], langs: Option<&[String]>, hidden: usize) -> String {
    if groups.is_empty() {
        return "未找到对齐\n".to_string();
    }
    let mut out = render_groups(groups, langs);
    if hidden > 0 {
        out.push_str(&format!("\n… 还有 {hidden} 组匹配,加 --all 查看全部\n"));
    }
    out.push_str(&format!("\n{FOOTER}\n"));
    out
}
```

- [ ] **Step 5: 运行,确认通过(golden 是关键)**

Run: `rustup run 1.96.0 cargo test`
Expected: 全部 PASS。**`tests/golden.rs` 必须仍然通过** —— 抽出 `render_groups` 是纯重构,一个字节都不能变。

- [ ] **Step 6: clippy + 提交**

Run: `rustup run 1.96.0 cargo clippy --all-targets -- -D warnings`

```bash
git add src/render.rs tests/render.rs
git commit -m "feat(render): 分句小节与回退提示渲染,JSON 加 schema_version/segments"
```

---

### Task 8: 端到端验证、真实数据冒烟、文档(主会话,合并后)

**Files:**
- Modify: `tests/command.rs`
- Modify: `README.md`
- Modify: `CHANGELOG.md`

- [ ] **Step 1: 写端到端测试**

追加到 `tests/command.rs`:

```rust
fn write_split_fixture(dir: &std::path::Path) {
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
    for (zt, zn, f) in [
        ("色即是空", "色即是空", "rūpaṃ śūnyatā"),
        ("受想行識", "受想行识", "vedanā saṃjñā"),
    ] {
        conn.execute(
            "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)
             VALUES (?1,?2,'sa',?3,1.0,'T0251','心經',1)",
            rusqlite::params![zt, zn, f],
        )
        .unwrap();
    }
}

#[test]
fn long_query_splits_and_stdout_stays_pure_json() {
    let dir = tempfile::tempdir().unwrap();
    write_split_fixture(dir.path());

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args([
            "parallel",
            "色即是空，受想行識",
            "--json",
            "--offline",
            "--data-dir",
        ])
        .arg(dir.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("stdout must be pure JSON");
    assert_eq!(v["schema_version"], serde_json::json!(1));
    assert_eq!(v["segments"].as_array().unwrap().len(), 2);
}

#[test]
fn reverse_lookup_finds_chinese_from_sanskrit() {
    let dir = tempfile::tempdir().unwrap();
    write_split_fixture(dir.path());

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fojin"))
        .args([
            "parallel", "śūnyatā", "--from", "sa", "--json", "--offline", "--data-dir",
        ])
        .arg(dir.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(output.stdout).unwrap()).unwrap();
    assert_eq!(v["matched"], serde_json::json!(true));
    assert_eq!(v["groups"][0]["zh_text"], serde_json::json!("色即是空"));
}
```

`serde_json` 已在 `[dependencies]`,集成测试可直接以 `serde_json::` 路径使用,无需 `use`,也无需改 `Cargo.toml`。

- [ ] **Step 2: 运行全量测试**

Run: `rustup run 1.96.0 cargo test`
Expected: 全部 PASS,包含 `tests/golden.rs`。

- [ ] **Step 3: 真实数据冒烟(必须实测,不得跳过)**

```bash
rustup run 1.96.0 cargo build --release
B=./target/release/fojin

# 1) 反查:计时,确认没有跌出 spec 的 2s 退路阈值
time $B parallel "śūnyatā" --from sa --offline --limit 3

# 2) 长句切分:改动前返回"未找到对齐"的那一句
time $B parallel "觀自在菩薩行深般若波羅蜜多時，照見五蘊皆空，度一切苦厄" --offline

# 3) 回退:无标点、整串必然零命中
time $B parallel "觀自在菩薩摩訶薩清淨法身" --offline

# 4) 回归:有命中的查询,输出必须与改动前一致
$B parallel "色即是空" --offline
```

Expected:
1. 反查有结果且**热缓存耗时 < 2s**。超过 2s 就按 spec 的退路改回 SQL `instr` 精确匹配,并在无命中时提示"反查区分大小写与变音符号"——这是 spec 明确写好的分支,不是失败。
2. 输出分句小节,至少一句有命中。
3. 输出"未找到对齐;其中「…」(N 字) 有对齐,可单独查询"。
4. 与本计划 Task 1 golden 里的格式一致(经名/卷号来自真实数据)。

把四条命令的实际输出与耗时记录下来,作为完成证据。

- [ ] **Step 4: 更新 README**

在「功能 / Usage」的 flag 表格中加入两行:

```markdown
| `--from <lang>` | 反向查询:用该语种原文查汉文对应(sa/bo),至少 3 字符 | — |
| `--no-split` | 零命中时不自动按句切分重查 | — |
```

在「输入规则与匹配方式」小节,把"整段文字超出分段长度,基本查不到——请拆成短句分别查"改写为:

```markdown
- 匹配为**整串子串匹配**(FTS5 trigram):查询串须连续完整出现在某条经文分段中。4~12 字的短语/名句命中最佳。
- **整串查不到时会自动按句切分重查**(加 `--no-split` 关闭),并对仍无命中的分句给出该句中最长的可命中子串。
- 输入端不再限于汉文:`--from sa` / `--from bo` 可用梵文转写或藏文反查汉文对应(不区分大小写与首字母),反查不做切分与回退。
```

在「For AI Agents」小节,把"边界:无语义搜索、无巴利、无翻译"一行下方补一句:

```markdown
- `--json` 输出含 `schema_version`(当前为 `1`);切分发生时额外带 `segments[]`,整串回退时额外带 `fallback{}`,`matched`/`total`/`shown`/`groups` 四个字段语义不变。
```

- [ ] **Step 5: 更新 CHANGELOG**

在 `## [0.3.0] - Unreleased` 的列表中追加:

```markdown
- 检索能力:新增 `--from` 反向查询(梵/藏 → 汉,Unicode 大小写折叠);零命中时自动按句切分重查(`--no-split` 关闭);仍零命中时给出最长可命中子串。`--lang` / `--from` 的未知语种代码现在报用法错误,而非静默返回空结果。
```

- [ ] **Step 6: 最终验证 + 提交**

Run: `rustup run 1.96.0 cargo test && rustup run 1.96.0 cargo clippy --all-targets -- -D warnings && rustup run 1.96.0 cargo fmt --check`
Expected: 全部通过。

```bash
git add tests/command.rs README.md CHANGELOG.md
git commit -m "docs: 记录反查/切分/回退三项检索能力"
```
