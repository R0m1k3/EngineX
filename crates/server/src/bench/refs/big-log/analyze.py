import json
import math
import re
from collections import Counter, defaultdict

LINE = re.compile(r'^(\S+) - - \[[^\]]+\] "([A-Z]+) (\S+) HTTP/1\.[01]" (\d{3}) (\d+) (\d+)ms$')
total, errors, ips, lat = 0, 0, Counter(), defaultdict(list)
with open("access.log", encoding="utf-8") as f:
    for raw in f:
        m = LINE.match(raw.rstrip("\n"))
        if not m:
            continue
        ip, _, path, status, _, ms = m.groups()
        total += 1
        if 500 <= int(status) <= 599:
            errors += 1
            ips[ip] += 1
        lat[path.split("?")[0]].append(int(ms))
p95 = {r: sorted(v)[math.ceil(0.95 * len(v)) - 1] for r, v in lat.items()}
top = [{"ip": i, "count": n} for i, n in sorted(ips.items(), key=lambda x: (-x[1], x[0]))[:5]]
json.dump({"total_requests": total, "errors_5xx": errors, "top_ips_5xx": top, "p95_ms": p95}, open("report.json", "w"), indent=2)
