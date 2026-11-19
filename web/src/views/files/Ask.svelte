<script lang="ts">
  // Saisie d'un nom dans une modale (remplace window.prompt) : `await ask.ask({ title, label })`
  // rend le texte saisi, ou null si la fenêtre est fermée sans valider.
  import { Button, Dialog, Input } from '../../lib/ui';

  interface AskOpts {
    title: string;
    label: string;
    value?: string;
    hint?: string;
    confirm?: string;
    mono?: boolean;
  }

  let open = $state(false);
  let opts = $state<AskOpts>({ title: '', label: '' });
  let value = $state('');
  let ref = $state<HTMLInputElement>();
  let resolve: ((v: string | null) => void) | null = null;
  const formId = `ask-${Math.random().toString(36).slice(2, 9)}`;

  export function ask(o: AskOpts): Promise<string | null> {
    resolve?.(null);
    opts = o;
    value = o.value ?? '';
    open = true;
    setTimeout(() => {
      ref?.focus();
      // Nom de fichier proposé : sélectionner le nom sans l'extension.
      const dot = value.lastIndexOf('.');
      ref?.setSelectionRange(0, dot > 0 ? dot : value.length);
    }, 30);
    return new Promise((r) => (resolve = r));
  }

  function done(v: string | null) {
    const r = resolve;
    resolve = null;
    open = false;
    r?.(v);
  }
</script>

<Dialog bind:open title={opts.title} width="440px" onclose={() => done(null)}>
  <form id={formId} onsubmit={(e) => { e.preventDefault(); done(value); }}>
    <Input bind:value bind:ref label={opts.label} hint={opts.hint} class={opts.mono ? 'ask-mono' : ''} autocomplete="off" spellcheck="false" />
  </form>
  {#snippet actions()}
    <Button onclick={() => done(null)}>Annuler</Button>
    <Button variant="primary" type="submit" form={formId}>{opts.confirm ?? 'Valider'}</Button>
  {/snippet}
</Dialog>

<style>
  form :global(.ask-mono input) { font-family: var(--mono); }
</style>
