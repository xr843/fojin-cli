#!/usr/bin/env python3
"""从 Python 调用 fojin-cli:子进程 + JSON,无需任何第三方依赖。

用法: python3 python_call.py 色即是空
"""

import json
import subprocess
import sys


def parallel(phrase: str) -> dict:
    """查询汉文短语的梵/藏平行;fojin 未安装或数据缺失时抛出异常。"""
    proc = subprocess.run(
        ["fojin", "parallel", phrase, "--json", "--offline"],
        capture_output=True,
        text=True,
    )
    if proc.returncode == 2:
        raise ValueError(f"用法错误: {proc.stderr.strip()}")
    if proc.returncode != 0:
        raise RuntimeError(proc.stderr.strip())
    return json.loads(proc.stdout)


if __name__ == "__main__":
    phrase = sys.argv[1] if len(sys.argv) > 1 else "色即是空"
    result = parallel(phrase)
    print(f"匹配 {result['total']} 组")
    for g in result["groups"][:3]:
        print(f"\n汉  {g['zh_text']}  ({g['title_zh']} {g['cbeta_id']})")
        for p in g["parallels"]:
            print(f"{p['lang']}  {p['text'][:60]}…  [{p['confidence']:.2f}]")
