# Changelog

All notable changes to this project will be documented in this file.

## [0.3.1] - 2026-08-16

- 发布校验:`check-release-archive.sh` 不再把顶层目录条目当作压缩包的必需成员。GNU tar 会写这个条目,构建 Windows zip 的 PowerShell `Compress-Archive` 不写,于是 v0.3.0 的 release 运行在四个平台全部构建成功、三个 tar.gz 全部通过之后卡在 Windows zip 上,GitHub Release 没能发出。压缩包本身一直是对的。其余约束一条未放宽(成员不得落在顶层目录之外、四个文件必须齐全、Unix 二进制必须可执行、目录条目若存在仍必须是目录),并补上按真实 Windows 产物建模的回归测试——此前的 fixture 由 `shutil.make_archive` 生成,复现的是发布流水线不产出的形状。

**0.3.0 只发到了 crates.io,没有 GitHub 二进制 release**(即上述失败)。需要预编译二进制或使用 `install.sh` 的,请用 0.3.1。

## [0.3.0] - 2026-08-16

本版把 0.2.1 之后的稳定化工作一次发出。它同时修好了一处只有发布才能修的故障:
`install.sh` 要求目标 release 提供 `SHA256SUMS`,而在本版发布前它解析到的最新版本
一直是 v0.2.1 —— 那个 release 早于这项校验合同、没有该文件,于是一行安装命令对所有
人都以 404 失败。本版起 `SHA256SUMS` 随每个 release 发布,该路径恢复可用。

包含的改动:

- Data verification: strengthen `fojin data verify` and dataset compatibility checks.
- Data pipeline: move installs and updates to a bounded, disk-streamed, checksum-first pipeline with hard end-to-end HTTP deadlines and rollback-safe Windows replacement backups.
- Data concurrency: serialize concurrent install, update, and clean operations per data directory, with full candidate validation before publication.
- Query correctness: make short-query matching literal and remove duplicate parallel text within a match group and language.
- 检索能力:新增 `--from` 反向查询(梵/藏 → 汉,Unicode 大小写折叠);零命中时自动按句切分重查(`--no-split` 关闭;最多处理 20 句,超出部分会在输出中明确告知);仍零命中时给出最长可命中子串(归一化后超过 60 字则不回退)。`--lang` / `--from` 的未知语种代码现在报用法错误,而非静默返回空结果。
- SQLite safety: upgrade the bundled SQLite and verify its runtime version.
- Release integrity: validate release versions, locked builds, archive contents, checksums, and installer verification.
- Project governance: document private security reporting and contribution checks, and add issue and pull request templates.
- 查询性能:`cite` 现在使用本地建立的 `cbeta_id` 索引,不再全表扫描;索引在安装/更新时建立,已有数据在首次 `cite` 时补建,数据目录不可写时静默退回扫描。
- 中断恢复:写入数据时被中断(Ctrl-C、断电等)留下的回滚日志现在会在查询打开数据时由 SQLite 自动回滚;数据确实不可写而无法回滚时,报错会说明真实原因与处理办法,不再谎称数据不兼容。`fojin data clean` 与 `fojin data update` 也会一并清理活跃数据的 `-journal`/`-shm`/`-wal` 边车,新下载的数据不会再被上一份数据的陈旧日志污染。
- 输出可读性:置信度显示为 `1.00` 时不再逐行标注 `[MITRA 1.00]`(当前数据集全部如此);`--json` 的 `confidence` 字段不变。
- 文档:README 收敛到「怎么装、怎么查、数据从哪来」。安装校验合同与各平台核对命令移入 `docs/install-verification.md`,传输限额/超时/并发/版本固定/中断恢复移入 `docs/data-operations.md`。
- 数据署名:修正 CC BY-SA 4.0 署名。此前的 `Dharmamitra + fojin` 把两家并列成对齐内容的共同来源,而全部对齐均来自 Dharmamitra 的 MITRA-parallel,fojin 只做加工层(Taishō 编号/经名/卷号关联、简繁归一化列、SQLite+FTS 打包)。人类可读输出的脚注、`README`、`DATA_LICENSE` 现在分层署名,并补上此前全仓库缺失的许可文本链接与「已作改动」声明。`--json` 输出与数据文件本身不变(数据 meta 表的署名在下一个数据版本生效)。
