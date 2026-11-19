---
name: memory-compaction
description: Compacter un memory.md d'EngineX devenu long (avertissement « compacte-le bientôt » ou refus à 16 Kio) en fusionnant les doublons et en retirant les faits périmés, avec mcp__enginex__mem_read puis mem_write.
---

# Compacter la mémoire d'EngineX

À faire quand un outil `mem_*` signale que `memory.md` approche ou dépasse 16 Kio, ou quand Michael le demande.

1. Lis le fichier : `mcp__enginex__mem_read {level}`.
2. Prépare la nouvelle version :
   - fusionne les lignes qui disent la même chose (garde la date la plus récente) ;
   - retire les faits qui ne sont plus vrais ou ne servent plus ;
   - garde chaque fait encore vrai, même ancien ; dans le doute, garde-le ;
   - une ligne par fait, au format `- AAAA-MM-JJ : fait`.
3. Écris-la en une fois : `mcp__enginex__mem_write {level, content}`. Si l'outil répond que le fichier a changé,
   relis-le et recommence à l'étape 2.
4. Relis le résultat avec `mem_read` pour vérifier qu'aucun fait utile n'a disparu.

Chaque version précédente reste dans l'historique de la page Mémoire : Michael peut la restaurer.
