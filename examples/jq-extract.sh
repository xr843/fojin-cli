#!/bin/sh
# 提取一段汉文的全部梵文平行(一行一条),需要 jq。
# 用法: ./jq-extract.sh "色即是空"
set -eu
fojin parallel "${1:?用法: $0 <汉文短语>}" --json --offline --all \
  | jq -r '.groups[].parallels[] | select(.lang=="sa") | .text'
