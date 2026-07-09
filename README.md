# fojin-cli — `fojin parallel`

[![CI](https://github.com/xr843/fojin-cli/actions/workflows/ci.yml/badge.svg)](https://github.com/xr843/fojin-cli/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/fojin-cli.svg)](https://crates.io/crates/fojin-cli)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#许可)

**离线 · 无需登录 · 单二进制。** 给一段汉文,查它在梵/巴/藏正典中的平行文本。

```
$ fojin parallel "色即是空"
汉  色即是空  (《心經》T0251 卷1)
梵  rūpaṃ śūnyatā ...      [MITRA 0.91]
藏  gzugs stong pa ...     [MITRA 0.88]
巴  (无对齐)

完整上下文见 https://fojin.app  ·  数据 CC BY-SA(Dharmamitra + fojin)
```

> 这不是 fojin.app 的账号客户端 —— 它不联网(首次下载数据后)、不需要登录。

## 安装

从源码安装(命令为 `fojin`）：

```bash
cargo install --git https://github.com/xr843/fojin-cli
```

> 发布到 crates.io 后即可 `cargo install fojin-cli`；打出 `v*` tag 后也会在 [Releases](https://github.com/xr843/fojin-cli/releases) 提供各平台预编译二进制。

首次运行 `fojin parallel` 会自动下载对齐数据集(见下方「数据集」),之后完全离线。

## 功能 / Usage

```
fojin parallel "色即是空"          # 位置参数
echo "色即是空" | fojin parallel    # 或从 stdin 读取
```

| flag | 说明 | 默认值 |
| --- | --- | --- |
| `--lang sa,bo` | 只看指定语种,逗号分隔(如 `sa,bo,pi`) | 显示 sa/bo/pi |
| `--top N` | 每个语种最多显示 N 条平行 | `3` |
| `--limit N` | 最多显示 N 组匹配 | `10` |
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
  "total": 1,
  "shown": 1,
  "groups": [
    {
      "zh_text": "色即是空",
      "cbeta_id": "T0251",
      "title_zh": "心經",
      "juan_num": 1,
      "parallels": [
        { "lang": "sa", "text": "rūpaṃ śūnyatā ...", "confidence": 0.91 },
        { "lang": "bo", "text": "gzugs stong pa ...", "confidence": 0.88 }
      ]
    }
  ]
}
```

## 数据集

- **908,620** 条跨正典平行,锚定到汉文大藏经(Taishō 编号 + 经名):
  - 藏 / Tibetan:676,898 条
  - 梵 / Sanskrit:231,722 条
- 来源:Dharmamitra 的 MITRA 对齐数据集,以 GitHub Release(`data-v1`)形式分发。
- 首次运行时下载,压缩包约 **184 MB**,解压后约 **561 MB**(SQLite)。下载后完全离线可用。
- 许可:**CC BY-SA 4.0**(Dharmamitra + fojin)。
- 范围:仅含 MITRA 跨藏平行;fojin 自有的精选对齐(alignment_pairs)**未包含**在本数据集中。
- 未来可能提供体积更小的 lite 子集,供带宽/存储受限场景使用(尚未实现)。

## 许可

- **代码**:MIT OR Apache-2.0,见 [`LICENSE-MIT`](LICENSE-MIT) / [`LICENSE-APACHE`](LICENSE-APACHE)。
- **数据**:CC BY-SA 4.0(Dharmamitra + fojin),见 [`DATA_LICENSE`](DATA_LICENSE)。

代码与数据的许可证是分开的 —— 使用/分发本项目产出的数据集时,请遵循 `DATA_LICENSE`(署名 + 相同方式共享),与代码许可无关。

## 生态 / Ecosystem

`fojin-cli` 是 [fojin](https://fojin.app) 开放工具集的一部分 —— fojin.app 提供带账号的在线阅读与对读体验,`fojin-cli` 是其离线、无需登录的命令行对应物,共享同一份跨正典对齐数据。

<!-- ecosystem: add masterl-kill link once its repo is known -->
