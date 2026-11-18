<script lang="ts">
  import { onDestroy } from 'svelte';
  import {
    downloads, pauseDownload, resumeDownload, deleteDownload, message, size, jobError, type Download,
  } from '../../lib/api';

  let list = $state<Download[]>([]);
  let error = $state('');

  async function load() {
    try {
      list = await downloads();
      error = '';
    } catch (e) {
      error = message(e);
    }
  }

  async function act(fn: (id: number) => Promise<unknown>, id: number) {
    try {
      await fn(id);
      await load();
    } catch (e) {
      error = message(e);
    }
  }

  const label: Record<string, string> = {
    queued: 'en attente', running: 'en cours', paused: 'en pause', done: 'terminé', failed: 'échec', cancelled: 'annulé',
  };

  function eta(d: Download) {
    if (d.state !== 'running' || d.speed_bps <= 0) return '';
    const s = (d.total_bytes - d.received_bytes) / d.speed_bps;
    return s > 3600 ? `${(s / 3600).toFixed(1)} h` : s > 60 ? `${Math.ceil(s / 60)} min` : `${Math.ceil(s)} s`;
  }

  load();
  const timer = setInterval(load, 1000);
  onDestroy(() => clearInterval(timer));
</script>

{#if error}<p class="error">{error}</p>{/if}
{#if list.length === 0}<p class="muted">Aucun téléchargement. Cherchez un modèle dans l’onglet Chercher.</p>{/if}

{#each list as d (d.id)}
  <article class="card">
    <div class="row wrap">
      <div>
        <strong>{d.repo}</strong>
        <span class="muted small"> · {d.files[0]?.name}{d.files.length > 1 ? ` + ${d.files.length - 1}` : ''}</span>
      </div>
      <div class="actions">
        {#if d.state === 'running' || d.state === 'queued'}<button class="secondary" onclick={() => act(pauseDownload, d.id)}>Pause</button>{/if}
        {#if d.state === 'paused' || d.state === 'failed'}<button class="secondary" onclick={() => act(resumeDownload, d.id)}>Reprendre</button>{/if}
        <button class="link" onclick={() => act(deleteDownload, d.id)}>{d.state === 'done' || d.state === 'cancelled' ? 'Retirer' : 'Annuler'}</button>
      </div>
    </div>
    <div class="bar"><span style:width="{d.total_bytes ? (d.received_bytes / d.total_bytes) * 100 : 0}%"></span></div>
    <p class="small">
      <span class="state state-{d.state}">{label[d.state]}</span>
      <span class="muted"> · {size(d.received_bytes)} / {size(d.total_bytes)}</span>
      {#if d.state === 'running'}<span class="muted"> · {size(d.speed_bps)}/s · reste {eta(d)}</span>{/if}
    </p>
    {#if d.error}<p class="error small">{jobError(d.error)}</p>{/if}
    {#if d.state === 'done'}<p class="small"><a href="#models/presets">Preset créé : voir les presets</a></p>{/if}
  </article>
{/each}
