"""Vérité calculée depuis access.log ; compare avec report.json. Argument : keys|total|errors|top|p95."""
import json
import math
import re
import sys
from collections import Counter, defaultdict

LINE = re.compile(r'^(\S+) - - \[[^\]]+\] "([A-Z]+) (\S+) HTTP/1\.[01]" (\d{3}) (\d+) (\d+)ms$')
total, errors, ips, lat = 0, 0, Counter(), defaultdict(list)
for raw in open("access.log", encoding="utf-8"):
    m = LINE.match(raw.rstrip("\n"))
    if not m:
        continue
    ip, _, path, status, _, ms = m.groups()
    total += 1
    if 500 <= int(status) <= 599:
        errors += 1
        ips[ip] += 1
    lat[path.split("?")[0]].append(int(ms))
top = [{"ip": ip, "count": n} for ip, n in sorted(ips.items(), key=lambda x: (-x[1], x[0]))[:5]]
p95 = {}
for route, v in lat.items():
    v.sort()
    p95[route] = v[math.ceil(0.95 * len(v)) - 1]

try:
    rep = json.load(open("report.json", encoding="utf-8"))
except Exception as e:
    print("report.json illisible :", e)
    sys.exit(1)
what = sys.argv[1]
if what == "keys":
    miss = [k for k in ("total_requests", "errors_5xx", "top_ips_5xx", "p95_ms") if k not in rep]
    print("clés manquantes :", miss) if miss else print("4 clés présentes")
    sys.exit(1 if miss else 0)
if what == "total":
    print("attendu", total, "obtenu", rep.get("total_requests"))
    sys.exit(0 if rep.get("total_requests") == total else 1)
if what == "errors":
    print("attendu", errors, "obtenu", rep.get("errors_5xx"))
    sys.exit(0 if rep.get("errors_5xx") == errors else 1)
if what == "top":
    got = [{"ip": d.get("ip"), "count": d.get("count")} for d in rep.get("top_ips_5xx") or [] if isinstance(d, dict)]
    print("attendu", top)
    print("obtenu ", got)
    sys.exit(0 if got == top else 1)
got = rep.get("p95_ms") or {}
good = sum(1 for r, v in p95.items() if isinstance(got, dict) and got.get(r) is not None and float(got.get(r)) == v)
for r, v in sorted(p95.items()):
    g = got.get(r) if isinstance(got, dict) else None
    if g is None or float(g) != v:
        print(f"KO {r} : attendu {v}, obtenu {g}")
extra = [r for r in got if r not in p95] if isinstance(got, dict) else []
if extra:
    print("routes en trop :", extra)
print(f"BENCH_SCORE {good}/{len(p95)}")
sys.exit(0 if good == len(p95) and not extra else 1)
