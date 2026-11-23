# Spécification : carnet de notes en ligne de commande

Réalise un paquet Python `notes` (bibliothèque standard seulement) :

- `notes/store.py` : classe `Store(path)` ; méthodes `add(text) -> int` (renvoie l'id, à partir de 1),
  `all() -> list[dict]` (clés `id`, `text`, `done`), `done(id)`, `remove(id)`. Les données sont écrites
  dans le fichier JSON `path` à chaque changement. Un id inconnu lève `KeyError`. Un nouvel id vaut le plus grand id existant + 1 (1 s'il n'y a
  aucune note). Si le fichier n'existe pas encore, la liste est vide.
- `notes/cli.py` : fonction `main(argv)` qui lit le fichier indiqué par la variable d'environnement
  `NOTES_FILE` (défaut `notes.json`). Commandes : `add TEXTE`, `list`, `done ID`, `rm ID`.
  `list` affiche une ligne par note : `[x] 1 texte` (fait) ou `[ ] 1 texte` ; rien s'il n'y a aucune note.
  Un id inconnu affiche `introuvable` sur la sortie d'erreur et retourne le code 1.
- `notes/__main__.py` : appelle `main` pour que `python3 -m notes list` fonctionne.
- `notes/__init__.py` : peut être vide.
- `README.md` : explique l'usage avec des exemples `python3 -m notes add "acheter du pain"`.
