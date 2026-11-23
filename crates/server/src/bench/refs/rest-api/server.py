import json
import os
import re
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ITEMS = {}
NEXT = [1]


class H(BaseHTTPRequestHandler):
    def send(self, code, obj=None):
        body = b"" if obj is None else json.dumps(obj, ensure_ascii=False).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def item_id(self):
        m = re.fullmatch(r"/items/(\d+)", self.path)
        return int(m[1]) if m else None

    def do_GET(self):
        if self.path == "/items":
            return self.send(200, list(ITEMS.values()))
        i = self.item_id()
        if i in ITEMS:
            return self.send(200, ITEMS[i])
        self.send(404, {"error": "introuvable"})

    def do_POST(self):
        if self.path != "/items":
            return self.send(404, {"error": "introuvable"})
        n = int(self.headers.get("Content-Length") or 0)
        try:
            data = json.loads(self.rfile.read(n) or b"null")
            name = data["name"]
            assert isinstance(name, str)
        except Exception:
            return self.send(400, {"error": "name attendu"})
        obj = {"id": NEXT[0], "name": name}
        ITEMS[NEXT[0]] = obj
        NEXT[0] += 1
        self.send(201, obj)

    def do_DELETE(self):
        i = self.item_id()
        if i in ITEMS:
            del ITEMS[i]
            return self.send(204)
        self.send(404, {"error": "introuvable"})

    def log_message(self, *a):
        pass


if __name__ == "__main__":
    ThreadingHTTPServer(("0.0.0.0", int(os.environ.get("PORT", "8000"))), H).serve_forever()
