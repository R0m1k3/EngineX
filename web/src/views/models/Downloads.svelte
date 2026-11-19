<script lang="ts">
  import { octets } from './fmt';
  import { onDestroy } from 'svelte';
  import DownloadIcon from '@lucide/svelte/icons/download';
  import Pause from '@lucide/svelte/icons/pause';
  import Play from '@lucide/svelte/icons/play';
  import X from '@lucide/svelte/icons/x';
  import Button from '../../lib/ui/Button.svelte';
  import Badge from '../../lib/ui/Badge.svelte';
  import ProgressBar from '../../lib/ui/ProgressBar.svelte';
  import EmptyState from '../../lib/ui/EmptyState.svelte';
  import Icon from '../../lib/ui/Icon.svelte';
  import type { Tone } from '../../lib/ui/types';
  import Note from '../settings/Note.svelte';
  import {
    downloads, pauseDownload, resumeDownload, deleteDownload, message, jobError, type Download, type DownloadState,
  } from '../../lib/api';

  let list = $state<Download[]>([]);
  let loaded = $state(false);
  let error = $state('');

  async function load() {
    try {
      list = await downloads();
      error = '';
    } catch (e) {
      error = message(e);
    } finally {
      loaded = true;
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
  const tone: Record<DownloadState, Tone> = {
    queued: 'neutral', running: 'ok', paused: 'warn', done: 'ok', failed: 'danger', cancelled: 'neutral',
  };

  function eta(d: Download) {
    if (d.state !== 'running' || d.speed_bps <= 0) return '';
    const s = (d.total_bytes - d.received_bytes) / d.speed_bps;
    return s > 3600 ? `${(s / 3600).toFixed(1).replace('.', ',')} h` : s > 60 ? `${Math.ceil(s / 60)} min` : `${Math.ceil(s)} s`;
  }

  load();
  const timer = setInterval(load, 1000);
  onDestroy(() => clearInterval(timer));
</script>

<div class="dl">
  {#if error}<Note tone="danger">{error}</Note>{/if}
  {#if loaded && list.length === 0}
    <EmptyState icon={DownloadIcon} title="Aucun téléchargement" text="Choisissez une quantification dans la recherche Hugging Face : son avancement s’affiche ici.">
      {#snippet action()}<Button href="#models">Chercher un modèle</Button>{/snippet}
    </EmptyState>
  {/if}

  {#if list.length}
    <ul class="list" aria-label="Téléchargements">
      {#each list as d (d.id)}
        <li>
          <div class="head">
            <Icon icon={DownloadIcon} size={16} class="lead" />
            <div class="names">
              <strong class="ell">{d.repo}</strong>
              <span class="file ell">{d.files[0]?.name}{d.files.length > 1 ? ` + ${d.files.length - 1}` : ''}</span>
            </div>
            <Badge tone={tone[d.state]}>{label[d.state]}</Badge>
            <div class="acts">
              {#if d.state === 'running' || d.state === 'queued'}
                <Button variant="ghost" icon={Pause} label="Pause" onclick={() => act(pauseDownload, d.id)} />
              {/if}
              {#if d.state === 'paused' || d.state === 'failed'}
                <Button variant="ghost" icon={Play} label="Reprendre" onclick={() => act(resumeDownload, d.id)} />
              {/if}
              <Button variant="ghost" icon={X} label={d.state === 'done' || d.state === 'cancelled' ? 'Retirer' : 'Annuler'} onclick={() => act(deleteDownload, d.id)} />
            </div>
          </div>
          <ProgressBar value={d.received_bytes} max={d.total_bytes} label="Téléchargement de {d.repo}"
            tone={d.state === 'failed' ? 'danger' : d.state === 'paused' ? 'warn' : 'accent'} />
          <div class="meta num">
            <span>{octets(d.received_bytes)} / {octets(d.total_bytes)}</span>
            {#if d.state === 'running'}<span>{octets(d.speed_bps)}/s, reste {eta(d)}</span>{/if}
          </div>
          {#if d.error}<Note tone="danger">{jobError(d.error)}</Note>{/if}
          {#if d.state === 'done'}<p class="done"><a href="#models/presets">Preset créé : voir les presets</a></p>{/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .dl { display: flex; flex-direction: column; gap: var(--s4); padding: var(--s6); max-width: 1000px; }
  .list { list-style: none; margin: 0; padding: 0; border-top: 1px solid var(--line); }
  li { display: flex; flex-direction: column; gap: var(--s2); padding: var(--s4) 0; border-bottom: 1px solid var(--line); }
  .head { display: flex; align-items: center; gap: var(--s3); min-width: 0; }
  .head :global(.lead) { color: var(--accent); }
  .names { display: flex; flex-direction: column; gap: 2px; flex: 1; min-width: 0; }
  .names strong { font-weight: 600; }
  .file { color: var(--muted); font-family: var(--mono); font-size: var(--fs-code); }
  .acts { display: flex; gap: var(--s1); }
  .meta { display: flex; justify-content: space-between; gap: var(--s3); color: var(--muted); font-size: var(--fs-sm); }
  .done { margin: 0; font-size: var(--fs-sm); }
  @media (max-width: 767px) {
    .dl { padding: var(--s4); }
    .head { flex-wrap: wrap; }
    .acts :global(.btn) { width: var(--touch); height: var(--touch); }
  }
</style>
