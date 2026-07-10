# 在 Claude Code 里使用 fojin-cli

两个文件,按需取用:

1. **`commands/parallel.md`** — 拷进你项目的 `.claude/commands/`,即可在 Claude Code 里用
   `/parallel 色即是空` 触发离线平行查询。
2. **`CLAUDE-snippet.md`** — 内容粘贴进你项目的 `CLAUDE.md`,让 Claude 在任何任务中
   知道本机有 fojin-cli 可用、何时该用它(以及何时不该——语义检索/巴利/翻译请走在线 API)。

前提:`fojin` 已安装且在 PATH 中(`cargo install fojin-cli` 或仓库根目录的 `install.sh`)。
建议先跑一次 `fojin data status` 确认数据就绪,agent 会话中即可全程 `--offline`。

其他 agent 框架(LangChain、OpenAI function calling 等)同理:fojin-cli 的
`--json` + 退出码约定就是天然的工具接口,把 CLAUDE-snippet.md 的调用约定翻译成
你框架的 tool description 即可。
