<script lang="ts">
  // Vue bord à bord (tableau de bord, tâches, crons, agents, notifications) : en-tête de 48 px
  // (titre, méta, actions) puis le contenu. Annule la marge de `.view` de la coquille.
  import type { Snippet } from 'svelte';

  let {
    title,
    meta,
    actions,
    tabs,
    children,
  }: { title: string; meta?: Snippet; actions?: Snippet; tabs?: Snippet; children: Snippet } = $props();
</script>

<div class="page">
  <header class="vh">
    <h1>{title}</h1>
    {#if meta}<div class="meta">{@render meta()}</div>{/if}
    {#if actions}<div class="acts">{@render actions()}</div>{/if}
  </header>
  {#if tabs}<div class="tabs-row">{@render tabs()}</div>{/if}
  <div class="body">{@render children()}</div>
</div>

<style>
  .page {
    margin: calc(-1 * var(--s6));
    min-height: calc(100% + 2 * var(--s6));
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .vh {
    display: flex;
    align-items: center;
    gap: var(--s3);
    min-height: 48px;
    padding: var(--s2) var(--s5);
    border-bottom: 1px solid var(--line);
    flex: none;
    flex-wrap: wrap;
  }
  h1 { margin: 0; font-size: var(--fs-lg); font-weight: 600; line-height: 1.3; }
  .meta { color: var(--muted); font-size: var(--fs-sm); display: flex; align-items: center; gap: var(--s3); min-width: 0; }
  .acts { margin-left: auto; display: flex; align-items: center; gap: var(--s2); flex-wrap: wrap; }
  .tabs-row { padding: 0 var(--s3); flex: none; }
  .tabs-row :global(.tabs-ui) { border-bottom: 0; }
  .tabs-row { border-bottom: 1px solid var(--line); }
  .body { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  @media (max-width: 767px) {
    .page {
      margin: calc(-1 * var(--s4)) calc(-1 * (var(--s4) + var(--safe-right))) calc(-1 * var(--s4)) calc(-1 * (var(--s4) + var(--safe-left)));
      min-height: calc(100% + 2 * var(--s4));
    }
    .vh { padding: var(--s2) calc(var(--s4) + var(--safe-right)) var(--s2) calc(var(--s4) + var(--safe-left)); min-height: 56px; }
    .tabs-row { padding: 0 calc(var(--s1) + var(--safe-right)) 0 calc(var(--s1) + var(--safe-left)); }
  }
</style>
