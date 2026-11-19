// Thème (Système / Clair / Sombre) et taille du texte (S / M / L), mémorisés
// dans le navigateur. public/theme-init.js les pose sur <html>
// avant le premier rendu ; ce module prend la suite (bascules, Système suivi
// en direct). Les éditeurs (CodeMirror, xterm) s'abonnent par onThemeChange.

export type ThemeChoice = 'system' | 'light' | 'dark';
export type TextSize = 's' | 'm' | 'l';

const THEME_KEY = 'enginex.theme';
const SIZE_KEY = 'enginex.size';
/** Couleur de la barre du navigateur / de l'appli installée (jeton `panel`). */
const BAR = { light: '#f2f4f6', dark: '#1c1f24' };

const read = <T extends string>(key: string, ok: readonly T[], dflt: T): T => {
  try {
    const v = localStorage.getItem(key) as T | null;
    return v && ok.includes(v) ? v : dflt;
  } catch {
    return dflt;
  }
};
const store = (key: string, v: string) => {
  try {
    localStorage.setItem(key, v);
  } catch {
    /* navigation privée : réglage gardé pour la page seulement */
  }
};

const media = typeof matchMedia === 'function' ? matchMedia('(prefers-color-scheme: dark)') : null;

export const ui = $state({
  theme: read<ThemeChoice>(THEME_KEY, ['system', 'light', 'dark'], 'system'),
  size: read<TextSize>(SIZE_KEY, ['s', 'm', 'l'], 'm'),
  /** Thème réellement affiché (Système résolu). */
  dark: false,
});

type Listener = (dark: boolean) => void;
const listeners = new Set<Listener>();

function apply() {
  const dark = ui.theme === 'dark' || (ui.theme === 'system' && !!media?.matches);
  const changed = dark !== ui.dark;
  ui.dark = dark;
  const h = document.documentElement;
  h.dataset.theme = ui.theme;
  h.dataset.size = ui.size;
  document.querySelectorAll<HTMLMetaElement>('meta[name="theme-color"]').forEach((m) => {
    // Une balise par schéma (media) quand le thème suit le système ; une seule couleur sinon.
    const scheme = m.media.includes('dark') ? 'dark' : m.media.includes('light') ? 'light' : null;
    m.content = ui.theme === 'system' && scheme ? BAR[scheme] : BAR[dark ? 'dark' : 'light'];
  });
  if (changed) listeners.forEach((fn) => fn(dark));
}

export function setTheme(t: ThemeChoice) {
  ui.theme = t;
  store(THEME_KEY, t);
  apply();
}

export function setSize(s: TextSize) {
  ui.size = s;
  store(SIZE_KEY, s);
  apply();
}

const ORDER: ThemeChoice[] = ['system', 'light', 'dark'];
export const cycleTheme = () => setTheme(ORDER[(ORDER.indexOf(ui.theme) + 1) % ORDER.length]);

export const THEME_LABEL: Record<ThemeChoice, string> = { system: 'Système', light: 'Clair', dark: 'Sombre' };
export const SIZE_LABEL: Record<TextSize, string> = { s: 'Petit', m: 'Moyen', l: 'Grand' };

/** Appelé à chaque changement du thème affiché ; renvoie la fonction de désabonnement. */
export function onThemeChange(fn: Listener): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

/** Thème affiché maintenant (hors composant). */
export const isDark = () => ui.dark;

/** Valeur courante d'un jeton CSS (`--bg`…), pour les bibliothèques qui veulent des couleurs. */
export const token = (name: string) =>
  getComputedStyle(document.documentElement).getPropertyValue(`--${name}`).trim();

media?.addEventListener('change', apply);
apply();
