<script lang="ts">
  // Bloc d'attente (chargement) : `lines` lignes de texte, ou un bloc de largeur/hauteur données.
  let { lines = 1, width = '100%', height }: { lines?: number; width?: string; height?: string } = $props();
</script>

<div class="sk" aria-hidden="true">
  {#each Array(lines) as _, i (i)}
    <span style:width={lines > 1 && i === lines - 1 ? '60%' : width} style:height={height ?? '0.9em'}></span>
  {/each}
</div>
<span class="sr-only">Chargement…</span>

<style>
  .sk { display: flex; flex-direction: column; gap: var(--s2); }
  span { display: block; border-radius: var(--r-sm); background: var(--raised); animation: pulse 1.4s ease-in-out infinite; }
  @keyframes pulse { 50% { opacity: 0.55; } }
</style>
