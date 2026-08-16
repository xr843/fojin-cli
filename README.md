# fojin-cli

[![CI](https://github.com/xr843/fojin-cli/actions/workflows/ci.yml/badge.svg)](https://github.com/xr843/fojin-cli/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/fojin-cli.svg)](https://crates.io/crates/fojin-cli)
[![release](https://img.shields.io/github/v/release/xr843/fojin-cli?filter=v*&label=release)](https://github.com/xr843/fojin-cli/releases/latest)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#许可)

**离线 · 无需登录 · 单二进制。** 给一段汉文,查它在梵/藏正典中的平行文本。命中时本地查询毫秒级(实测典型 2 ms,数千组命中的高频词约 0.3 s);零命中并触发切分与回退时更慢,实测单串约 13 ms、20 句的长输入约 0.33 s。

*English readers: see the [English summary](#english-summary) at the bottom.*

```
$ fojin parallel "色即是空"
汉  色不異空，空不異色，色即是空，空即是色；  (《般若波羅蜜多心經》T0251 卷1)
梵  śūnyat'aiva rūpaṃ, rūpān na pṛthak śūnyatā …
藏  གཟུགས་ལས་སྟོང་པ་ཉིད་གཞན་མ་ཡིན༏ …

… 还有 38 组匹配,加 --all 查看全部

对齐数据:Dharmamitra MITRA-parallel · CC BY-SA 4.0 · 经 fojin 归一化并打包
https://creativecommons.org/licenses/by-sa/4.0/ · 完整上下文见 https://fojin.app
```

> 这不是 fojin.app 的账号客户端 —— 它不联网(首次下载数据后)、不需要登录。

## 安装

**没有 Rust 环境** —— 一行脚本装好对应平台的预编译二进制(Linux x64 / macOS ARM+Intel)：

```bash
curl -fsSL https://raw.githubusercontent.com/xr843/fojin-cli/master/install.sh | sh
```

脚本在解压前用该 release 的 `SHA256SUMS` 核对 archive,校验不过就中止,不会落地二进制。

**有 Rust** —— 要求 **1.95+**(MSRV 1.95),装好后的命令名是 `fojin`：

```bash
cargo install fojin-cli --locked
# 或直接从源码:cargo install --git https://github.com/xr843/fojin-cli --locked
```

**Windows 或想手动下载** —— 从 [Releases](https://github.com/xr843/fojin-cli/releases/latest)
取各平台二进制(含 Windows x64 zip),并一并下载**同一个 release** 的 `SHA256SUMS` 自行核对。
校验合同、环境变量与各平台的核对命令见 [`docs/install-verification.md`](docs/install-verification.md)。

首次运行 `fojin parallel` 会自动下载对齐数据集(约 183 MB,带进度显示,见下方「数据集」),之后完全离线。

## 功能 / Usage

```
fojin parallel "色即是空"          # 位置参数
echo "色即是空" | fojin parallel    # 或从 stdin 读取
```

| flag | 说明 | 默认值 |
| --- | --- | --- |
| `--lang sa,bo` | 只看指定语种,逗号分隔 | 显示 sa/bo |
| `--top N` | 每个语种最多显示 N 条平行(N ≥ 1) | `3` |
| `--limit N` | 最多显示 N 组匹配(N ≥ 1) | `10` |
| `--all` | 显示全部匹配组,忽略 `--limit` | — |
| `--json` | 输出机器可读 JSON | — |
| `--data-dir <path>` | 指定数据目录,覆盖默认缓存位置 | 系统缓存目录 |
| `--offline` | 不联网;本地数据缺失时直接报错(而非下载) | — |
| `--from <lang>` | 反向查询:用该语种原文查汉文对应(sa/bo),至少 3 字符 | — |
| `--no-split` | 零命中时不自动按句切分重查 | — |

示例:

```bash
# 只看梵文与藏文平行,每语最多 1 条
fojin parallel "色即是空" --lang sa,bo --top 1

# 显示全部匹配组(忽略 --limit)
fojin parallel "色即是空" --all

# 指定数据目录 + 离线模式(适合脚本 / CI / 容器)
fojin parallel "色即是空" --data-dir ./data --offline

# JSON 输出,便于管道处理
fojin parallel "色即是空" --json
```

`--json` 输出结构:

```json
{
  "groups": [
    {
      "cbeta_id": "T0310",
      "juan_num": 2,
      "parallels": [
        {
          "confidence": 1.0,
          "lang": "bo",
          "text": "ཁྱེད་མཚན་མའི་འདུ་ཤེས་མ་བྱེད་ཅིག་།"
        }
      ],
      "title_zh": "大寶積經",
      "zh_text": "勿於處所生住著心，應無所住。"
    }
  ],
  "matched": true,
  "schema_version": 1,
  "shown": 1,
  "total": 10
}
```

(以上是 `fojin parallel "应无所住" --json --top 1 --limit 1` 的完整真实输出,未作删改;
`--top 1` 只保留每语一条,`--limit 1` 只保留 10 组匹配中的第一组,字段按字母序输出。)

## 其他子命令

```bash
fojin texts "心经"        # 模糊查经名(简繁均可) → Taishō 编号 + 各语对齐条数
fojin cite T0251          # 按编号列出一部经的对齐,经文顺序;--juan N 限定卷
fojin data status         # 本地数据状态(位置/大小/版本/行数统计)
fojin data clean          # 删除本地数据,释放 578 MB
fojin data update         # 重新下载数据(覆盖本地)
fojin data verify         # 校验版本、SQLite 与 FTS 完整性
```

`texts` 与 `cite` 支持与 `parallel` 一致的 `--json` / `--data-dir` / `--offline`;
`cite` 另有 `--lang` / `--top` / `--limit` / `--all`。典型工作流:`texts` 找到编号 → `cite` 通读对齐。

按经号查询依赖一个本地索引(约 17 MB)。v0.3.0 之后**全新下载**的数据在安装时就已带上它,
首次 `fojin cite` 不会有额外提示或等待;**升级前已经下载过的数据**则在首次运行 `fojin cite`
时补建一次(约 1–2 秒,数据目录相应增加约 17 MB),之后按经号查询为毫秒级。
数据目录不可写时会跳过建索引,查询结果不受影响,只是较慢。

```
$ fojin texts "心经" | head -3
T0249  佛說帝釋般若波羅蜜多心經  (藏 50 · 梵 25)
T0251  般若波羅蜜多心經  (藏 47 · 梵 53)
T0252  普遍智藏般若波羅蜜多心經  (藏 21 · 梵 48)
```

## For AI Agents / LLM 工具调用

fojin-cli 是为 agent 设计友好的离线检索原语:**毫秒级、确定性输出、零网络、纯 JSON stdout**。
需要核对"这段汉文有没有已知梵藏对齐"时,让 agent 调它,比在线 API 快两个数量级且不占配额:

```bash
fojin parallel "<汉文短语>" --json --offline
```

- 退出码可编程分支:`0` 成功(看 JSON `matched`)、`1` 运行期错误、`2` 用法错误;进度/提示全在 stderr。
- 现成集成包见 [`examples/claude/`](examples/claude/):Claude Code 斜杠命令 + CLAUDE.md 片段,
  其他框架(function calling 等)可照搬其中的调用约定。
- 边界:无语义搜索、无巴利、无翻译——这三样请接 [Dharmamitra](https://dharmamitra.org) 在线 API,与本工具互补。
- `--json` 输出含 `schema_version`(当前为 `1`);切分发生时额外带 `segments[]`,整串回退时额外带 `fallback{}`,`matched`/`total`/`shown`/`groups` 四个字段语义不变。

更多集成样例(jq 管道、批量查询、Python 调用)见 [`examples/`](examples/)。

## 输入规则与匹配方式

- 查询须至少 **2 个汉字**;单字查询会被拒绝(范围过大,无对读价值)。
- **简繁通用、标点无关**:查询前自动做简繁归一并剥离标点——简体「应无所住」可直接命中繁体原文「應無所住而生其心」。
- 匹配为**整串子串匹配**(FTS5 trigram):查询串须连续完整出现在某条经文分段中。4~12 字的短语/名句命中最佳。
- **整串查不到时会自动按句切分重查**(加 `--no-split` 关闭;最多处理 20 句,超出部分会在输出中明确告知),并对仍无命中的分句给出该句中最长的可命中子串(子串至少 3 字——更短的子串在 90 万行语料里几乎必然命中,提示没有信息量;查询归一化后超过 60 字则不回退;子串为归一化形式——简体、已去标点,可能与原字形不同)。
- 切句只按**句级**标点(`，。；：！？` 及对应半角、换行);顿号 `、` 不算——佛典列举(色聲香味觸法)本身就是一条对齐分段,在那里断会把它切碎。
- 输入端不再限于汉文:`--from sa` / `--from bo` 可用梵文转写或藏文反查汉文对应(完整 Unicode 大小写折叠;变音符号仍需与原文一致,不做折叠),反查不做切分与回退。

## 退出码

| code | 含义 |
| --- | --- |
| `0` | 成功(包括「未找到对齐」) |
| `1` | 运行期错误(数据缺失、下载校验失败、单字查询等) |
| `2` | 用法错误(非法参数、无输入) |

进度与提示信息全部走 stderr;`--json` 时 stdout 保证为纯 JSON,可直接接管道。

## 数据集

- **908,620** 条跨正典平行,锚定到汉文大藏经(Taishō 编号 + 经名):
  - 藏 / Tibetan:676,898 条
  - 梵 / Sanskrit:231,722 条
- 来源:Dharmamitra 的 [MITRA-parallel](https://github.com/dharmamitra/mitra-parallel) 对齐数据集([Nehrdich & Keutzer, 2026](https://arxiv.org/pdf/2601.06400)),以 GitHub Release(`data-v1`)形式分发;学术使用请引用原论文(BibTeX 见 [`DATA_LICENSE`](DATA_LICENSE))。
- 首次运行时下载,压缩包约 **183 MB**,解压并建好按经号索引后约 **578 MB**(SQLite)。下载后完全离线可用。
- 当前不含巴利对齐(上游 MITRA-parallel 尚未覆盖巴利),默认输出不显示巴利行;显式 `--lang pi` 仍可查询(如实答「未找到对齐」)。
- 磁盘与传输限额、超时、并发串行化、`data-v1` 版本固定策略、置信度字段与中断恢复,见
  [`docs/data-operations.md`](docs/data-operations.md)。
- 许可:**CC BY-SA 4.0**(<https://creativecommons.org/licenses/by-sa/4.0/>)。**全部对齐内容归 Dharmamitra**;
  fojin 做的是加工层——Taishō 编号/经名/卷号的关联、简繁归一化列、SQLite+FTS 打包,并按 ShareAlike 以同一许可分发。
  改动清单见 [`DATA_LICENSE`](DATA_LICENSE)。
- 范围:仅含 MITRA 跨藏平行;fojin 自有的精选对齐(alignment_pairs)**未包含**在本数据集中。
- 未来可能提供体积更小的 lite 子集,供带宽/存储受限场景使用(尚未实现)。

## 许可

- **代码**:MIT OR Apache-2.0,见 [`LICENSE-MIT`](LICENSE-MIT) / [`LICENSE-APACHE`](LICENSE-APACHE)。
- **数据**:CC BY-SA 4.0(<https://creativecommons.org/licenses/by-sa/4.0/>),对齐来自 Dharmamitra 的 MITRA-parallel、
  由 fojin 归一化并打包,见 [`DATA_LICENSE`](DATA_LICENSE)。

代码与数据的许可证是分开的 —— 使用/分发本项目产出的数据集时,请遵循 `DATA_LICENSE`(署名 + 相同方式共享),与代码许可无关。

## 生态 / Ecosystem

`fojin-cli` 是 [fojin](https://fojin.app) 开放工具集的一部分 —— fojin.app 提供带账号的在线阅读与对读体验,`fojin-cli` 是其离线、无需登录的命令行对应物,共享同一份跨藏佛典对齐数据。

<!-- ecosystem: add masterl-kill link once its repo is known -->

## English Summary

**fojin-cli** is an offline command-line tool: give it a Chinese Buddhist canonical passage, it returns the aligned Sanskrit/Tibetan parallels — from a local SQLite, in ~2 ms on a hit (a zero-hit query that falls through to sentence splitting and substring fallback costs more — up to ~0.33 s measured), fully offline after a one-time 183 MB data download. Single binary, no account, deterministic output.

```bash
cargo install fojin-cli --locked # or: curl -fsSL https://raw.githubusercontent.com/xr843/fojin-cli/master/install.sh | sh
fojin parallel "色即是空"         # Sanskrit + Tibetan parallels with Taishō source refs
fojin texts "心经"                # fuzzy title search → Taishō numbers
fojin cite T0251                  # browse one text's alignments in canonical order
fojin data status                 # local dataset stats
fojin data verify                 # verify version, SQLite, and FTS integrity
```

- **Input**: Chinese by default (traditional/simplified folded, punctuation ignored); literal substring matching over normalized text, 2-to-12-character phrases work best. A whole-string miss auto-splits into sentences and retries (`--no-split` disables this; up to 20 sentences are processed, and the output states explicitly if more were skipped), falling back to the longest matchable substring per sentence — skipped once the normalized text exceeds 60 characters, and the returned substring is itself normalized (simplified, punctuation stripped), so it may not match the original characters. `--from sa`/`--from bo` reverses the query direction (Sanskrit/Tibetan → Chinese, full Unicode case folding — diacritics must still match exactly) without splitting or falling back.
- **Build/install integrity**: prebuilt binaries need no Rust; building from crates.io or source requires Rust 1.95+ (MSRV 1.95). The shell installer verifies the archive against that release's `SHA256SUMS` before extraction and fails closed if the file is missing, ambiguous, or mismatched — see [`docs/install-verification.md`](docs/install-verification.md).
- **For AI agents**: pure-JSON stdout, semantic exit codes (`0` ok / `1` runtime / `2` usage), zero network with `--offline`. Ready-made Claude Code integration in [`examples/claude/`](examples/claude/).
- **Data**: 908,620 zh↔sa/bo alignments from Dharmamitra's [MITRA-parallel](https://github.com/dharmamitra/mitra-parallel) dataset, redistributed under [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/). **All alignments are Dharmamitra's**; fojin's contribution is the processing layer — Taishō/title/fascicle linkage, a simplified-Chinese normalization column, and SQLite+FTS packaging — redistributed under the same license per ShareAlike (change list in [`DATA_LICENSE`](DATA_LICENSE)). Academic use: please cite [Nehrdich & Keutzer (2026)](https://arxiv.org/pdf/2601.06400) — BibTeX in [`DATA_LICENSE`](DATA_LICENSE).
- **Data operations**: transfer caps, timeouts, single-flight concurrency, `data-v1` version pinning, the confidence field, and interrupted-write recovery are documented in [`docs/data-operations.md`](docs/data-operations.md).
- **Not in scope**: semantic search, Pāli, translation — use [Dharmamitra](https://dharmamitra.org)'s online APIs for those; the two are complementary.
- **License**: code MIT OR Apache-2.0; data [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/) (alignments by Dharmamitra, adapted and packaged by fojin).

Part of the [fojin](https://fojin.app) open tool suite — fojin.app is the online reading & parallel-reading platform; fojin-cli is its offline, no-login companion.
