<script lang="ts">
  // Libellé + aide + erreur autour d'un contrôle (utilisé par Input, Textarea, Select).
  import type { Snippet } from 'svelte';
  let {
    label,
    hint,
    error,
    id,
    children,
  }: { label?: string; hint?: string; error?: string; id: string; children: Snippet } = $props();
</script>

<div class="f">
  {#if label}<label class="l" for={id}>{label}</label>{/if}
  {@render children()}
  {#if error}<small class="err" id="{id}-err">{error}</small>{:else if hint}<small class="hint" id="{id}-hint">{hint}</small>{/if}
</div>

<style>
  .f { display: flex; flex-direction: column; gap: var(--s2); min-width: 0; font-size: var(--fs-sm); }
  .l { display: block; color: var(--muted); font-size: var(--fs-sm); }
  .hint { color: var(--faint); font-size: var(--fs-xs); }
  .err { color: var(--danger); font-size: var(--fs-xs); }
</style>
