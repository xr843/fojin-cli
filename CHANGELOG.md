# Changelog

All notable changes to this project will be documented in this file.

## [0.3.0] - Unreleased

Version 0.3.0 is prepared but has not been published. Its stabilization work includes:

- Data verification: strengthen `fojin data verify` and dataset compatibility checks.
- Data pipeline: move installs and updates to a bounded, disk-streamed, checksum-first pipeline with hard end-to-end HTTP deadlines and rollback-safe Windows replacement backups.
- Data concurrency: serialize concurrent install, update, and clean operations per data directory, with full candidate validation before publication.
- Query correctness: make short-query matching literal and remove duplicate parallel text within a match group and language.
- 检索能力:新增 `--from` 反向查询(梵/藏 → 汉,Unicode 大小写折叠);零命中时自动按句切分重查(`--no-split` 关闭;最多处理 20 句,超出部分会在输出中明确告知);仍零命中时给出最长可命中子串(归一化后超过 60 字则不回退)。`--lang` / `--from` 的未知语种代码现在报用法错误,而非静默返回空结果。
- SQLite safety: upgrade the bundled SQLite and verify its runtime version.
- Release integrity: validate release versions, locked builds, archive contents, checksums, and installer verification.
- Project governance: document private security reporting and contribution checks, and add issue and pull request templates.
- 查询性能:`cite` 现在使用本地建立的 `cbeta_id` 索引,不再全表扫描;索引在安装/更新时建立,已有数据在首次 `cite` 时补建,数据目录不可写时静默退回扫描。
- 输出可读性:置信度显示为 `1.00` 时不再逐行标注 `[MITRA 1.00]`(当前数据集全部如此);`--json` 的 `confidence` 字段不变。

These changes are not released until the 0.3.0 release tag is published.
