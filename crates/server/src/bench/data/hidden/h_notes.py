import json
import os
import subprocess
import sys
import tempfile

from benchh import case, raises, run

from notes.store import Store

D = tempfile.mkdtemp()


def cli(path, *args, cwd=None):
    env = dict(os.environ)
    env.pop("NOTES_FILE", None)
    if path:
        env["NOTES_FILE"] = path
    env["PYTHONPATH"] = os.getcwd()
    return subprocess.run([sys.executable, "-m", "notes", *args], capture_output=True, text=True, env=env, cwd=cwd)


@case("fichier absent : liste vide")
def _():
    assert Store(os.path.join(D, "rien.json")).all() == []

@case("nouvel id = plus grand + 1, même après réouverture")
def _():
    p = os.path.join(D, "ids.json")
    s = Store(p)
    for t in "abc":
        s.add(t)
    s.remove(2)
    assert Store(p).add("d") == 4

@case("remove d'un id inconnu -> KeyError")
def _():
    raises(KeyError, Store(os.path.join(D, "k.json")).remove, 5)

@case("le fichier est un JSON valide")
def _():
    p = os.path.join(D, "j.json")
    Store(p).add("x")
    json.load(open(p, encoding="utf-8"))

@case("CLI : done d'un id inconnu -> code 1, introuvable")
def _():
    r = cli(os.path.join(D, "c1.json"), "done", "3")
    assert r.returncode == 1 and "introuvable" in r.stderr, (r.returncode, r.stderr)

@case("CLI : list vide n'affiche rien, code 0")
def _():
    r = cli(os.path.join(D, "c2.json"), "list")
    assert r.returncode == 0 and r.stdout.strip() == "", (r.returncode, r.stdout)

@case("CLI : rm puis list")
def _():
    p = os.path.join(D, "c3.json")
    cli(p, "add", "un"); cli(p, "add", "deux"); cli(p, "rm", "1")
    assert cli(p, "list").stdout.splitlines() == ["[ ] 2 deux"]

@case("CLI : NOTES_FILE absent -> notes.json du dossier courant")
def _():
    d = tempfile.mkdtemp()
    cli(None, "add", "défaut", cwd=d)
    assert os.path.exists(os.path.join(d, "notes.json"))

run()
