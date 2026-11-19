<script lang="ts">
  // En-tête commun des pages : 48 px (56 au téléphone), titre, méta, actions à droite,
  // rangée d'onglets facultative, puis le contenu qui remplit la hauteur restante.
  // `back` : lien de retour montré au téléphone seulement (sous-page d'une liste).
  import type { Snippet } from 'svelte';
  import ArrowLeft from '@lucide/svelte/icons/arrow-left';
  import IconButton from './IconButton.svelte';

  let {
    title,
    back,
    backLabel = 'Retour',
    meta,
    actions,
    tabs,
    children,
  }: {
    title: string;
    back?: string;
    backLabel?: string;
    meta?: Snippet;
    actions?: Snippet;
    tabs?: Snippet;
    children: Snippet;
  } = $props();
</script>

<div class="page">
  <header class="vh">
    {#if back}<span class="back"><IconButton icon={ArrowLeft} label={backLabel} size="lg" href={back} /></span>{/if}
    <h1>{title}</h1>
    {#if meta}<div class="meta">{@render meta()}</div>{/if}
    {#if actions}<div class="acts">{@render actions()}</div>{/if}
  </header>
  {#if tabs}<div class="tabs-row">{@render tabs()}</div>{/if}
  <div class="body">{@render children()}</div>
</div>

<style>
  .page { flex: 1; display: flex; flex-direction: column; min-width: 0; min-height: 0; }
  .vh {
    display: flex;
    align-items: center;
    gap: var(--s3);
    min-height: 48px;
    padding: var(--s2) var(--s5);
    background: var(--panel);
    border-bottom: 1px solid var(--line);
    flex: none;
    flex-wrap: wrap;
  }
  h1 { margin: 0; font-size: var(--fs-lg); font-weight: 600; line-height: 1.3; }
  .back { display: none; }
  .meta { color: var(--muted); font-size: var(--fs-sm); display: flex; align-items: center; gap: var(--s3); min-width: 0; }
  .acts { margin-left: auto; display: flex; align-items: center; gap: var(--s2); flex-wrap: wrap; }
  .tabs-row { display: flex; align-items: flex-end; gap: var(--s4); padding: 0 var(--s3); flex: none; border-bottom: 1px solid var(--line); min-width: 0; }
  .tabs-row :global(.tabs-ui) { border-bottom: 0; min-width: 0; }
  .body { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; }
  @media (max-width: 767px) {
    .vh { padding: var(--s2) calc(var(--s4) + var(--safe-right)) var(--s2) calc(var(--s4) + var(--safe-left)); min-height: 56px; }
    .back { display: contents; }
    .back + h1 { margin-left: calc(-1 * var(--s2)); }
    .vh:has(.back) { padding-left: calc(var(--s1) + var(--safe-left)); }
    .tabs-row { flex-wrap: wrap; gap: 0; padding: 0 calc(var(--s1) + var(--safe-right)) 0 calc(var(--s1) + var(--safe-left)); }
  }
</style>
