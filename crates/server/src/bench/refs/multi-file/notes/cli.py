import os
import sys

from notes.store import Store


def main(argv):
    store = Store(os.environ.get("NOTES_FILE", "notes.json"))
    if not argv:
        print("usage : add TEXTE | list | done ID | rm ID", file=sys.stderr)
        return 2
    cmd, args = argv[0], argv[1:]
    try:
        if cmd == "add":
            print(store.add(" ".join(args)))
        elif cmd == "list":
            for n in store.all():
                print(f"[{'x' if n['done'] else ' '}] {n['id']} {n['text']}")
        elif cmd == "done":
            store.done(int(args[0]))
        elif cmd == "rm":
            store.remove(int(args[0]))
        else:
            print("commande inconnue", file=sys.stderr)
            return 2
    except (KeyError, ValueError, IndexError):
        print("introuvable", file=sys.stderr)
        return 1
    return 0
