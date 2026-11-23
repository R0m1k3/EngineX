import sys

from benchh import case, raises, run

from dateparse import parse_date

GOOD = [
    ("2024-02-29", (2024, 2, 29)), ("29/02/2024", (2024, 2, 29)), ("1 janvier 2027", (2027, 1, 1)),
    ("22 Novembre 2026", (2026, 11, 22)), ("3 février 2026", (2026, 2, 3)), ("15 août 2026", (2026, 8, 15)),
    ("1 décembre 2026", (2026, 12, 1)), ("Jan 1, 2027", (2027, 1, 1)), ("Dec 31, 2026", (2026, 12, 31)),
    ("Feb 29, 2024", (2024, 2, 29)), ("31/12/2026", (2026, 12, 31)), ("2026-12-31", (2026, 12, 31)),
    ("10 mars 2026", (2026, 3, 10)), ("20 JUIN 2026", (2026, 6, 20)), ("7 juillet 2026", (2026, 7, 7)),
]
BAD = ["2026-02-29", "29/02/2025", "31 avril 2026", "Feb 30, 2026", "2026-13-01", "00/10/2026",
       "22 novembr 2026", "bonjour", "", "2026/11/22"]

which = sys.argv[1] if len(sys.argv) > 1 else "good"
if which == "good":
    for s, want in GOOD:
        def f(s=s, want=want):
            got = parse_date(s)
            assert tuple(got) == want, f"{s!r} -> {got!r}"
        case(repr(s))(f)
else:
    for s in BAD:
        case(repr(s))(lambda s=s: raises(ValueError, parse_date, s))
run()
