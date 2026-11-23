def parse_line(line):
    line = line.strip()
    if not line or line.startswith("#"):
        return None
    parts = line.split(";")
    if len(parts) != 3:
        return None
    try:
        amount = float(parts[2].replace(",", "."))
    except ValueError:
        return None
    if amount <= 0:
        return None
    return (parts[0].strip(), parts[1].strip().lower(), amount)


def group_rows(rows):
    groups = {}
    for name, cat, amount in rows:
        groups.setdefault(cat, []).append((name, amount))
    return groups


def money(x):
    return "%.2f" % x


def format_group(cat, items):
    out = [cat.upper()]
    subtotal = 0
    for name, amount in sorted(items, key=lambda x: (-x[1], x[0])):
        out.append("  " + name.ljust(12) + money(amount).rjust(9))
        subtotal += amount
    out.append("  " + "total".ljust(12) + money(subtotal).rjust(9))
    out.append("")
    return out, subtotal


def footer(rows):
    if not rows:
        return "Aucune ligne valide"
    best = max(rows, key=lambda r: (r[2], r[0]))
    return "Plus grosse ligne : " + best[0] + " (" + money(best[2]) + ")"


def build_report(lines):
    rows = [r for r in map(parse_line, lines) if r]
    groups = group_rows(rows)
    out, grand = [], 0
    for cat in sorted(groups):
        part, subtotal = format_group(cat, groups[cat])
        out += part
        grand += subtotal
    out.append("TOTAL GENERAL " + money(grand))
    out.append(footer(rows))
    return "\n".join(out)
