# examples — 集成样例

| 文件 | 用途 |
| --- | --- |
| [`claude/`](claude/) | **AI agent 集成包**:Claude Code 斜杠命令 + CLAUDE.md 片段,让 agent 把 fojin-cli 当离线工具调用 |
| [`jq-extract.sh`](jq-extract.sh) | jq 管道:提取一段汉文的全部梵文平行 |
| [`batch-query.sh`](batch-query.sh) | 批量查询短语列表 → TSV 报表 |
| [`python_call.py`](python_call.py) | Python 子进程调用,零第三方依赖 |

所有样例假定 `fojin` 在 PATH 且数据已就绪(`fojin data status` 可确认);
样例统一用 `--offline`,保证确定性、不产生网络请求。
