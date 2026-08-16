# 安装完整性校验

`install.sh` 与手动下载都以 SHA-256 为准。这里是完整规则；日常安装看
[README 的「安装」节](../README.md#安装) 就够了。

## install.sh 的校验合同

脚本在**解压之前**核对 archive，任何一步不满足都停止安装，不会落地二进制：

- 目标 release 必须提供 `SHA256SUMS`；
- 本机必须有 `sha256sum` 或 `shasum -a 256`；
- `SHA256SUMS` 里对应 archive 的记录必须**唯一且格式正确**（64 位十六进制 + 文件名）；
- 实际摘要必须与记录一致。

这项合同从 **v0.3.0** 起适用。更早的 release 没有 `SHA256SUMS`，脚本会在解压前
安全失败（fail closed）——这是刻意的：宁可装不上，也不装一个没核对过的二进制。
如果你用 `FOJIN_VERSION` 显式指定了 v0.3.0 之前的版本而遇到这个失败，请改用
crates.io 上的当前版本或从源码构建。

环境变量：

| 变量 | 作用 | 默认值 |
| --- | --- | --- |
| `FOJIN_INSTALL_DIR` | 安装目录 | `~/.local/bin` |
| `FOJIN_VERSION` | 指定要装的 tag，如 `v0.3.0` | 最新的 `v*` release |

（仓库同时发布 `data-v*` 数据 release，所以脚本按 `v[0-9]*` 解析版本，而不是取
`/releases/latest`。）

## 手动下载时自己核对

从 [Releases](https://github.com/xr843/fojin-cli/releases/latest) 下载时，请一并下载
**同一个 release** 的 `SHA256SUMS`，在解压前核对。

GNU `sha256sum`：

```bash
archive="fojin-<VERSION>-<TARGET>.tar.gz"
grep "  ${archive}$" SHA256SUMS | sha256sum -c -
```

macOS 把最后一段换成 `shasum -a 256 -c -`。

Windows PowerShell：

```powershell
Get-FileHash -Algorithm SHA256 fojin-<VERSION>-x86_64-pc-windows-msvc.zip
```

再与 `SHA256SUMS` 中对应的那条唯一记录比对。

## 数据集的校验

二进制里钉死了 `data-v1` 的下载地址与 SHA-256（`DATA_SHA256`，在 `src/cli.rs`）。
数据下载走的是同一套先校验后落地的流程，细节见
[`data-operations.md`](data-operations.md)。
