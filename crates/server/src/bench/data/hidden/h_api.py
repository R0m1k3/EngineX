"""Tests cachés de server.py : `python3 h_api.py <groupe> <port>`."""
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request

from benchh import case, run

group, port = sys.argv[1], int(sys.argv[2])
base = f"http://127.0.0.1:{port}"
srv = subprocess.Popen([sys.executable, "server.py"], env=dict(os.environ, PORT=str(port)),
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def req(method, path, body=None, raw=None, ctype="application/json"):
    data = raw if raw is not None else (json.dumps(body).encode() if body is not None else None)
    r = urllib.request.Request(base + path, data=data, method=method)
    if data is not None:
        r.add_header("Content-Type", ctype)
    try:
        with urllib.request.urlopen(r, timeout=5) as resp:
            return resp.status, resp.read().decode(), resp.headers.get("Content-Type", "")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode(), e.headers.get("Content-Type", "")


for _ in range(60):
    try:
        req("GET", "/items")
        break
    except Exception:
        time.sleep(0.1)


def create(name):
    s, b, _ = req("POST", "/items", {"name": name})
    assert s == 201, f"POST -> {s}"
    return json.loads(b)


try:
    if group == "basic":
        @case("POST /items -> 201, id et name renvoyés")
        def _():
            o = create("x")
            assert "id" in o and o.get("name") == "x", o
    elif group == "read":
        @case("GET /items liste l'objet créé")
        def _():
            create("zz")
            s, b, _ = req("GET", "/items")
            assert s == 200 and any(i.get("name") == "zz" for i in json.loads(b)), (s, b)

        @case("GET /items/<id> renvoie l'objet")
        def _():
            o = create("yy")
            s, b, _ = req("GET", f"/items/{o['id']}")
            assert s == 200 and json.loads(b).get("name") == "yy", (s, b)
    elif group == "delete":
        @case("id inconnu -> 404")
        def _():
            assert req("GET", "/items/99999")[0] == 404

        @case("DELETE -> 204 puis GET -> 404")
        def _():
            o = create("d")
            assert req("DELETE", f"/items/{o['id']}")[0] == 204
            assert req("GET", f"/items/{o['id']}")[0] == 404
    else:
        @case("POST sans name -> 400")
        def _():
            assert req("POST", "/items", {"nom": "x"})[0] == 400

        @case("POST au JSON invalide -> 400")
        def _():
            assert req("POST", "/items", raw=b"{pas du json")[0] == 400

        @case("réponses en application/json")
        def _():
            create("j")
            assert "application/json" in req("GET", "/items")[2]

        @case("ids différents à chaque création")
        def _():
            assert create("a")["id"] != create("b")["id"]

        @case("DELETE d'un id inconnu -> 404")
        def _():
            assert req("DELETE", "/items/424242")[0] == 404

        @case("route inconnue -> 404")
        def _():
            assert req("GET", "/nimporte")[0] == 404

        @case("la liste n'a plus l'objet supprimé")
        def _():
            o = create("parti")
            req("DELETE", f"/items/{o['id']}")
            assert not any(i.get("id") == o["id"] for i in json.loads(req("GET", "/items")[1]))

        @case("le nom garde ses accents")
        def _():
            o = create("café crème")
            assert json.loads(req("GET", f"/items/{o['id']}")[1]).get("name") == "café crème"
    run()
finally:
    srv.kill()
