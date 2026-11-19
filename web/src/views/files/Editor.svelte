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

{#if failed}<p class="error small">Éditeur indisponible : {failed}</p>{/if}
<div class="cm" bind:this={host}></div>

<style>
  .cm { flex: 1; height: 100%; min-height: 300px; border: 1px solid var(--line); border-radius: var(--radius); overflow: hidden; }
</style>
