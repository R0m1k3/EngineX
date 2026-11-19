<script lang="ts">
  // Barre d'état (bas, bureau ; haut et réduite au téléphone) : modèle chargé et son état,
  // débit du dernier tour, file, VRAM par carte, thème, notifications.
  import { onDestroy } from 'svelte';
  import Bell from '@lucide/svelte/icons/bell';
  import Sun from '@lucide/svelte/icons/sun';
  import Moon from '@lucide/svelte/icons/moon';
  import Monitor from '@lucide/svelte/icons/monitor';
  import StatusDot from '../../lib/ui/StatusDot.svelte';
  import IconButton from '../../lib/ui/IconButton.svelte';
  import Gauge from '../../lib/ui/Gauge.svelte';
  import { tooltip } from '../../lib/ui/tooltip';
  import { gb } from '../../lib/ui/format';
  import { ui, cycleTheme, THEME_LABEL } from '../../lib/ui/theme.svelte';
  import { status, watchStatus, engineSummary, queueSummary, gpuRows, rate } from './status.svelte';

  let { unread }: { unread: number } = $props();

  onDestroy(watchStatus());

  const sum = $derived(engineSummary(status.eng, status.presets));
  const rows = $derived(gpuRows(status.eng));
  const used = $derived(rows.reduce((a, r) => a + (r.used_mib ?? 0), 0));
  const total = $derived(rows.reduce((a, r) => a + r.total_mib, 0));
  const tps = $derived(rate(status.eng));
  const themeIcon = $derived(ui.theme === 'system' ? Monitor : ui.theme === 'dark' ? Moon : Sun);
  const next = $derived(ui.theme === 'system' ? 'light' : ui.theme === 'light' ? 'dark' : 'system');
</script>

<footer class="statusbar" aria-label="État du moteur">
  <a class="it model" href="#models" use:tooltip={'Modèles et presets'}>
    <StatusDot kind={status.error ? 'err' : sum.dot} />
    <span class="ell name">{status.error ? 'Serveur injoignable' : sum.model}</span>
    {#if !status.error}<span class="muted st">{sum.state}</span>{/if}
  </a>
  {#if tps || status.queue || rows.length}<span class="sep wide"></span>{/if}
  {#if tps}<span class="it num wide" use:tooltip={'Débit de la dernière réponse'}>{tps}</span>{/if}
  {#if status.queue}<a class="it wide" href="#dashboard">{queueSummary(status.queue)}</a>{/if}
  {#if rows.length}
    <a class="it num wide" href="#system" use:tooltip={rows.map((r) => `${r.label} ${gb(r.used_mib ?? 0)} / ${gb(r.total_mib)} Go`).join(', ')}>
      <Gauge {rows} size="mini" />VRAM {gb(used)} / {gb(total)} Go
    </a>
  {/if}
  <span class="right">
    <IconButton
      icon={themeIcon}
      label="Thème : {THEME_LABEL[ui.theme]} (passer à {THEME_LABEL[next].toLowerCase()})"
      size="sm"
      class="phone-lg"
      onclick={cycleTheme}
    />
    <IconButton icon={Bell} label="Notifications" size="sm" count={unread} href="#feed" class="wide" />
  </span>
</footer>

<style>
  .statusbar {
    display: flex;
    align-items: center;
    gap: var(--s5);
    height: var(--status-h);
    padding: 0 var(--s3);
    background: var(--panel);
    border-top: 1px solid var(--line);
    font-size: var(--fs-xs);
    color: var(--muted);
    white-space: nowrap;
    min-width: 0;
  }
  .it { display: inline-flex; align-items: center; gap: var(--s2); color: inherit; text-decoration: none; min-width: 0; }
  a.it:hover { color: var(--text); }
  .model { color: var(--text); }
  .name { max-width: 32ch; }
  .sep { width: 1px; height: 14px; background: var(--line-strong); flex: none; }
  .right { margin-left: auto; display: flex; align-items: center; gap: var(--s1); }
  @media (max-width: 767px) {
    .statusbar {
      height: calc(40px + var(--safe-top));
      padding: var(--safe-top) calc(var(--s2) + var(--safe-right)) 0 calc(var(--s4) + var(--safe-left));
      border-top: 0;
      border-bottom: 1px solid var(--line);
      font-size: var(--fs-sm);
    }
    .wide, .statusbar :global(.wide) { display: none; }
    .statusbar :global(.phone-lg) { width: var(--touch); height: 40px; }
    .model { flex: 1; }
    .name { max-width: none; }
  }
</style>
