<script lang="ts">
  // Panneau qui monte du bas au téléphone (poignée, zone sûre) ; au bureau, tiroir à droite.
  // <dialog> natif : focus piégé, Échap ferme, clic sur le fond ferme.
  import type { Snippet } from 'svelte';
  import X from '@lucide/svelte/icons/x';
  import IconButton from './IconButton.svelte';

  let {
    open = $bindable(false),
    title,
    onclose,
    children,
  }: { open?: boolean; title: string; onclose?: () => void; children: Snippet } = $props();

  let el: HTMLDialogElement | undefined = $state();
  const titleId = `sh-${Math.random().toString(36).slice(2, 9)}`;

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

<dialog bind:this={el} class="sheet" aria-labelledby={titleId} onclose={closed} onclick={(e) => e.target === el && closed()}>
  {#if open}
    <div class="grip" aria-hidden="true"></div>
    <header>
      <h2 id={titleId}>{title}</h2>
      <IconButton icon={X} label="Fermer" size="lg" onclick={closed} />
    </header>
    <div class="body">{@render children()}</div>
  {/if}
</dialog>

<style>
  .sheet {
    margin: 0 0 0 auto;
    height: 100dvh;
    max-height: none;
    width: min(420px, 100vw);
    max-width: 100vw;
    padding: 0;
    border: 0;
    border-left: 1px solid var(--line-strong);
    background: var(--panel);
    color: var(--text);
    box-shadow: var(--shadow);
    flex-direction: column;
  }
  .sheet[open] { display: flex; }
  .sheet::backdrop { background: var(--backdrop); }
  .grip { display: none; }
  header { display: flex; align-items: center; gap: var(--s2); min-height: 52px; padding: var(--s1) var(--s2) var(--s1) var(--s4); border-bottom: 1px solid var(--line); flex: none; }
  h2 { flex: 1; margin: 0; font-size: var(--fs-lg); font-weight: 600; }
  .body { flex: 1; min-height: 0; overflow: auto; padding-bottom: var(--safe-bottom); }
  @media (max-width: 767px) {
    .sheet {
      margin: auto 0 0;
      width: 100vw;
      height: auto;
      max-height: 88dvh;
      border-left: 0;
      border-top: 1px solid var(--line-strong);
      border-radius: 12px 12px 0 0;
      animation: up 0.18s ease-out;
    }
    .grip { display: block; width: 36px; height: 4px; border-radius: 2px; background: var(--line-strong); margin: 8px auto 0; flex: none; }
  }
  @keyframes up { from { transform: translateY(24px); opacity: 0.6; } }
</style>
