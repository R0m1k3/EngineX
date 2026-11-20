<script lang="ts">
  // Bandeau du moteur, en haut de la page Modèles : état, preset chargé, débit, cartes, journaux.
  import { onDestroy } from 'svelte';
  import ScrollText from '@lucide/svelte/icons/scroll-text';
  import Square from '@lucide/svelte/icons/square';
  import Button from '../../lib/ui/Button.svelte';
  import StatusDot from '../../lib/ui/StatusDot.svelte';
  import ProgressBar from '../../lib/ui/ProgressBar.svelte';
  import Gauge from '../../lib/ui/Gauge.svelte';
  import { dec1, int } from '../../lib/ui/format';
  import type { DotKind, GaugeRow } from '../../lib/ui/types';
  import Note from '../settings/Note.svelte';
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
  const dot: Record<string, DotKind> = { stopped: 'off', starting: 'warn', ready: 'ok', failed: 'err', stopping: 'warn' };
  const originLabel: Record<string, string> = {
    env: 'binaire imposé par l’environnement',
    active: 'moteur installé',
    image: 'moteur de l’image',
    secours: 'moteur de secours',
  };
  let preset = $derived(list.find((p) => p.id === info?.preset_id) ?? null);
  let presetName = $derived(preset?.name ?? null);
  // `--fit` a réduit le contexte faute de VRAM : EX reçoit ce contexte-là.
  let ctxCut = $derived(
    info?.state.state === 'ready' && info.n_ctx != null && preset != null && info.n_ctx < preset.settings.ctx,
  );
  let gen = $derived(info?.last_gen ?? null);
  // Tout sur GPU, llama-server occupe ≈ 1 cœur (le fil qui attend les cartes).
  // Au-delà, des couches sont calculées par le processeur.
  let cpuHeavy = $derived(
    info?.state.state === 'ready' && (info?.gpus.length ?? 0) > 0 && gen != null &&
      gen.preset === presetName && (gen.cpu_cores ?? 0) >= 2.5,
  );
  // Tâche en cours, ou dernière tâche ratée sur le preset (activation, arrêt).
  let shownJob = $derived(
    job && (!job.finished || (job.step === 'failed' && ['activate', 'stop', 'delete'].includes(job.kind))) ? job : null,
  );
  let rows = $derived<GaugeRow[]>(
    (info?.gpus ?? []).map((g) => ({ label: `CUDA${g.index}`, sub: g.name, total_mib: g.total_mib, used_mib: g.used_mib })),
  );

  load();
  const timer = setInterval(load, 2000);
  onDestroy(() => clearInterval(timer));
</script>

