<script lang="ts">
  // Point d'état 8 px : ok = prêt/actif, warn = attend ton accord, off = au repos, err = erreur.
  import type { DotKind } from './types';
  let { kind = 'ok', label }: { kind?: DotKind; label?: string } = $props();
</script>

<span class="dot {kind}" title={label} role={label ? 'img' : undefined} aria-label={label} aria-hidden={label ? undefined : 'true'}></span>

<style>
  .dot { display: inline-block; width: 8px; height: 8px; border-radius: 50%; flex: none; background: var(--accent-fill); }
  .warn { background: var(--warn); }
  .off { background: var(--faint); }
  .err { background: var(--danger); }
</style>
