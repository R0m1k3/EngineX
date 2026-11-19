// Téléphone (< 768 px, même seuil que la coquille) : la liste et le fil passent en plein écran.
const query = typeof matchMedia === 'function' ? matchMedia('(max-width: 767px)') : null;

export const screen = $state({ phone: !!query?.matches });
query?.addEventListener('change', (e) => (screen.phone = e.matches));

/** Largeur mémorisée d'un volet (localStorage), bornée. */
export function stored(key: string, fallback: number, min: number, max: number): number {
  try {
    const v = Number(localStorage.getItem(key));
    return v ? Math.min(max, Math.max(min, v)) : fallback;
  } catch {
    return fallback;
  }
}
export function store(key: string, v: number | string) {
  try {
    localStorage.setItem(key, String(v));
  } catch {
    /* stockage plein ou refusé : largeur non gardée */
  }
}
