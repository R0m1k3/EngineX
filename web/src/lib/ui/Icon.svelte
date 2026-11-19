<script lang="ts">
  // Icône lucide au trait 1,75 (direction : 18 px, 16 en ligne dense, 22 en navigation).
  import type { Component } from 'svelte';

  let {
    icon: Glyph,
    size = 18,
    label,
    class: cls = '',
  }: { icon: Component<any>; size?: number; label?: string; class?: string } = $props();
</script>

<Glyph
  {size}
  strokeWidth={1.75}
  class="ui-icon {cls}"
  aria-hidden={label ? undefined : 'true'}
  aria-label={label}
  role={label ? 'img' : undefined}
/>

<style>
  :global(.ui-icon) { flex: none; display: block; }
</style>
