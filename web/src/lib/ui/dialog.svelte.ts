// confirmDialog() : remplace window.confirm() par une modale (montée par <DialogHost/> dans la coquille).
export interface ConfirmOpts {
  title?: string;
  /** Texte du bouton qui confirme (« Supprimer »…). */
  confirm?: string;
  cancel?: string;
  /** Action destructrice : bouton rouge. */
  danger?: boolean;
}

interface Pending extends Required<ConfirmOpts> {
  message: string;
  resolve: (ok: boolean) => void;
}

export const dialogs = $state<{ current: Pending | null; queue: Pending[] }>({ current: null, queue: [] });

export function confirmDialog(message: string, opts: ConfirmOpts = {}): Promise<boolean> {
  return new Promise((resolve) => {
    const p: Pending = {
      message,
      resolve,
      title: opts.title ?? 'Confirmer',
      confirm: opts.confirm ?? 'Confirmer',
      cancel: opts.cancel ?? 'Annuler',
      danger: opts.danger ?? false,
    };
    if (dialogs.current) dialogs.queue.push(p);
    else dialogs.current = p;
  });
}

export function settle(ok: boolean) {
  const p = dialogs.current;
  if (!p) return;
  p.resolve(ok);
  dialogs.current = dialogs.queue.shift() ?? null;
}
