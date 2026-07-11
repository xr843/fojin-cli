# Changelog

All notable changes to this project will be documented in this file.

## [0.3.0] - Unreleased

Version 0.3.0 is prepared but has not been published. Its stabilization work includes:

- Data verification: strengthen `fojin data verify` and dataset compatibility checks.
- Data pipeline: move installs and updates to a bounded, disk-streamed, checksum-first pipeline with hard end-to-end HTTP deadlines and rollback-safe Windows replacement backups.
- Data concurrency: serialize concurrent install, update, and clean operations per data directory, with full candidate validation before publication.
- Query correctness: make short-query matching literal and remove duplicate parallel text within a match group and language.
- SQLite safety: upgrade the bundled SQLite and verify its runtime version.
- Release integrity: validate release versions, locked builds, archive contents, checksums, and installer verification.
- Project governance: document private security reporting and contribution checks, and add issue and pull request templates.

These changes are not released until the 0.3.0 release tag is published.
