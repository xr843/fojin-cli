# Task 1 Report: Reject One-Character Normalized Queries Before Data Access

## Status

Completed on branch `agent/query-contract`.

## What changed

- Added `normalize::MIN_QUERY_CHARS: usize = 2`.
- Added `normalize::validate_query_length(normalized: &str) -> anyhow::Result<()>`.
- Added a preflight normalization+validation step in `run()` before `resolve_data_path`.
- Added validation of the fully normalized query in `compute_output()` before search.
- Added normalization tests for one-character rejection and two-character acceptance.
- Added a command-level regression test proving one-character offline queries fail before missing-data handling.

## RED/GREEN log

### RED 1: normalization API missing

Command:

```bash
cargo test --test normalize query
```

Output:

```text
   Compiling fojin-cli v0.1.0 (/home/lqsxi/projects/fojin-cli)
error[E0425]: cannot find function `validate_query_length` in module `fojin_cli::normalize`
  --> tests/normalize.rs:44:37
   |
44 |     let err = fojin_cli::normalize::validate_query_length("佛").unwrap_err();
   |                                     ^^^^^^^^^^^^^^^^^^^^^ not found in `fojin_cli::normalize`

error[E0425]: cannot find function `validate_query_length` in module `fojin_cli::normalize`
  --> tests/normalize.rs:50:35
   |
50 |     assert!(fojin_cli::normalize::validate_query_length("般若").is_ok());
   |                                   ^^^^^^^^^^^^^^^^^^^^^ not found in `fojin_cli::normalize`

For more information about this error, try `rustc --explain E0425`.
error: could not compile `fojin-cli` (test "normalize") due to 2 previous errors
```

### RED 2: command path still reaches missing-data handling first

Command:

```bash
cargo test --test command one_character_query_fails_before_missing_offline_data
```

Output:

```text
   Compiling fojin-cli v0.1.0 (/home/lqsxi/projects/fojin-cli)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 1.58s
     Running tests/command.rs (target/debug/deps/command-6b0e527df957727a)

running 1 test
test one_character_query_fails_before_missing_offline_data ... FAILED

failures:

---- one_character_query_fails_before_missing_offline_data stdout ----

thread 'one_character_query_fails_before_missing_offline_data' panicked at tests/command.rs:12:5:
assertion failed: stderr.contains("至少需要 2 个汉字")
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    one_character_query_fails_before_missing_offline_data

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

error: test failed, to rerun pass `--test command`
```

### GREEN 1: normalization tests

Command:

```bash
cargo test --test normalize
```

Output:

```text
   Compiling fojin-cli v0.1.0 (/home/lqsxi/projects/fojin-cli)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 1.18s
     Running tests/normalize.rs (target/debug/deps/normalize-de1b32c64c2d0fb5)

running 4 tests
test golden_norm_cases ... ok
test rejects_single_character_query ... ok
test accepts_two_character_query ... ok
test load_norm_map_reads_rows ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

### GREEN 2: focused task tests

Command:

```bash
cargo test --test normalize --test command --test cli
```

Output:

```text
   Compiling fojin-cli v0.1.0 (/home/lqsxi/projects/fojin-cli)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.83s
     Running tests/cli.rs (target/debug/deps/cli-4dc58a923bf258ef)

running 4 tests
test compute_json_output_matches ... ok
test compute_limit_caps_groups_and_reports_hidden ... ok
test compute_human_output_matches ... ok
test compute_applies_normalization ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/command.rs (target/debug/deps/command-6b0e527df957727a)

running 1 test
test one_character_query_fails_before_missing_offline_data ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/normalize.rs (target/debug/deps/normalize-de1b32c64c2d0fb5)

running 4 tests
test accepts_two_character_query ... ok
test rejects_single_character_query ... ok
test golden_norm_cases ... ok
test load_norm_map_reads_rows ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

### Full Rust suite once

Command:

```bash
cargo test
```

Output:

