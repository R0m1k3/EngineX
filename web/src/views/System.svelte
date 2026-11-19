<script lang="ts">
  import { octets } from './models/fmt';
  import { onDestroy } from 'svelte';
  import RefreshCw from '@lucide/svelte/icons/refresh-cw';
  import MonitorCog from '@lucide/svelte/icons/monitor-cog';
  import Server from '@lucide/svelte/icons/server';
  import HardDrive from '@lucide/svelte/icons/hard-drive';
  import ScrollText from '@lucide/svelte/icons/scroll-text';
  import Button from '../lib/ui/Button.svelte';
  import Card from '../lib/ui/Card.svelte';
  import Gauge from '../lib/ui/Gauge.svelte';
  import ProgressBar from '../lib/ui/ProgressBar.svelte';
  import { gb, dec1 } from '../lib/ui/format';
  import type { GaugeRow } from '../lib/ui/types';
  import Page from './ops/Page.svelte';
  import Note from './settings/Note.svelte';
  import { system, engineLogs, message, diskUsage, type SystemInfo, type DiskUsage } from '../lib/api';

  let info = $state<SystemInfo | null>(null);
  let error = $state('');
  let disk = $state<DiskUsage | null>(null);
  let busy = $state(false);

  async function load(refresh = false) {
    try {
      info = await system(refresh);
      diskUsage().then((d) => (disk = d)).catch(() => {});
      error = '';
    } catch (e) {
      error = message(e);
    }
  }

  async function reread() {
    busy = true;
    await load(true);
    busy = false;
  }

  // Journal de llama-server, lu à la demande (200 dernières lignes).
  let logs = $state<string[] | null>(null);
  let logsBusy = $state(false);
  async function readLogs() {
    logsBusy = true;
    try {
      logs = (await engineLogs(200)).lines;
    } catch (e) {
      error = message(e);
    } finally {
      logsBusy = false;
    }
  }

  function why(err: string, what: string) {
    return err === 'not_found' ? `${what} introuvable` : `${what} : ${err}`;
  }

  const rows = $derived<GaugeRow[]>(
    (info?.gpus ?? []).map((g) => ({ label: `GPU${g.index}`, sub: g.name, total_mib: g.total_mib, used_mib: g.used_mib })),
  );
  const uptime = (s: number) => {
    const m = Math.floor(s / 60);
    return m < 60 ? `${m} min` : m < 1440 ? `${Math.floor(m / 60)} h ${m % 60} min` : `${Math.floor(m / 1440)} j ${Math.floor((m % 1440) / 60)} h`;
  };

  load();
  const timer = setInterval(() => load(), 10_000);
  onDestroy(() => clearInterval(timer));
</script>

