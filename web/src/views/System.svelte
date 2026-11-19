<script lang="ts">
  import ProgressBar from '../lib/ui/ProgressBar.svelte';
  import { onDestroy } from 'svelte';
  import { system, message, diskUsage, type SystemInfo, type DiskUsage } from '../lib/api';

  let info = $state<SystemInfo | null>(null);
  let error = $state('');
  let disk = $state<DiskUsage | null>(null);

  async function load(refresh = false) {
    try {
      info = await system(refresh);
      diskUsage().then((d) => (disk = d)).catch(() => {});
      error = '';
    } catch (e) {
      error = message(e);
    }
  }

  function why(err: string, what: string) {
    return err === 'not_found' ? `${what} introuvable` : `${what} : ${err}`;
  }

  const gib = (mib: number) => (mib / 1024).toFixed(1);

  load();
  const timer = setInterval(() => load(), 10_000);
  onDestroy(() => clearInterval(timer));
</script>

<header class="row">
  <h2>Système</h2>
  <button onclick={() => load(true)}>Relire</button>
</header>

{#if error}<p class="error">{error}</p>{/if}
{#if disk?.alert}
  <p class="error">Disque de /data rempli à {disk.used_pct} % (alerte à {disk.alert_pct} %) : libérez de la place depuis Fichiers ou Sessions.</p>
{/if}

{#if info}
  <section class="grid">
    {#if info.gpus_error}
      <article class="card"><h3>GPU</h3><p class="error">{why(info.gpus_error, 'nvidia-smi')}</p></article>
    {/if}
    {#each info.gpus as gpu}
      <article class="card">
        <h3>GPU {gpu.index} · {gpu.name}</h3>
        <div class="pb"><ProgressBar value={gpu.used_mib} max={gpu.total_mib} label="VRAM utilisée" /></div>
        <p>{gib(gpu.used_mib)} / {gib(gpu.total_mib)} Gio · {gpu.temp_c} °C · pilote {gpu.driver}</p>
      </article>
    {/each}

    <article class="card">
      <h3>llama-server</h3>
      {#if info.llama?.error}
        <p class="error">{why(info.llama.error, 'llama-server')}</p>
      {/if}
      <p>Version : {info.llama?.version ?? 'inconnue'}</p>
      <p class="muted"><code>{info.llama?.bin}</code></p>
      {#if info.llama && info.llama.devices.length > 0}
        <ul>
          {#each info.llama.devices as d}
            <li><strong>{d.id}</strong> {d.name} · {gib(d.free_mib)} Gio libres sur {gib(d.total_mib)}</li>
          {/each}
        </ul>
      {:else}
        <p class="muted">Aucun périphérique CUDA vu.</p>
      {/if}
    </article>
  </section>
  <p class="muted small">EngineX {info.version} · démarré il y a {Math.floor(info.uptime_s / 60)} min</p>
{/if}

<style>
  .pb { margin-bottom: var(--s2); }
</style>