<section class="eng" aria-label="Moteur">
  {#if info}
    {@const st = info.state.state}
    <div class="main">
      <div class="who">
        <div class="line1">
          <StatusDot kind={dot[st]} />
          <span class="state">{label[st]}</span>
          {#if presetName}<strong class="ell pname">{presetName}</strong>{/if}
          {#if info.state.state === 'starting'}<span class="num muted">{info.state.progress} %</span>{/if}
        </div>
        <div class="line2 muted">{info.build ?? 'build inconnu'}, {originLabel[info.origin]}</div>
      </div>

      {#if st === 'ready'}
        <dl class="stats">
          <div><dt>Contexte</dt><dd class="num">{info.n_ctx != null ? int(info.n_ctx) : '?'}</dd></div>
          <div>
            <dt>Outils</dt>
            <dd>{#if info.tools_ok === true}OK{:else if info.tools_ok === false}<span class="bad">KO</span>{:else}?{/if}</dd>
          </div>
          {#if gen?.tokens_per_s != null}
            <div><dt>Dernière réponse</dt><dd class="num">{dec1(gen.tokens_per_s)} jetons/s</dd></div>
            {#if gen.draft_n}
              <div>
                <dt>Brouillon accepté</dt>
                <dd class="num">{gen.draft_accepted ?? 0}/{gen.draft_n} ({Math.round(((gen.draft_accepted ?? 0) / gen.draft_n) * 100)} %)</dd>
              </div>
            {/if}
          {/if}
        </dl>
      {/if}

      {#if rows.length}
        <div class="eg"><Gauge {rows} size="md" compact /></div>
      {/if}

      <div class="acts">
        <Button variant="ghost" icon={ScrollText} aria-pressed={showLogs} onclick={() => { showLogs = !showLogs; load(); }}>
          {showLogs ? 'Masquer les journaux' : 'Journaux'}
        </Button>
        {#if st === 'ready' || st === 'starting'}
          <Button icon={Square} onclick={stop} disabled={!!job && !job.finished}>Arrêter</Button>
        {/if}
      </div>
    </div>

    {#if info.state.state === 'starting'}
      <ProgressBar value={info.state.progress} label="Chargement du modèle" />
    {/if}

    {#if ctxCut && preset}
      <Note tone="warn">
        Contexte réduit par --fit : {int(info.n_ctx ?? 0)} jetons chargés au lieu de {int(preset.settings.ctx)} (VRAM courte).
        EX reçoit {int(info.n_ctx ?? 0)}. Baisser le contexte, passer le KV en q4_0 ou mettre des experts en RAM.
      </Note>
    {/if}
    {#if cpuHeavy && gen}
      <Note tone="warn">
        Le processeur calcule une partie du modèle ({gen.cpu_cores} cœurs occupés pendant la dernière réponse) :
        il ne tient pas entier dans la VRAM, d’où la lenteur. Dans le preset : flash attention activée, cache q8_0,
        contexte plus court, ou vision désactivée.
      </Note>
    {/if}
    {#if info.spec_fallback && info.spec_fallback.preset_id === info.preset_id && st === 'ready'}
      <Note tone="warn">Chargé sans décodage spéculatif : le brouillon a fait échouer le chargement ({info.spec_fallback.why}).</Note>
    {/if}
    {#if info.state.state === 'failed'}
      <Note tone="danger">{info.state.cause}</Note>
    {/if}
    {#if shownJob}
      <div class="job">
        {JOB_KIND[shownJob.kind]}{shownJob.tag && !['activate', 'stop', 'delete'].includes(shownJob.kind) ? ` (${shownJob.tag})` : ''} :
        <span class:okc={shownJob.step === 'done'} class:bad={shownJob.step === 'failed'}>{JOB_STEP[shownJob.step]}</span>
      </div>
      {#if shownJob.error}<Note tone="danger">{jobError(shownJob.error)}</Note>{/if}
    {/if}
    {#if !info.has_fit}
      <Note tone="warn">Ce llama-server n’a pas --fit : placement par défaut de llama.cpp.</Note>
    {/if}
    {#if showLogs}
      <pre class="elog" aria-label="Journal de llama-server">{lines.join('\n') || 'Journal vide.'}</pre>
    {/if}
  {/if}
  {#if error}<Note tone="danger">{error}</Note>{/if}
</section>

<style>
  .eng {
    display: flex;
    flex-direction: column;
    gap: var(--s3);
    padding: var(--s3) var(--s6);
    background: var(--panel);
    border-bottom: 1px solid var(--line);
    font-size: var(--fs-sm);
  }
  .eng:empty { display: none; }
  .main { display: flex; align-items: center; gap: var(--s6); flex-wrap: wrap; min-height: 40px; }
  .who { min-width: 0; flex: 0 1 auto; }
  .line1 { display: flex; align-items: center; gap: var(--s2); min-width: 0; }
  .state { color: var(--muted); }
  .pname { font-weight: 600; font-size: var(--fs-md); max-width: 36ch; }
  .line2 { margin-top: 2px; }
  .stats { display: flex; gap: var(--s5); margin: 0; }
  .stats div { display: flex; flex-direction: column; gap: 2px; }
  dt { color: var(--muted); }
  dd { margin: 0; color: var(--text); }
  .eg { flex: 1 1 300px; max-width: 420px; min-width: 240px; }
  .acts { display: flex; gap: var(--s2); margin-left: auto; }
  .bad { color: var(--danger); }
  .okc { color: var(--accent); }
  .job { color: var(--muted); }
  .elog {
    margin: 0;
    max-height: 360px;
    overflow: auto;
    padding: var(--s3) var(--s4);
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: var(--r);
    color: var(--text);
    font-family: var(--mono);
    font-size: var(--fs-code);
    line-height: 1.6;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    tab-size: 4;
  }
  @media (max-width: 767px) {
    .eng { padding: var(--s3) var(--s4); }
    .main { gap: var(--s3); }
    .stats { flex-wrap: wrap; gap: var(--s2) var(--s4); }
    /* Téléphone : la barre d'état montre déjà la VRAM. */
    .eg, .line2 { display: none; }
    .main { flex-wrap: wrap; }
    .acts { margin-left: auto; }
    .acts :global(.btn) { height: var(--touch); }
  }
</style>
