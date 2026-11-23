#!/usr/bin/env python3
"""Rejoue les contrôles de la page Tester sur les solutions de référence
(crates/server/src/bench/refs/<test>/) ou sur l'état de départ, sans sandbox ni
modèle : shell en local, pages dans un Chromium Playwright.

    BENCH_DUMP=/tmp/bench.json cargo test -p enginex-server --lib dump_for_refs -- --ignored
    python3 scripts/bench-refs.py /tmp/bench.json [--start] [--only clé,clé]

Attendu : en mode référence, tous les contrôles passent (sauf « log », lus dans le
journal d'une vraie session, non rejoués ici). En mode --start, au moins un
contrôle essentiel échoue pour chaque test (sinon le test ne prouve rien).
"""
import argparse
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
REFS = os.path.join(ROOT, "crates/server/src/bench/refs")


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def write(base, rel, content):
    p = os.path.join(base, rel)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, "w", encoding="utf-8") as f:
        f.write(content)


def sh(script, cwd, timeout=240):
    env = dict(os.environ, BENCH_DIR=cwd, BENCH_HIDDEN=os.path.join(cwd, ".bench-hidden"))
    try:
        r = subprocess.run(["bash", "-lc", script], cwd=cwd, env=env, capture_output=True, text=True, timeout=timeout)
        return r.returncode, (r.stdout + r.stderr)[-1500:]
    except subprocess.TimeoutExpired:
        return -1, "délai dépassé"


def run_case(case, start_mode, browser):
    d = tempfile.mkdtemp(prefix=f"bench-{case['key']}-")
    for rel, content in case["files"]:
        write(d, rel, content)
    if case["setup"]:
        code, out = sh(case["setup"], d)
        assert code == 0, f"setup : {out}"
    ref = os.path.join(REFS, case["key"])
    if not start_mode and os.path.isdir(ref):
        for dirpath, _, names in os.walk(ref):
            for n in names:
                if n == "COMMIT":
                    continue
                src = os.path.join(dirpath, n)
                dst = os.path.join(d, os.path.relpath(src, ref))
                os.makedirs(os.path.dirname(dst), exist_ok=True)
                shutil.copy(src, dst)
        if os.path.exists(os.path.join(ref, "COMMIT")):
            msg = open(os.path.join(ref, "COMMIT"), encoding="utf-8").read().strip()
            sh(f"git add -A && git commit -qm {json.dumps(msg)}", d)
        if case["key"] == "big-log":
            sh("python3 analyze.py", d)
    for rel, content in case["hidden"]:
        write(d, ".bench-hidden/" + rel, content)
    write(d, ".bench-hidden/benchh.py", case["benchh"])

    srv, port = None, None
    if any(c["type"] in ("page", "console") for c in case["checks"]):
        port = free_port()
        srv = subprocess.Popen([sys.executable, "-m", "http.server", str(port), "--bind", "127.0.0.1"], cwd=d,
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(0.6)
    results, errors, opened = [], [], False
    try:
        for c in case["checks"]:
            if c["type"] == "shell":
                code, out = sh(c["script"], d)
                results.append((c, code == 0, out.strip().splitlines()[-1] if out.strip() else ""))
            elif c["type"] == "page":
                phone = c["phone"]
                ctx = browser.new_context(viewport={"width": 390, "height": 844} if phone else {"width": 1280, "height": 800},
                                          is_mobile=phone, device_scale_factor=1)
                page = ctx.new_page()
                page.on("pageerror", lambda e: errors.append(f"exception : {e}"))
                page.on("console", lambda m: m.type == "error" and not (m.location or {}).get("url", "").endswith("favicon.ico")
                        and errors.append(f"console.error : {m.text}"))
                url = f"http://127.0.0.1:{port}/{c['path']}"
                ok, why = True, "scénario réussi"
                try:
                    page.goto(url)
                    opened = True
                    time.sleep(0.4)
                    if phone and page.evaluate("innerWidth") > 500:
                        ok, why = False, "pas de balise viewport"
                    for st in c["steps"] if ok else []:
                        if st == "@reload":
                            page.reload()
                            time.sleep(0.4)
                        elif st == "@wait":
                            time.sleep(0.7)
                            page.wait_for_load_state("load")
                            time.sleep(0.3)
                        else:
                            r = page.evaluate(st)
                            if r != "OK":
                                ok, why = False, r
                                break
                except Exception as e:
                    ok, why = False, f"exception Playwright : {e}"
                ctx.close()
                results.append((c, ok, why))
            elif c["type"] == "console":
                results.append((c, None, "plus bas"))
            else:
                results.append((c, None, "journal : non rejoué"))
        results = [(c, (not errors and opened) if c["type"] == "console" else ok,
                    ("; ".join(errors)[:300] or "aucune erreur") if c["type"] == "console" else why)
                   for c, ok, why in results]
    finally:
        if srv:
            srv.kill()
        shutil.rmtree(d, ignore_errors=True)
    return results


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("dump")
    ap.add_argument("--start", action="store_true", help="état de départ au lieu des références")
    ap.add_argument("--only", default="")
    a = ap.parse_args()
    cases = json.load(open(a.dump, encoding="utf-8"))
    only = set(filter(None, a.only.split(",")))
    from playwright.sync_api import sync_playwright

    bad = 0
    with sync_playwright() as p:
        exe = os.environ.get("CHROME")
        browser = p.chromium.launch(executable_path=exe) if exe else p.chromium.launch()
        for case in cases:
            if only and case["key"] not in only:
                continue
            res = run_case(case, a.start, browser)
            print(f"\n== {case['key']}")
            for c, ok, why in res:
                mark = "  " if ok is None else ("OK" if ok else "KO")
                print(f"  {mark} {c['name']}" + ("" if ok else f"  ← {why}"))
            if a.start:
                ess = [ok for c, ok, _ in res if ok is not None and any(
                    k["name"] == c["name"] for k in case["checks"])]
                if all(ok for ok in ess):
                    print("  !! l'état de départ passe tout : le test ne prouve rien")
                    bad += 1
            elif any(ok is False for _, ok, _ in res):
                bad += 1
        browser.close()
    print(f"\n{bad} test(s) à revoir")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
