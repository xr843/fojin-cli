# fojin-cli

[![CI](https://github.com/xr843/fojin-cli/actions/workflows/ci.yml/badge.svg)](https://github.com/xr843/fojin-cli/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/fojin-cli.svg)](https://crates.io/crates/fojin-cli)
[![release](https://img.shields.io/github/v/release/xr843/fojin-cli?filter=v*&label=release)](https://github.com/xr843/fojin-cli/releases/latest)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#许可)

**离线 · 无需登录 · 单二进制。** 给一段汉文,查它在梵/藏正典中的平行文本。本地查询毫秒级(实测典型 2 ms,数千组命中的高频词约 0.3 s)。

*English readers: see the [English summary](#english-summary) at the bottom.*

```
$ fojin parallel "色即是空"
汉  色不異空，空不異色，色即是空，空即是色；  (《般若波羅蜜多心經》T0251 卷1)
梵  śūnyat'aiva rūpaṃ, rūpān na pṛthak śūnyatā …  [MITRA 1.00]
藏  གཟུགས་ལས་སྟོང་པ་ཉིད་གཞན་མ་ཡིན༏ …  [MITRA 1.00]

… 还有 38 组匹配,加 --all 查看全部

完整上下文见 https://fojin.app  ·  数据 CC BY-SA(Dharmamitra + fojin)
```

> 这不是 fojin.app 的账号客户端 —— 它不联网(首次下载数据后)、不需要登录。

## 安装

从 crates.io 或源码构建要求 **Rust 1.95 或更新版本**（MSRV 1.95）；使用预编译二进制不需要安装 Rust。

通过 [crates.io](https://crates.io/crates/fojin-cli) 安装(命令为 `fojin`）：

```bash
cargo install fojin-cli --locked
```

没有 Rust 环境?一行脚本自动安装对应平台的预编译二进制(Linux x64 / macOS ARM+Intel)：

```bash
curl -fsSL https://raw.githubusercontent.com/xr843/fojin-cli/master/install.sh | sh
```

这项校验合同从 **v0.3.0** 起适用：安装脚本要求它解析出的最新版本或 `FOJIN_VERSION` 指定的目标
**二进制 release** 同时提供 `SHA256SUMS`，并在解压和安装前用 `sha256sum` 或 `shasum -a 256`
核对 archive；缺少校验工具、校验记录不唯一或摘要不匹配时都会停止安装。

如果目标二进制 release 早于 v0.3.0（包括 v0.3.0 尚未发布的过渡窗口中，脚本自动解析到旧版），
旧 release 没有 `SHA256SUMS` 时脚本会在解压前安全失败。此时请改用 crates.io 当前已发布版本，
或从源码构建；这段说明不表示 v0.3.0 已经发布。

也可从 [Releases](https://github.com/xr843/fojin-cli/releases/latest) 手动下载各平台二进制(含 Windows x64 zip),或从源码安装：

```bash
cargo install --git https://github.com/xr843/fojin-cli --locked
```

手动下载时请一并下载同一 release 的 `SHA256SUMS`，并在解压前核对所下载 archive 的 SHA-256。
例如 GNU `sha256sum` 可从清单中筛选对应文件后校验（将占位符替换为 release 中的实际名称）：

```bash
archive="fojin-<VERSION>-<TARGET>.tar.gz"
grep "  ${archive}$" SHA256SUMS | sha256sum -c -
```

macOS 可将最后一段换为 `shasum -a 256 -c -`；Windows 可用 `Get-FileHash -Algorithm SHA256`
并与 `SHA256SUMS` 中对应的唯一记录比较。

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
  "matched": true,
  "total": 10,
  "shown": 1,
  "groups": [
    {
      "zh_text": "是故菩薩應生如是無住著心，不住色、聲、香、味、觸、法生心，應無所住而生其心。",
      "cbeta_id": "T0237",
      "title_zh": "金剛般若波羅蜜經",
      "juan_num": 1,
      "parallels": [
        { "lang": "sa", "text": "tasmāt tarhi subhūte bodhisatvena evaṃ cittam utpādayitavyaṃ apratiṣṭhitaṃ …", "confidence": 1.0 },
        { "lang": "bo", "text": "བྱང་ཆུབ་སེམས་དཔའ་སེམས་དཔའ་ཆེན་པོས་འདི་ལྟར་མི་གནས་པར་སེམས་བསྐྱེད་པར་བྱའོ་༎ …", "confidence": 1.0 }
      ]
    }
  ]
}
```

(示例取自真实查询 `fojin parallel "应无所住" --json --top 1 --limit 1`,文本有截断;字段实际按字母序输出。)

## 其他子命令

```bash
fojin texts "心经"        # 模糊查经名(简繁均可) → Taishō 编号 + 各语对齐条数
fojin cite T0251          # 按编号列出一部经的对齐,经文顺序;--juan N 限定卷
fojin data status         # 本地数据状态(位置/大小/版本/行数统计)
fojin data clean          # 删除本地数据,释放 561 MB
fojin data update         # 重新下载数据(覆盖本地)
fojin data verify         # 校验版本、SQLite 与 FTS 完整性
```

`texts` 与 `cite` 支持与 `parallel` 一致的 `--json` / `--data-dir` / `--offline`;
`cite` 另有 `--lang` / `--top` / `--limit` / `--all`。典型工作流:`texts` 找到编号 → `cite` 通读对齐。

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

更多集成样例(jq 管道、批量查询、Python 调用)见 [`examples/`](examples/)。

## 输入规则与匹配方式

- 查询须至少 **2 个汉字**;单字查询会被拒绝(范围过大,无对读价值)。
- **简繁通用、标点无关**:查询前自动做简繁归一并剥离标点——简体「应无所住」可直接命中繁体原文「應無所住而生其心」。
- 匹配为**整串子串匹配**(FTS5 trigram):查询串须连续完整出现在某条经文分段中。4~12 字的短语/名句命中最佳;整段文字超出分段长度,基本查不到——请拆成短句分别查。
- 输入端仅支持汉文(查询方向:汉 → 梵/藏);用梵文转写或藏文查询不会报错,但必然「未找到对齐」。

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
- 当前二进制把官方下载地址、SHA-256 与兼容元数据固定在 `data-v1`;`fojin data update` 只会重新获取这份固定数据,不会自动切换到未来的数据主版本。版本、归一化规则或查询所需 schema 不兼容的数据会被拒绝。
- 首次运行时下载,压缩包约 **183 MB**,解压后约 **561 MB**(SQLite)。下载后完全离线可用。
- 当前不含巴利对齐(上游 MITRA-parallel 尚未覆盖巴利),默认输出不显示巴利行;显式 `--lang pi` 仍可查询(如实答「未找到对齐」)。程序的渲染路径可兼容未来新增语言行,但当前官方下载通道仍固定为 `data-v1`;上游出现新语言不代表当前二进制会自动获得它。**渲染兼容不等于官方更新通道无需升级**,未来数据版本可能要求升级二进制或明确切换数据发布。
- 许可:**CC BY-SA 4.0**(Dharmamitra + fojin)。
- 范围:仅含 MITRA 跨藏平行;fojin 自有的精选对齐(alignment_pairs)**未包含**在本数据集中。
- 未来可能提供体积更小的 lite 子集,供带宽/存储受限场景使用(尚未实现)。

## 许可

- **代码**:MIT OR Apache-2.0,见 [`LICENSE-MIT`](LICENSE-MIT) / [`LICENSE-APACHE`](LICENSE-APACHE)。
- **数据**:CC BY-SA 4.0(Dharmamitra + fojin),见 [`DATA_LICENSE`](DATA_LICENSE)。

代码与数据的许可证是分开的 —— 使用/分发本项目产出的数据集时,请遵循 `DATA_LICENSE`(署名 + 相同方式共享),与代码许可无关。

## 生态 / Ecosystem

`fojin-cli` 是 [fojin](https://fojin.app) 开放工具集的一部分 —— fojin.app 提供带账号的在线阅读与对读体验,`fojin-cli` 是其离线、无需登录的命令行对应物,共享同一份跨藏佛典对齐数据。

<!-- ecosystem: add masterl-kill link once its repo is known -->

## English Summary

**fojin-cli** is an offline command-line tool: give it a Chinese Buddhist canonical passage, it returns the aligned Sanskrit/Tibetan parallels — from a local SQLite, in ~2 ms, fully offline after a one-time 183 MB data download. Single binary, no account, deterministic output.

```bash
cargo install fojin-cli --locked # or: curl -fsSL https://raw.githubusercontent.com/xr843/fojin-cli/master/install.sh | sh
fojin parallel "色即是空"         # Sanskrit + Tibetan parallels with Taishō source refs
fojin texts "心经"                # fuzzy title search → Taishō numbers
fojin cite T0251                  # browse one text's alignments in canonical order
fojin data status                 # local dataset stats
fojin data verify                 # verify version, SQLite, and FTS integrity
```

- **Input**: Chinese only (traditional/simplified folded, punctuation ignored); literal substring matching over normalized text. 2-to-12-character phrases work best.
- **Build/install integrity**: building from crates.io or source requires Rust 1.95+ (MSRV 1.95). Starting with v0.3.0, the shell installer requires the target binary release to provide `SHA256SUMS` and verifies the archive before extraction. It fails closed for an older latest or explicitly selected release without that file, including the transition before v0.3.0 is published; use the currently published crates.io version or a source build instead. This does not state that v0.3.0 has been released.
- **For AI agents**: pure-JSON stdout, semantic exit codes (`0` ok / `1` runtime / `2` usage), zero network with `--offline`. Ready-made Claude Code integration in [`examples/claude/`](examples/claude/).
- **Data**: 908,620 zh↔sa/bo alignments from Dharmamitra's [MITRA-parallel](https://github.com/dharmamitra/mitra-parallel) dataset, redistributed under CC BY-SA 4.0. The official URL, checksum, and compatibility contract remain pinned to `data-v1`; rendering support for future language rows does not mean the official update channel can adopt them without a binary upgrade. Academic use: please cite [Nehrdich & Keutzer (2026)](https://arxiv.org/pdf/2601.06400) — BibTeX in [`DATA_LICENSE`](DATA_LICENSE).
- **Not in scope**: semantic search, Pāli, translation — use [Dharmamitra](https://dharmamitra.org)'s online APIs for those; the two are complementary.
- **License**: code MIT OR Apache-2.0; data CC BY-SA 4.0.

Part of the [fojin](https://fojin.app) open tool suite — fojin.app is the online reading & parallel-reading platform; fojin-cli is its offline, no-login companion.
