"""Petit harnais des tests cachés : chaque cas est une fonction ; dernière ligne
`BENCH_SCORE réussis/total` (points au prorata), code 0 si tout passe."""
import os
import sys

sys.path.insert(0, os.getcwd())
_CASES = []


def case(name):
    def deco(f):
        _CASES.append((name, f))
        return f
    return deco


def raises(exc, f, *a, **k):
    try:
        f(*a, **k)
    except exc:
        return
    except Exception as e:  # mauvaise exception
        raise AssertionError(f"{type(e).__name__} au lieu de {exc.__name__}")
    raise AssertionError(f"pas de {exc.__name__}")


def run():
    good = 0
    for name, f in _CASES:
        try:
            f()
            good += 1
        except Exception as e:
            msg = str(e).replace("\n", " ")[:200]
            print(f"KO {name} : {type(e).__name__} {msg}")
    print(f"BENCH_SCORE {good}/{len(_CASES)}")
    sys.exit(0 if good == len(_CASES) else 1)
