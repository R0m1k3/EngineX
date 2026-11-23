import json
import os


class Store:
    def __init__(self, path):
        self.path = path

    def _load(self):
        if not os.path.exists(self.path):
            return []
        with open(self.path, encoding="utf-8") as f:
            return json.load(f)

    def _save(self, notes):
        with open(self.path, "w", encoding="utf-8") as f:
            json.dump(notes, f, ensure_ascii=False)

    def all(self):
        return self._load()

    def add(self, text):
        notes = self._load()
        i = max((n["id"] for n in notes), default=0) + 1
        notes.append({"id": i, "text": text, "done": False})
        self._save(notes)
        return i

    def _find(self, notes, i):
        for n in notes:
            if n["id"] == i:
                return n
        raise KeyError(i)

    def done(self, i):
        notes = self._load()
        self._find(notes, i)["done"] = True
        self._save(notes)

    def remove(self, i):
        notes = self._load()
        notes.remove(self._find(notes, i))
        self._save(notes)
