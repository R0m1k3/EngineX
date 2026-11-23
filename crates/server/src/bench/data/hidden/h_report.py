"""Le build_report refactoré donne-t-il le même texte que l'original sur 300 entrées ?"""
import random

from benchh import case, run

import report
import report_orig

rnd = random.Random(20261123)
NAMES = ["stylo", "cahier", "lampe", "tapis", "plante", "gomme", "règle", "vase", "a", "zz"]
CATS = ["papeterie", "Papeterie", "maison", " MAISON ", "jardin", "Jardin"]


def line():
    k = rnd.random()
    if k < 0.06:
        return "# commentaire"
    if k < 0.1:
        return ""
    if k < 0.14:
        return "ligne sans séparateur"
    amount = rnd.choice(["%.2f" % rnd.uniform(-5, 50), "%d" % rnd.randint(0, 40), "%d,%d" % (rnd.randint(0, 9), rnd.randint(0, 99)), "abc", "12.5"])
    parts = [rnd.choice(NAMES), rnd.choice(CATS), amount]
    if rnd.random() < 0.05:
        parts.append("trop")
    return " " * rnd.randint(0, 2) + ";".join(parts) + " " * rnd.randint(0, 2)


for i in range(300):
    lines = [line() for _ in range(rnd.randint(0, 14))]

    def f(lines=lines):
        want = report_orig.build_report(lines)
        got = report.build_report(lines)
        assert got == want, f"entrée {lines!r}"
    case(f"entrée {i}")(f)
run()
