def build_report(lines):
    # lines : liste de "nom;categorie;montant" ; renvoie le rapport en texte
    rows = []
    for line in lines:
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split(";")
        if len(parts) != 3:
            continue
        name = parts[0].strip()
        cat = parts[1].strip().lower()
        try:
            amount = float(parts[2].replace(",", "."))
        except ValueError:
            continue
        if amount <= 0:
            continue
        rows.append((name, cat, amount))
    groups = {}
    for name, cat, amount in rows:
        if cat not in groups:
            groups[cat] = []
        groups[cat].append((name, amount))
    out = []
    grand = 0
    for cat in sorted(groups):
        items = sorted(groups[cat], key=lambda x: (-x[1], x[0]))
        subtotal = 0
        out.append(cat.upper())
        for name, amount in items:
            out.append("  " + name.ljust(12) + ("%.2f" % amount).rjust(9))
            subtotal += amount
        out.append("  " + "total".ljust(12) + ("%.2f" % subtotal).rjust(9))
        out.append("")
        grand += subtotal
    out.append("TOTAL GENERAL " + ("%.2f" % grand))
    if rows:
        best = max(rows, key=lambda r: (r[2], r[0]))
        out.append("Plus grosse ligne : " + best[0] + " (" + ("%.2f" % best[2]) + ")")
    else:
        out.append("Aucune ligne valide")
    return "\n".join(out)
