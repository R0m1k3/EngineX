// ask() : remplace window.prompt() par une modale (montée par <AskHost/> dans Sessions).
export interface AskOpts {
  title: string;
  label: string;
  value?: string;
  confirm?: string;
}
interface Pending extends AskOpts {
  resolve: (v: string | null) => void;
}

export const asking = $state<{ current: Pending | null }>({ current: null });

/** Texte saisi (sans espaces autour), ou `null` si annulé ou vide. */
export function ask(opts: AskOpts): Promise<string | null> {
  asking.current?.resolve(null);
  return new Promise((resolve) => (asking.current = { ...opts, resolve }));
}

export function answer(v: string | null) {
  const p = asking.current;
  asking.current = null;
  p?.resolve(v?.trim() ? v.trim() : null);
}
