<script lang="ts">
  // Section de formulaire à filets : titre et explication à gauche (220 px), contenu à droite (≤ 800).
  // Partagée par l'éditeur de preset, les Paramètres et le Système. Au téléphone, le titre passe au-dessus.
  import type { Snippet } from 'svelte';

  let {
    title,
    text,
    aside,
    id,
    children,
  }: { title: string; text?: string; aside?: Snippet; id?: string; children: Snippet } = $props();
</script>

<section class="sec" {id} aria-label={title}>
  <div class="head">
    <h3>{title}</h3>
    {#if text}<p>{text}</p>{/if}
    {#if aside}{@render aside()}{/if}
  </div>
  <div class="body">{@render children()}</div>
</section>

<style>
  .sec {
    display: grid;
    grid-template-columns: 220px minmax(0, 800px);
    gap: var(--s8);
    padding: var(--s6);
    border-bottom: 1px solid var(--line);
  }
  .head { min-width: 0; }
  h3 { margin: 0 0 var(--s2); font-size: var(--fs-md); font-weight: 600; line-height: 1.4; }
  p { margin: 0; color: var(--muted); font-size: var(--fs-sm); line-height: 1.5; }
  .body { display: flex; flex-direction: column; gap: var(--s4); min-width: 0; }
  /* Un interrupteur posé seul garde sa largeur (sinon son libellé se centre). */
  .body > :global(.sw) { align-self: flex-start; }
  @media (max-width: 1023px) {
    .sec { grid-template-columns: minmax(0, 1fr); gap: var(--s3); padding: var(--s5) var(--s4); }
  }
</style>
