<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import type { Editor } from '../../lib/editor';

  let {
    path,
    text,
    readonly = false,
    onsave,
    ondirty,
  }: { path: string; text: string; readonly?: boolean; onsave: () => void; ondirty: () => void } = $props();

  let host: HTMLDivElement;
  let ed: Editor | null = null;
  let failed = $state('');

  /** Texte courant de l'éditeur. */
  export function current(): string {
    return ed?.text() ?? text;
  }

  /** Remplace le texte (rechargement). */
  export function replace(t: string) {
    ed?.set(t);
  }

  onMount(async () => {
    try {
      const { createEditor } = await import('../../lib/editor');
      ed = await createEditor(host, { path, text, readonly, onChange: () => ondirty(), onSave: () => onsave() });
    } catch (e) {
      failed = String(e);
    }
  });
  onDestroy(() => ed?.destroy());
</script>

{#if failed}<p class="failed" role="alert">Éditeur indisponible : {failed}</p>{/if}
<div class="cm" bind:this={host}></div>

<style>
  /* Éditeur bord à bord : le cadre est celui du panneau. */
  .cm { flex: 1; height: 100%; min-height: 240px; overflow: hidden; background: var(--bg); }
  .failed { margin: 0; padding: var(--s2) var(--s4); color: var(--danger); background: var(--danger-soft); font-size: var(--fs-sm); border-bottom: 1px solid var(--line); }
</style>
