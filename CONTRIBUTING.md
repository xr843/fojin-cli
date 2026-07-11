# 贡献指南 / Contributing

感谢你改进 fojin-cli。请先搜索已有 issue 和 pull request；行为变更或范围较大的工作，建议先用普通
issue 说明问题与预期结果。安全问题不要公开提交，请遵循 [SECURITY.md](SECURITY.md)。

## 开发环境

- Rust 1.95.0 是最低支持版本（MSRV）；日常格式、Clippy 与测试也应使用当前 stable。
- Python parity job 使用 Python 3.12 与 pytest。
- release contract 使用 Bash 与 ShellCheck。

安装所需 Rust 工具链与组件：

```bash
rustup toolchain install 1.95.0
rustup toolchain install stable --component rustfmt --component clippy
```

## 提交前检查

以下命令与 `.github/workflows/ci.yml` 中的 Linux CI 对应，请从仓库根目录运行。

Stable Rust 检查：

```bash
cargo +stable fmt --all --check
cargo +stable clippy --all-targets --locked -- -D warnings
cargo +stable test --all --locked
```

MSRV 检查：

```bash
cargo +1.95.0 test --all --locked
cargo +1.95.0 install --path . --locked
```

Python parity 检查：

```bash
python3.12 -m pip install pytest
cd data-pipeline
python3.12 -m pytest tests/ -q
cd ..
```

Release/installer shell contract：

```bash
shellcheck install.sh scripts/*.sh tests/*.sh
bash tests/release-scripts.sh
bash tests/install-script.sh
```

跨平台 release build 也在 CI 中覆盖 Linux x86_64、macOS ARM/Intel 与 Windows x64，执行的构建命令为：

```bash
cargo +stable build --release --locked --target <TARGET>
```

`<TARGET>` 由 CI matrix 提供；本地无需模拟不属于当前主机的 target。

所有 Cargo 命令保留 `--locked`，避免无意更新依赖解析。若确需更新依赖，请同时提交并说明
`Cargo.lock` 的变更。

## Pull request

- 保持改动聚焦，并说明用户可见行为与兼容性影响。
- 为行为修复或新增功能补充测试；文档应与实际命令和输出一致。
- 不要提交生成的数据集、构建产物、凭据或安全漏洞细节。
- 需要 changelog 的用户可见改动，请更新 `CHANGELOG.md` 的未发布版本条目。
- 确认适用的 CI 命令通过，并在 PR 模板中列出实际运行的命令。

提交贡献即表示你同意代码按本仓库的 MIT OR Apache-2.0 双许可证提供；数据文件仍受
`DATA_LICENSE` 中的独立条款约束。
