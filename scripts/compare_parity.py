#!/usr/bin/env python3
"""Compare abcop vs rubocop Metrics/AbcSize JSON output.

usage: compare_parity.py rubocop.json abcop.json [abcop_path_prefix_to_strip]
"""
import json
import re
import sys


def load_rubocop(path):
    data = json.load(open(path))
    out = {}
    for f in data["files"]:
        rel = f["path"]
        for o in f["offenses"]:
            if o["cop_name"] != "Metrics/AbcSize":
                continue
            m = re.search(r"\[<(\d+), (\d+), (\d+)> ([0-9.e+]+)", o["message"])
            if not m:
                continue
            key = (rel, o["location"]["line"])
            out[key] = (f"<{m.group(1)}, {m.group(2)}, {m.group(3)}>", float(m.group(4)))
    return out


def load_abcop(path, strip):
    data = json.load(open(path))
    out = {}
    for o in data["diagnostics"]:
        if o["rule"] != "Metrics/AbcSize":
            continue
        key = (o["file"].removeprefix(strip), o["line"])
        out[key] = (o["vector"], float(o["score"]))
    return out


a = load_rubocop(sys.argv[1])
b = load_abcop(sys.argv[2], sys.argv[3] if len(sys.argv) > 3 else "")

keys = set(a) | set(b)
both = [k for k in keys if k in a and k in b]
vec_match = sum(1 for k in both if a[k][0] == b[k][0])
score_match = sum(1 for k in both if abs(a[k][1] - b[k][1]) < 0.005)
only_a = sorted(k for k in a if k not in b)
only_b = sorted(k for k in b if k not in a)
mismatched = [(k, a[k], b[k]) for k in both if a[k][0] != b[k][0]]

print(f"rubocop offenses : {len(a)}")
print(f"abcop offenses   : {len(b)}")
print(f"joined           : {len(both)}  vector-exact {vec_match}  score-exact {score_match}")
print(f"only in rubocop  : {len(only_a)}")
print(f"only in abcop    : {len(only_b)}")
print("\nsample missing from abcop:")
for k in only_a[:12]:
    print(" ", k, a[k])
print("\nsample extra in abcop:")
for k in only_b[:12]:
    print(" ", k, b[k])
print("\nsample vector mismatches:")
for k, va, vb in mismatched[:15]:
    print(f"  {k} rubocop={va} abcop={vb}")
