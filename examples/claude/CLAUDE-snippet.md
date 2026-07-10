# fojin-cli — 本地佛典平行文本查询(粘贴进你项目的 CLAUDE.md)

本机装有 `fojin`(fojin-cli):离线查询汉文佛典在梵/藏正典中的对齐平行文本。
毫秒级、确定性输出、不联网 —— 需要核对"这段汉文有没有已知梵藏对齐"时**优先用它**,
而不是调在线 API。

## 调用约定

```bash
fojin parallel "<汉文短语>" --json --offline      # 平行查询(2~12 字短语最佳)
fojin texts "<经名关键词>" --json --offline        # 模糊查经名 → Taishō 编号
fojin cite <Taishō编号> --json --offline           # 按编号浏览整部经的对齐
fojin data status --json                           # 数据是否就绪
```

- 简繁均可,标点自动剥离;查询是**整串子串匹配**,长段落请拆短句。
- 退出码:`0` 成功(含无结果,看 JSON 的 `matched`);`1` 运行期错误(读 stderr);`2` 用法错误。
- stdout 保证纯 JSON;进度/提示都在 stderr。
- JSON 字段:`groups[].zh_text`(汉文)、`cbeta_id`/`title_zh`/`juan_num`(出处)、
  `parallels[].lang`(`sa` 梵 / `bo` 藏)、`text`、`confidence`(MITRA 置信度 0–1)。
- 数据范围:MITRA 对齐 908,620 条(藏+梵),**无巴利、无语义搜索、无翻译**——
  这三样请用 Dharmamitra 在线 API。
- 首次使用若数据未下载:去掉 `--offline` 跑一次(自动下载 ~183 MB)或 `fojin data update`。
