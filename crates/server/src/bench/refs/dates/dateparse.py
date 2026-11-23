import datetime
import re

FR = ["janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre", "octobre", "novembre", "décembre"]
EN = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"]


def _check(y, m, d):
    try:
        datetime.date(y, m, d)
    except ValueError as e:
        raise ValueError(str(e)) from None
    return (y, m, d)


def parse_date(text):
    if not isinstance(text, str):
        raise ValueError("texte attendu")
    t = text.strip()
    if m := re.fullmatch(r"(\d{4})-(\d{2})-(\d{2})", t):
        return _check(int(m[1]), int(m[2]), int(m[3]))
    if m := re.fullmatch(r"(\d{2})/(\d{2})/(\d{4})", t):
        return _check(int(m[3]), int(m[2]), int(m[1]))
    if m := re.fullmatch(r"(\d{1,2}) (\S+) (\d{4})", t):
        name = m[2].lower()
        if name in FR:
            return _check(int(m[3]), FR.index(name) + 1, int(m[1]))
    if m := re.fullmatch(r"([A-Za-z]{3}) (\d{1,2}), (\d{4})", t):
        name = m[1].lower()
        if name in EN:
            return _check(int(m[3]), EN.index(name) + 1, int(m[2]))
    raise ValueError(f"date illisible : {text!r}")
