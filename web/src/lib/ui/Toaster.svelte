<script lang="ts">
  // Conteneur des toasts, à monter une fois dans la coquille.
  import Check from '@lucide/svelte/icons/check';
  import Info from '@lucide/svelte/icons/info';
  import CircleAlert from '@lucide/svelte/icons/circle-alert';
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
  import X from '@lucide/svelte/icons/x';
  import Icon from './Icon.svelte';
  import { toasts, dismiss } from './toast.svelte';

  const ICON = { info: Info, ok: Check, warn: TriangleAlert, error: CircleAlert };
</script>

<div class="toaster" aria-live="polite" aria-relevant="additions">
  {#each toasts as t (t.id)}
    <div class="toast {t.kind}" role={t.kind === 'error' ? 'alert' : 'status'}>
      <Icon icon={ICON[t.kind]} size={18} class="ic" />
      <span class="m">{t.message}</span>
      <button class="x" aria-label="Fermer" onclick={() => dismiss(t.id)}><Icon icon={X} size={16} /></button>
    </div>
  {/each}
</div>

<style>
  .toaster {
    position: fixed;
    z-index: 90;
    right: calc(var(--s4) + var(--safe-right));
    bottom: calc(var(--status-h) + var(--s4));
    display: flex;
    flex-direction: column;
    gap: var(--s2);
    width: min(380px, calc(100vw - 32px));
    pointer-events: none;
  }
  .toast {
    pointer-events: auto;
    display: flex;
    align-items: flex-start;
    gap: var(--s3);
    padding: var(--s3) var(--s2) var(--s3) var(--s4);
    background: var(--panel);
    border: 1px solid var(--line-strong);
    border-left: 2px solid var(--accent);
    border-radius: var(--r);
    box-shadow: var(--shadow);
    font-size: var(--fs-sm);
  }
  .toast :global(.ic) { margin-top: 1px; color: var(--accent); }
  .warn { border-left-color: var(--warn); }
  .warn :global(.ic) { color: var(--warn); }
  .error { border-left-color: var(--danger); }
  .error :global(.ic) { color: var(--danger); }
  .info { border-left-color: var(--line-strong); }
  .info :global(.ic) { color: var(--muted); }
  .m { flex: 1; min-width: 0; padding-top: 1px; }
  .x { display: grid; place-items: center; width: 24px; height: 24px; min-height: 0; padding: 0; border: 0; background: none; color: var(--faint); border-radius: var(--r); filter: none; }
  .x:hover { color: var(--text); background: var(--raised); }
  @media (max-width: 767px) {
    .toaster { left: 16px; right: 16px; width: auto; bottom: calc(var(--tabbar-h) + var(--safe-bottom) + var(--s3)); }
  }
</style>
