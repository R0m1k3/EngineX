// toast('Preset enregistré', 'ok') : message bref en bas à droite (au-dessus de la barre d'onglets au téléphone).
export type ToastKind = 'info' | 'ok' | 'warn' | 'error';
export interface ToastItem {
  id: number;
  message: string;
  kind: ToastKind;
}

export const toasts = $state<ToastItem[]>([]);
let next = 1;

export function dismiss(id: number) {
  const i = toasts.findIndex((t) => t.id === id);
  if (i >= 0) toasts.splice(i, 1);
}

/** `ms` = durée d'affichage (0 = jusqu'au clic) ; les erreurs restent 8 s par défaut. */
export function toast(message: string, kind: ToastKind = 'info', ms?: number): number {
  const id = next++;
  toasts.push({ id, message, kind });
  if (toasts.length > 4) toasts.shift();
  const life = ms ?? (kind === 'error' ? 8000 : 4000);
  if (life > 0) setTimeout(() => dismiss(id), life);
  return id;
}
