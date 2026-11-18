<script lang="ts">
  import { onDestroy } from 'svelte';
  import { system, message, type SystemInfo } from '../lib/api';

  let info = $state<SystemInfo | null>(null);
  let error = $state('');

  async function load(refresh = false) {
    try {
      info = await system(refresh);
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

{#if info}
  <section class="grid">
    {#if info.gpus_error}
      <article class="card"><h3>GPU</h3><p class="error">{why(info.gpus_error, 'nvidia-smi')}</p></article>
    {/if}
    {#each info.gpus as gpu}
      <article class="card">
        <h3>GPU {gpu.index} · {gpu.name}</h3>
        <div class="bar"><span style:width="{gpu.total_mib ? (gpu.used_mib / gpu.total_mib) * 100 : 0}%"></span></div>
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
