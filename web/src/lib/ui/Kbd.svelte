<script lang="ts">
  import type { Snippet } from 'svelte';
  let { children }: { children: Snippet } = $props();
</script>

<kbd class="kbd">{@render children()}</kbd>

<style>
  .kbd {
    display: inline-block;
    font-family: var(--mono);
    font-size: 13px;
    line-height: 18px;
    padding: 0 5px;
    border: 1px solid var(--line-strong);
    border-bottom-width: 2px;
    border-radius: var(--r-sm);
    color: var(--muted);
    background: var(--bg);
    white-space: nowrap;
  }
</style>
