## Summary

Describe the problem and the focused change that solves it. Note user-visible behavior and compatibility impact.

## Verification

List the exact commands you ran and their results, for example:

```text
cargo +stable test --all --locked
```

## Checklist

- [ ] The change is focused and contains no credentials, generated datasets, build artifacts, or public vulnerability details.
- [ ] I added or updated tests for behavior changes, or explained why tests are not applicable.
- [ ] `cargo +stable fmt --all --check` passes.
- [ ] `cargo +stable clippy --all-targets --locked -- -D warnings` passes.
- [ ] `cargo +stable test --all --locked` passes.
- [ ] `cargo +1.95.0 test --all --locked` passes when the change affects Rust code or dependencies.
- [ ] Relevant Python and shell checks from `CONTRIBUTING.md` pass.
- [ ] Documentation and the unreleased changelog entry are updated when user-visible behavior changes.
- [ ] Dependency changes include the intended `Cargo.lock` update and retain the MSRV.
- [ ] Data changes comply with `DATA_LICENSE`; code changes are acceptable under MIT OR Apache-2.0.
