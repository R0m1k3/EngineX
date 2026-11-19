<script lang="ts">
  // Fenêtre modale (<dialog> natif : focus piégé, Échap ferme). `open` liable ;
  // `actions` = boutons du pied (le principal en dernier, à droite).
  import type { Snippet } from 'svelte';
  import X from '@lucide/svelte/icons/x';
  import IconButton from './IconButton.svelte';

  let {
    open = $bindable(false),
    title,
    width = '480px',
    onclose,
    children,
    actions,
  }: {
    open?: boolean;
    title: string;
    width?: string;
    onclose?: () => void;
    children: Snippet;
    actions?: Snippet;
  } = $props();

  let el: HTMLDialogElement | undefined = $state();
  const titleId = `dlg-${Math.random().toString(36).slice(2, 9)}`;

  $effect(() => {
    if (!el) return;
    if (open && !el.open) el.showModal();
    else if (!open && el.open) el.close();
  });

  function closed() {
    if (open) {
      open = false;
      onclose?.();
    }
  }
</script>

<dialog
  bind:this={el}
  class="dlg"
  style:width="min({width}, calc(100vw - 32px))"
  aria-labelledby={titleId}
  onclose={closed}
  onclick={(e) => e.target === el && closed()}
>
  {#if open}
    <header>
      <h2 id={titleId}>{title}</h2>
      <IconButton icon={X} label="Fermer" size="sm" onclick={closed} />
    </header>
    <div class="body">{@render children()}</div>
    {#if actions}<footer>{@render actions()}</footer>{/if}
  {/if}
</dialog>

<style>
  .dlg {
    padding: 0;
    border: 1px solid var(--line-strong);
    border-radius: var(--r);
    background: var(--bg);
    color: var(--text);
    box-shadow: var(--shadow);
    max-height: calc(100dvh - 48px);
  }
  .dlg::backdrop { background: var(--backdrop); }
  header { display: flex; align-items: center; gap: var(--s3); padding: var(--s3) var(--s3) var(--s3) var(--s5); border-bottom: 1px solid var(--line); }
  h2 { flex: 1; margin: 0; font-size: var(--fs-md); font-weight: 600; }
  .body { padding: var(--s5); display: flex; flex-direction: column; gap: var(--s4); overflow: auto; }
  footer { display: flex; justify-content: flex-end; flex-wrap: wrap; gap: var(--s2); padding: var(--s3) var(--s5); border-top: 1px solid var(--line); background: var(--panel); }
  @media (max-width: 767px) {
    footer :global(.btn) { height: var(--touch); flex: 1; }
  }
</style>
