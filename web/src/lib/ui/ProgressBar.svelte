<script lang="ts">
  // Barre de progression 6 px (remplace l'ancienne classe globale de barre). `value` sur `max` (100 par défaut).
  let {
    value,
    max = 100,
    label,
    tone = 'accent',
    size = 'md',
  }: { value: number; max?: number; label?: string; tone?: 'accent' | 'warn' | 'danger'; size?: 'sm' | 'md' } = $props();
  const pct = $derived(max > 0 ? Math.max(0, Math.min(100, (value / max) * 100)) : 0);
</script>

<div
  class="pb {tone} {size}"
  role="progressbar"
  aria-label={label}
  aria-valuemin={0}
  aria-valuemax={100}
  aria-valuenow={Math.round(pct)}
>
  <i style:width="{pct}%"></i>
</div>

<style>
  .pb { height: 6px; background: var(--raised); border-radius: 3px; overflow: hidden; }
  .sm { height: 4px; }
  i { display: block; height: 100%; background: var(--accent-fill); transition: width 0.2s; }
  .warn i { background: var(--warn); }
  .danger i { background: var(--danger); }
</style>
