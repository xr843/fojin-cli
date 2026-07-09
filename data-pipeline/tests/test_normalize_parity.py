import os, sys

HERE = os.path.dirname(os.path.abspath(__file__))
PIPE = os.path.dirname(HERE)                 # data-pipeline/
ROOT = os.path.dirname(PIPE)                 # repo root
sys.path.insert(0, PIPE)

from export_parallels import normalize  # noqa: E402


def _load_map():
    m = {}
    with open(os.path.join(ROOT, "tests/fixtures/norm_map.tsv"), encoding="utf-8") as f:
        for line in f:
            a, b = line.rstrip("\n").split("\t")
            m[a] = b
    return m


def test_normalize_matches_golden_cases():
    m = _load_map()
    with open(os.path.join(ROOT, "tests/fixtures/norm_cases.tsv"), encoding="utf-8") as f:
        for line in f:
            inp, exp = line.rstrip("\n").split("\t")
            assert normalize(inp, m) == exp, f"input={inp!r}"