```text
   Compiling fojin-cli v0.1.0 (/home/lqsxi/projects/fojin-cli)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 1.09s
     Running unittests src/lib.rs (target/debug/deps/fojin_cli-f40268b0d814ab49)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running unittests src/main.rs (target/debug/deps/fojin-3b300b549b2ba905)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/cli.rs (target/debug/deps/cli-4dc58a923bf258ef)

running 4 tests
test compute_human_output_matches ... ok
test compute_applies_normalization ... ok
test compute_json_output_matches ... ok
test compute_limit_caps_groups_and_reports_hidden ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/command.rs (target/debug/deps/command-6b0e527df957727a)

running 1 test
test one_character_query_fails_before_missing_offline_data ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/data.rs (target/debug/deps/data-54f55efc26ea7a0e)

running 6 tests
test resolve_honours_data_dir_override ... ok
test sha256_matches_known_vector ... ok
test offline_and_missing_errors_clearly ... ok
test gunzip_roundtrips ... ok
test present_file_is_a_noop ... ok
test write_atomic_writes_content_and_leaves_no_temp ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/normalize.rs (target/debug/deps/normalize-de1b32c64c2d0fb5)

running 4 tests
test accepts_two_character_query ... ok
test rejects_single_character_query ... ok
test golden_norm_cases ... ok
test load_norm_map_reads_rows ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/query.rs (target/debug/deps/query-fabdafd5758f1173)

running 7 tests
test empty_query_returns_empty ... ok
test exact_match_groups_two_langs ... ok
test short_query_uses_like_fallback ... ok
test no_match_is_empty_not_error ... ok
test top_zero_floors_to_one ... ok
test per_lang_cap_and_order ... ok
test lang_filter_keeps_only_requested ... ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

     Running tests/render.rs (target/debug/deps/render-45e68d3b0eded06c)

running 10 tests
test human_empty_is_honest ... ok
test human_lang_filter_hides_unrequested_and_no_false_wuduiqi ... ok
test human_no_hidden_hint_when_not_truncated ... ok
test human_multi_group_footer_once ... ok
test human_shows_extra_lang_and_full_footer ... ok
test human_shows_hidden_count_hint_when_truncated ... ok
test human_shows_parallels_wuduiqi_and_footer ... ok
test json_exposes_only_public_fields ... ok
test json_flags_matched ... ok
test json_includes_total_and_shown ... ok

test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/schema.rs (target/debug/deps/schema-ca5a8ac2915108a1)

running 1 test
test schema_creates_tables_and_fts_autopopulates ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/smoke.rs (target/debug/deps/smoke-508d2512345588d8)

running 1 test
test version_is_nonempty ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests fojin_cli

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

## Additional direct verification

Empty raw-input behavior remains unchanged:

```bash
printf '' | cargo run --quiet -- parallel
```

Output:

```text
用法: fojin parallel "色即是空"  (或管道: echo ... | fojin parallel)

EXIT:2
```

Two-character raw queries remain allowed and still proceed to downstream data checks:

```bash
tmpdir=$(mktemp -d)
cargo run --quiet -- parallel 般若 --offline --data-dir "$tmpdir"
status=$?
rm -rf "$tmpdir"
printf 'EXIT:%s\n' "$status"
```

Output:

```text
错误: 本地数据不存在且处于 --offline (offline)。请手动下载:
  https://github.com/xr843/fojin-cli/releases/download/data-v1/fojin-parallels-v1.sqlite.gz
解压后放到: /tmp/tmp.fWhnaC70vM/data.sqlite
EXIT:1
```

## Files changed

- `src/normalize.rs`
- `src/cli.rs`
- `tests/normalize.rs`
- `tests/command.rs`
- `.superpowers/sdd/task-1-report.md`

## Self-review

- The new validator lives in `normalize.rs`, which keeps the length contract data-independent and reusable.
- `run()` still checks `raw.trim().is_empty()` before preflight normalization, so empty stdin behavior is preserved.
- The preflight path uses `normalize::NormMap::new()` exactly as required, so one-character queries fail before any data-path resolution or offline checks.
- `compute_output()` also validates the fully normalized query after loading the real normalization map, which covers cases where a longer raw query could normalize down below the minimum.
- Scope stayed inside the allowed files. No unrelated edits were made.

## Concerns

None.
