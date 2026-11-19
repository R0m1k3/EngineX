<script lang="ts">
  // Enveloppe pour poser une infobulle sur n'importe quel contenu (sinon : use:tooltip).
  import type { Snippet } from 'svelte';
  import { tooltip, type TooltipSide } from './tooltip';

  let { text, side = 'top', kbd, children }: { text: string; side?: TooltipSide; kbd?: string; children: Snippet } = $props();
</script>

<span class="tip-host" use:tooltip={{ text, side, kbd }}>{@render children()}</span>

<style>
  .tip-host { display: inline-flex; }
</style>
