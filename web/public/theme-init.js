// Thème et taille du texte posés avant le premier rendu (pas de flash).
// Fichier à part : la CSP d'EngineX (script-src 'self') refuse les scripts en ligne.
// La suite est dans src/lib/ui/theme.svelte.ts.
(function () {
  var h = document.documentElement;
  try {
    var t = localStorage.getItem('enginex.theme');
    var s = localStorage.getItem('enginex.size');
    if (t === 'light' || t === 'dark' || t === 'system') h.dataset.theme = t;
    if (s === 's' || s === 'm' || s === 'l') h.dataset.size = s;
  } catch (e) {}
})();
