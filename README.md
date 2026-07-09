# fojin-cli — `fojin parallel`

**离线 · 无需登录 · 单二进制。** 给一段汉文,查它在梵/巴/藏正典中的平行文本。

```
$ fojin parallel "色即是空"
汉  色即是空  (《心經》T0251 卷1)
梵  rūpaṃ śūnyatā ...      [MITRA 0.91]
藏  gzugs stong pa ...     [MITRA 0.88]
巴  (无对齐)
```

> 这不是 fojin.app 的账号客户端 —— 它不联网(首次下载数据后)、不需要登录。

## 安装
```bash
cargo install fojin-cli   # 命令为 fojin
```

## 用法
```
fojin parallel "色即是空"          # 位置参数
echo "色即是空" | fojin parallel    # 或 stdin
  --lang sa,bo,pi   --top N   --json   --data-dir <path>   --offline
```

## 数据与许可
- 代码:MIT OR Apache-2.0。
- 数据:CC BY-SA 4.0(Dharmamitra + fojin);仅含 MITRA 跨藏平行。见 `DATA_LICENSE`。
