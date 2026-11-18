<script lang="ts">
  import { onDestroy } from 'svelte';
  import {
    engine, engineLogs, presets, stopEngine, versions, message, jobError, JOB_KIND, JOB_STEP,
    type EngineInfo, type Preset, type Job,
  } from '../../lib/api';

  let info = $state<EngineInfo | null>(null);
  let job = $state<Job | null>(null);
  let list = $state<Preset[]>([]);
  let error = $state('');
  let showLogs = $state(false);
  let lines = $state<string[]>([]);

  async function load() {
    try {
      [info, job] = await Promise.all([engine(), versions().then((v) => v.job)]);
      if (info && info.preset_id !== null && !list.some((p) => p.id === info?.preset_id)) list = await presets();
      if (showLogs) lines = (await engineLogs(200)).lines;
      error = '';
    } catch (e) {
      error = message(e);
    }
  }

  async function stop() {
    try {
      await stopEngine();
      await load();
    } catch (e) {
      error = message(e);
    }
  }

  const label: Record<string, string> = {
    stopped: 'arrêté',
    starting: 'chargement',
    ready: 'prêt',
    failed: 'échec',
    stopping: 'arrêt',
  };
  const originLabel: Record<string, string> = {
    env: 'binaire imposé par l’environnement',
    active: 'moteur installé',
    image: 'moteur de l’image',
    secours: 'moteur de secours',
  };
  let presetName = $derived(list.find((p) => p.id === info?.preset_id)?.name ?? null);
  // Tâche en cours, ou dernière tâche ratée sur le preset (activation, arrêt).
  let shownJob = $derived(
    job && (!job.finished || (job.step === 'failed' && ['activate', 'stop', 'delete'].includes(job.kind))) ? job : null,
  );

  load();
  const timer = setInterval(load, 2000);
  onDestroy(() => clearInterval(timer));
</script>

<section class="card engine-bar">
  {#if info}
    <div class="row wrap">
      <div>
        <strong class="state state-{info.state.state}">{label[info.state.state]}</strong>
        {#if presetName}<span> · {presetName}</span>{/if}
        {#if info.state.state === 'starting'}<span class="muted"> · {info.state.progress} %</span>{/if}
        <span class="muted small"> · {info.build ?? 'build inconnu'}, {originLabel[info.origin]}</span>
        {#if info.state.state === 'ready'}
          <span class="muted small">
            · contexte {info.n_ctx ?? '?'} · outils
            {#if info.tools_ok === true}OK{:else if info.tools_ok === false}<span class="error">KO</span>{:else}?{/if}
          </span>
        {/if}
      </div>
      <div class="actions">
        <button class="link" onclick={() => { showLogs = !showLogs; load(); }}>{showLogs ? 'Masquer les logs' : 'Logs'}</button>
        {#if info.state.state === 'ready' || info.state.state === 'starting'}
          <button class="secondary" onclick={stop} disabled={!!job && !job.finished}>Arrêter</button>
        {/if}
      </div>
    </div>
    {#if info.state.state === 'starting'}
      <div class="bar"><span style:width="{info.state.progress}%"></span></div>
    {/if}
    {#if info.state.state === 'failed'}
      <p class="error">{info.state.cause}</p>
    {/if}
    {#if shownJob}
      <p class="small">
        {JOB_KIND[shownJob.kind]}{shownJob.tag && !['activate', 'stop', 'delete'].includes(shownJob.kind) ? ` (${shownJob.tag})` : ''} :
        <span class="state state-{shownJob.step}">{JOB_STEP[shownJob.step]}</span>
      </p>
      {#if shownJob.error}<p class="error small">{jobError(shownJob.error)}</p>{/if}
    {/if}
    {#if !info.has_fit}
      <p class="small error">Ce llama-server n’a pas --fit : placement par défaut de llama.cpp.</p>
    {/if}
    <div class="gpus">
      {#each info.gpus as g}
        <div class="small">
          <span class="muted">CUDA{g.index}</span> {g.name}
          <div class="bar"><span style:width="{g.total_mib ? (g.used_mib / g.total_mib) * 100 : 0}%"></span></div>
          <span class="muted">{(g.used_mib / 1024).toFixed(1)} / {(g.total_mib / 1024).toFixed(1)} Gio</span>
        </div>
      {/each}
    </div>
    {#if showLogs}
      <pre class="logs">{lines.join('\n')}</pre>
    {/if}
  {/if}
  {#if error}<p class="error">{error}</p>{/if}
</section>
