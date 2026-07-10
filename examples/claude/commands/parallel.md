---
description: 离线查询汉文佛典的梵/藏平行文本(fojin-cli,毫秒级,不联网,结果确定可复现)
argument-hint: <汉文短语,如 色即是空>
---

用户要查的汉文: $ARGUMENTS

用本地 fojin-cli 查询(不要调在线 API):

```bash
fojin parallel "$ARGUMENTS" --json --offline
```

结果处理规则:

1. `matched: true` 时,从 `groups[]` 中挑与用户意图最相关的组,呈现:汉文原文(`zh_text`)、出处(`title_zh` + `cbeta_id` + `juan_num`)、各语种平行(`parallels[].lang`/`text`,`sa`=梵 `bo`=藏)及置信度(`confidence`,0–1)。
2. `matched: false` 时,先把查询缩短成 2~6 字的核心短语重试一次(fojin 是整串子串匹配,短语命中率更高);仍无结果再告知用户该句在 MITRA 数据集中无对齐。
3. 退出码 `1` 且 stderr 提示"至少需要 2 个汉字"时,请用户给出更长的短语。
4. 本地数据不存在时(exit 1,提示含 `--offline`),去掉 `--offline` 重跑一次以触发自动下载(约 183 MB,只需一次),或改用 `fojin data update`。

何时不该用本命令:需要语义相似(而非字面包含)检索、需要巴利文、或需要整部经翻译时——那些场景应使用 Dharmamitra 在线 API。
