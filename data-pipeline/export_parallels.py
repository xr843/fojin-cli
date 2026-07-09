"""Export fojin MITRA parallels into a sanitized public SQLite for fojin-cli.

MAINTAINER-ONLY. Runs against fojin's private Postgres. Emits MITRA data only —
NEVER any fojin-internal column (method / internal ids / provenance). fojin's own
alignment_pairs are excluded by construction (this reads mitra_alignments only).
"""
from __future__ import annotations

import gzip
import hashlib
import os
import sqlite3

# MUST stay byte-for-byte identical to the Rust STRIP_CHARS constant.
STRIP_CHARS = set(" \t\r\n　，。；：！？、,.;:!?“”‘’\"'()（）《》〈〉【】…—-·")


def normalize(text: str, norm_map: dict) -> str:
    return "".join(norm_map.get(c, c) for c in text if c not in STRIP_CHARS)


def build_norm_map() -> dict:
    """1:1 traditional->simplified char fold, derived from OpenCC's char table.
    Only chars that actually change are kept (normalize() falls back to identity)."""
    from opencc import OpenCC

    cc = OpenCC("t2s")
    m = {}
    # Sweep the CJK Unified Ideographs block; keep single-char conversions that differ.
    for cp in range(0x4E00, 0xA000):
        ch = chr(cp)
        conv = cc.convert(ch)
        if len(conv) == 1 and conv != ch:
            m[ch] = conv
    return m


SCHEMA_PATH = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "schema.sql")

SELECT_SQL = """
SELECT ma.zh_text, ma.foreign_lang, ma.foreign_text, ma.confidence,
       t.cbeta_id, t.title_zh, ma.juan_num
FROM mitra_alignments ma
JOIN texts t ON t.id = ma.text_id
WHERE ma.foreign_text <> '' AND ma.zh_text <> ''
"""


def build(db_url: str, out_path: str, version: str = "v1") -> str:
    import psycopg

    norm_map = build_norm_map()

    if os.path.exists(out_path):
        os.remove(out_path)
    sconn = sqlite3.connect(out_path)
    with open(SCHEMA_PATH, encoding="utf-8") as f:
        sconn.executescript(f.read())

    sconn.executemany(
        "INSERT INTO norm_map(from_char,to_char) VALUES (?,?)",
        list(norm_map.items()),
    )

    row_count = 0
    with psycopg.connect(db_url) as pg, pg.cursor(name="export") as cur:
        cur.execute(SELECT_SQL)
        for zh_text, lang, foreign, conf, cbeta_id, title_zh, juan in cur:
            sconn.execute(
                "INSERT INTO parallels(zh_text,zh_norm,foreign_lang,foreign_text,confidence,cbeta_id,title_zh,juan_num)"
                " VALUES (?,?,?,?,?,?,?,?)",
                (zh_text, normalize(zh_text, norm_map), lang, foreign, conf, cbeta_id, title_zh, juan),
            )
            row_count += 1

    meta = {
        "version": version,
        "license": "CC BY-SA 4.0",
        "attribution": "Dharmamitra + fojin.app",
        "row_count": str(row_count),
        "norm_ruleset": "t2s-char-1to1-v1",
    }
    sconn.executemany("INSERT INTO meta(key,value) VALUES (?,?)", list(meta.items()))
    sconn.commit()
    sconn.close()

    raw = open(out_path, "rb").read()
    gz_path = out_path + ".gz"
    with gzip.open(gz_path, "wb") as g:
        g.write(raw)
    digest = hashlib.sha256(open(gz_path, "rb").read()).hexdigest()
    with open(gz_path + ".sha256", "w") as f:
        f.write(digest + "\n")
    print(f"rows={row_count} sha256={digest} out={gz_path}")
    return digest


if __name__ == "__main__":
    url = os.environ["DATABASE_URL"]
    build(url, os.environ.get("OUT", "fojin-parallels-v1.sqlite"))
