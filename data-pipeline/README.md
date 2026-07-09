# fojin-cli data pipeline (maintainer only)

Builds the public SQLite artifact from fojin's **private** Postgres.

## What it emits
- `mitra_alignments` only (CC BY-SA, from Dharmamitra). **No** fojin-internal columns.
- fojin's own `alignment_pairs` are **never** touched.

## Run
```bash
pip install -r requirements.txt
DATABASE_URL=postgres://... OUT=fojin-parallels-v1.sqlite python export_parallels.py
```
Outputs `*.sqlite`, `*.sqlite.gz`, `*.sqlite.gz.sha256`.

## Publish
Upload `fojin-parallels-vN.sqlite.gz` + `.sha256` to the GitHub Release `data-vN`,
then set `DATA_SHA256` in `src/cli.rs` to the printed digest.

## Parity
`normalize()` here MUST match Rust `src/normalize.rs`. Guarded by
`tests/test_normalize_parity.py` against the shared golden fixtures.