<Page title="Système">
  {#snippet meta()}{#if info}<span>EngineX {info.version}, démarré il y a {uptime(info.uptime_s)}</span>{/if}{/snippet}
  {#snippet actions()}<Button icon={RefreshCw} onclick={reread} {busy}>Relire</Button>{/snippet}

  {#if error || disk?.alert}
    <div class="notes">
      {#if error}<Note tone="danger">{error}</Note>{/if}
      {#if disk?.alert}
        <Note tone="danger">Disque de /data rempli à {dec1(disk.used_pct)} % (alerte à {disk.alert_pct} %) : libérez de la place depuis Fichiers ou Sessions.</Note>
      {/if}
    </div>
  {/if}

  {#if info}
    <div class="ui-tiles sys">
      <Card title="Cartes graphiques" icon={MonitorCog} span={2}>
        {#if info.gpus_error}
          <Note tone="danger">{why(info.gpus_error, 'nvidia-smi')}</Note>
        {:else if rows.length}
          <Gauge {rows} size="md" />
          <table class="tbl-gpu">
            <thead><tr><th>Carte</th><th class="r">Utilisée</th><th class="r">Libre</th><th class="r">Température</th><th>Pilote</th></tr></thead>
            <tbody>
              {#each info.gpus as g (g.index)}
                <tr>
                  <td><span class="mono">GPU{g.index}</span> {g.name}</td>
                  <td class="r num" data-l="Utilisée">{gb(g.used_mib)} / {gb(g.total_mib)} Go</td>
                  <td class="r num" data-l="Libre">{gb(Math.max(0, g.total_mib - g.used_mib))} Go</td>
                  <td class="r num" data-l="Température" class:hot={g.temp_c >= 80}>{g.temp_c} °C</td>
                  <td class="mono" data-l="Pilote">{g.driver}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        {:else}
          <p class="muted">Aucune carte NVIDIA vue par nvidia-smi.</p>
        {/if}
      </Card>

      <Card title="Disque /data" icon={HardDrive}>
        {#if disk}
          {@const used = disk.total_bytes - disk.free_bytes}
          <div class="big num">{dec1(disk.used_pct)} %</div>
          <ProgressBar value={disk.used_pct} label="Disque /data utilisé" tone={disk.alert ? 'danger' : disk.used_pct >= disk.alert_pct - 10 ? 'warn' : 'accent'} />
          <dl class="kv">
            <dt>Utilisé</dt><dd class="num">{octets(used)} sur {octets(disk.total_bytes)}</dd>
            <dt>Espace de travail</dt><dd class="num">{octets(disk.workspace_bytes)}</dd>
            <dt>Sessions</dt><dd class="num">{octets(disk.sessions_bytes)}</dd>
            <dt>Alerte</dt><dd class="num">à {disk.alert_pct} %</dd>
          </dl>
        {:else}
          <p class="muted">Lecture du disque…</p>
        {/if}
      </Card>

      <Card title="llama-server" icon={Server} span={3}>
        {#if info.llama?.error}
          <Note tone="danger">{why(info.llama.error, 'llama-server')}</Note>
        {/if}
        <dl class="kv">
          <dt>Version</dt><dd class="mono">{info.llama?.version ?? 'inconnue'}</dd>
          <dt>Binaire</dt><dd class="mono path">{info.llama?.bin ?? 'inconnu'}</dd>
        </dl>
        {#if info.llama && info.llama.devices.length > 0}
          <table class="tbl-gpu">
            <thead><tr><th>Périphérique</th><th>Nom</th><th class="r">Libre</th><th class="r">Total</th></tr></thead>
            <tbody>
              {#each info.llama.devices as d (d.id)}
                <tr>
                  <td class="mono">{d.id}</td>
                  <td data-l="Nom">{d.name}</td>
                  <td class="r num" data-l="Libre">{gb(d.free_mib)} Go</td>
                  <td class="r num" data-l="Total">{gb(d.total_mib)} Go</td>
                </tr>
              {/each}
            </tbody>
          </table>
        {:else}
          <p class="muted">Aucun périphérique CUDA vu.</p>
        {/if}
      </Card>

      <Card title="Journal de llama-server" icon={ScrollText} span={3}>
        {#snippet actions()}
          <Button size="sm" icon={RefreshCw} onclick={readLogs} busy={logsBusy}>{logs ? 'Relire' : 'Afficher'}</Button>
        {/snippet}
        {#if logs}
          <pre class="log" aria-label="Journal de llama-server">{logs.join('\n') || 'Journal vide.'}</pre>
        {:else}
          <p class="muted">Les 200 dernières lignes écrites par llama-server, lues à la demande.</p>
        {/if}
      </Card>
    </div>
  {/if}
</Page>

<style>
  .notes { display: flex; flex-direction: column; gap: var(--s2); padding: var(--s4) var(--s5); border-bottom: 1px solid var(--line); }
  .sys { border-width: 0 0 1px; }
  .muted { margin: 0; color: var(--muted); font-size: var(--fs-sm); }
  .big { font-size: var(--fs-2xl); font-weight: 600; line-height: 1.1; }
  .kv { display: grid; grid-template-columns: max-content minmax(0, 1fr); gap: var(--s2) var(--s5); margin: 0; font-size: var(--fs-sm); }
  .kv dt { color: var(--muted); }
  .kv dd { margin: 0; }
  .path { overflow-wrap: anywhere; }
  .tbl-gpu { width: 100%; border-collapse: collapse; font-size: var(--fs-sm); }
  .tbl-gpu th { text-align: left; font-weight: 400; color: var(--muted); padding: var(--s2) var(--s3) var(--s2) 0; border-bottom: 1px solid var(--line); }
  .tbl-gpu td { padding: var(--s2) var(--s3) var(--s2) 0; border-bottom: 1px solid var(--line); vertical-align: middle; }
  .tbl-gpu tbody tr:hover { background: none; }
  .tbl-gpu tr:last-child td { border-bottom: 0; }
  .r { text-align: right !important; }
  .hot { color: var(--warn); }
  .log {
    margin: 0;
    max-height: 480px;
    overflow: auto;
    padding: var(--s3) var(--s4);
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: var(--r);
    font-family: var(--mono);
    font-size: var(--fs-code);
    line-height: 1.6;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  @media (max-width: 767px) {
    .notes { padding: var(--s3) var(--s4); }
    .tbl-gpu thead { display: none; }
    .tbl-gpu, .tbl-gpu tbody, .tbl-gpu tr, .tbl-gpu td { display: block; }
    .tbl-gpu tr { padding: var(--s2) 0; border-bottom: 1px solid var(--line); }
    .tbl-gpu td { border: 0; padding: 2px 0; text-align: left !important; }
    .tbl-gpu td[data-l] { display: flex; justify-content: space-between; gap: var(--s3); }
    .tbl-gpu td[data-l]::before { content: attr(data-l); color: var(--muted); }
  }
</style>
