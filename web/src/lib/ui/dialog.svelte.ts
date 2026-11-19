// confirmDialog() et promptDialog() : remplacent window.confirm() et window.prompt() par une modale
// (montée par <DialogHost/> dans la coquille). Une seule à la fois, les suivantes attendent.
export interface ConfirmOpts {
  title?: string;
  /** Texte du bouton qui confirme (« Supprimer »…). */
  confirm?: string;
  cancel?: string;
  /** Action destructrice : bouton rouge. */
  danger?: boolean;
}

export interface PromptOpts {
  title?: string;
  value?: string;
  placeholder?: string;
  confirm?: string;
  cancel?: string;
  /** Aide sous le champ. */
  hint?: string;
  /** Champ en police à chasse fixe (noms de fichiers, branches). */
  mono?: boolean;
  /** Vide autorisé (rend ''), sinon le bouton reste grisé et le texte est rendu sans espaces autour. */
  allowEmpty?: boolean;
  /** Sélectionner le nom sans l'extension (renommer un fichier). */
  selectStem?: boolean;
  maxlength?: number;
}

type Pending =
  | ({ kind: 'confirm'; message: string; resolve: (ok: boolean) => void } & Required<ConfirmOpts>)
  | ({ kind: 'prompt'; message: string; resolve: (v: string | null) => void } & PromptOpts & {
        title: string;
        confirm: string;
        cancel: string;
      });

export const dialogs = $state<{ current: Pending | null; queue: Pending[] }>({ current: null, queue: [] });

function push(p: Pending) {
  if (dialogs.current) dialogs.queue.push(p);
  else dialogs.current = p;
}

export function confirmDialog(message: string, opts: ConfirmOpts = {}): Promise<boolean> {
  return new Promise((resolve) =>
    push({
      kind: 'confirm',
      message,
      resolve,
      title: opts.title ?? 'Confirmer',
      confirm: opts.confirm ?? 'Confirmer',
      cancel: opts.cancel ?? 'Annuler',
      danger: opts.danger ?? false,
    }),
  );
}

/** Texte saisi (`message` = libellé du champ), ou `null` si la fenêtre est fermée sans valider. */
export function promptDialog(message: string, opts: PromptOpts = {}): Promise<string | null> {
  return new Promise((resolve) =>
    push({
      ...opts,
      kind: 'prompt',
      message,
      resolve,
      title: opts.title ?? 'Saisir',
      confirm: opts.confirm ?? 'Valider',
      cancel: opts.cancel ?? 'Annuler',
    }),
  );
}

/** Ferme la modale en cours : `true`/`false` pour une confirmation, le texte ou `null` pour une saisie. */
export function settle(v: boolean | string | null) {
  const p = dialogs.current;
  if (!p) return;
  if (p.kind === 'confirm') p.resolve(v === true);
  else if (typeof v !== 'string') p.resolve(null);
  else if (p.allowEmpty) p.resolve(v);
  else p.resolve(v.trim() ? v.trim() : null);
  dialogs.current = dialogs.queue.shift() ?? null;
}
