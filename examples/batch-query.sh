#!/bin/sh
# 批量查询:每行一个短语的文本文件 → TSV(短语、是否命中、匹配组数)。
# 用法: ./batch-query.sh phrases.txt > result.tsv
set -eu
while IFS= read -r phrase; do
  [ -n "$phrase" ] || continue
  json=$(fojin parallel "$phrase" --json --offline 2>/dev/null) || {
    printf '%s\tERROR\t0\n' "$phrase"
    continue
  }
  matched=$(printf '%s' "$json" | jq -r '.matched')
  total=$(printf '%s' "$json" | jq -r '.total')
  printf '%s\t%s\t%s\n' "$phrase" "$matched" "$total"
done < "${1:?用法: $0 <每行一个短语的文件>}"
