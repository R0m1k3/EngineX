<script lang="ts">
  import { onDestroy } from 'svelte';
  import {
    benchCases, benchRuns, benchRun, startBench, stopBench, deleteBench, benchCompare, message,
    type BenchCaseDef, type BenchRunSummary, type BenchRun, type BenchCase, type BenchCompare, type BenchSummary,
  } from '../lib/api';
  import { Button, Segmented, Badge, EmptyState, Skeleton, confirmDialog, toast } from '../lib/ui';
  import type { Tone } from '../lib/ui';
  import Page from '../lib/ui/Page.svelte';
  import { stamp } from '../lib/ui/format';
  import CaseDetail from './bench/CaseDetail.svelte';
  import Compare from './bench/Compare.svelte';
  import { FAMILY, VERDICT, STATUS, VISUAL_SCALE, dur, pct, pts, visual } from './bench/shared';
  import Play from '@lucide/svelte/icons/play';
  import Square from '@lucide/svelte/icons/square';
  import Trash2 from '@lucide/svelte/icons/trash-2';
  import FlaskConical from '@lucide/svelte/icons/flask-conical';
  import Star from '@lucide/svelte/icons/star';

  type Tab = 'tests' | 'results' | 'compare';
  const FAMILIES = ['visual', 'code', 'tools'] as const;
  let tab = $state<Tab>('tests');
  let defs = $state<BenchCaseDef[] | null>(null);
  let runs = $state<BenchRunSummary[]>([]);
  let current = $state<BenchRun | null>(null);
  let selectedId = $state<number | null>(null);
  let compare = $state<BenchCompare[]>([]);
  let error = $state('');
  let busy = $state(false);

  const selected = $derived(current?.cases.find((c) => c.id === selectedId) ?? null);
  const active = $derived(runs.find((r) => r.status === 'running') ?? null);
  let live = $state<BenchRun | null>(null);
  const liveCase = $derived(live?.cases.find((c) => c.status === 'running') ?? null);
  const liveIndex = $derived(live && liveCase ? live.cases.indexOf(liveCase) + 1 : 0);
  const liveByKey = $derived(new Map((live?.cases ?? []).map((c) => [c.key, c])));
  let nowS = $state(Math.floor(Date.now() / 1000));
  const elapsed = (c: BenchCase | null | undefined) =>
    c?.started_at == null ? null : Math.max(0, (c.finished_at ?? (c.status === 'running' ? nowS : c.started_at)) - c.started_at);
  const toRate = $derived(current?.cases.filter((c) => c.family === 'visual' && c.verdict && c.visual == null) ?? []);

  async function refresh() {
    try {
      const [r, d] = await Promise.all([benchRuns(), defs ? Promise.resolve(defs) : benchCases()]);
      runs = r;
      defs = d;
      const run = r.find((x) => x.status === 'running');
      live = run ? await benchRun(run.id) : null;
      if (current) current = await benchRun(current.id);
      error = '';
    } catch (e) {
      error = message(e);
    }
  }
  refresh();
  const timer = setInterval(() => { if (active || current?.status === 'running') refresh(); }, 3000);
  const clock = setInterval(() => { if (live) nowS = Math.floor(Date.now() / 1000); }, 1000);
  onDestroy(() => { clearInterval(timer); clearInterval(clock); });

  async function launch(keys?: string[]) {
    busy = true;
    try {
      const { id } = await startBench(keys);
      toast(keys?.length === 1 ? 'Test lancé.' : 'Lot lancé.');
      await refresh();
      await open(id);
    } catch (e) {
      toast(message(e), 'error');
    } finally {
      busy = false;
    }
  }

  async function stop() {
    if (!active) return;
    try { await stopBench(active.id); await refresh(); } catch (e) { toast(message(e), 'error'); }
  }

  async function open(id: number) {
    try {
      current = await benchRun(id);
      selectedId = null;
      tab = 'results';
    } catch (e) {
      toast(message(e), 'error');
    }
  }

  async function remove(r: BenchRunSummary) {
    if (!(await confirmDialog(`Supprimer le run ${r.id} ? Ses sessions de test et ses captures sont supprimées avec lui.`, { title: 'Supprimer le run', confirm: 'Supprimer', danger: true }))) return;
    try {
      await deleteBench(r.id);
      if (current?.id === r.id) { current = null; selectedId = null; }
      await refresh();
    } catch (e) {
      toast(message(e), 'error');
    }
  }

  async function showCompare() {
    tab = 'compare';
    try { compare = await benchCompare(); } catch (e) { error = message(e); }
  }

  async function saved(next: boolean) {
    await refresh();
    if (next) {
      const n = current?.cases.find((c) => c.family === 'visual' && c.verdict && c.visual == null && c.id !== selectedId);
      selectedId = n?.id ?? null;
      if (!n) toast('Tous les visuels de ce run sont notés.');
    }
  }

  const tone = (s: string): Tone => (s === 'done' ? 'ok' : s === 'failed' ? 'danger' : s === 'running' ? 'warn' : 'neutral');
  const line = (s: BenchSummary) =>
    `Réussis ${s.passed}/${s.cases} · Résultat ${pct(s.result)} · Visuel ${visual(s.visual)}`;
</script>

{#snippet summaryBlock(s: BenchSummary)}
  <div class="kpis">
    <div><span class="k">Réussis</span><strong class="num">{s.passed} / {s.cases}</strong><span class="faint small">{s.partial} partiel(s) · {s.failed} échoué(s)</span></div>
    <div><span class="k">Résultat</span><strong class="num">{pct(s.result)}</strong><span class="faint small">points gagnés / points possibles</span></div>
    <div><span class="k">Visuel</span><strong class="num">{visual(s.visual)}</strong><span class="faint small">{s.visual_rated} noté(s){s.to_rate ? ` · ${s.to_rate} à noter` : ''}</span></div>
    <div><span class="k">Durée</span><strong class="num">{dur(s.duration_s)}</strong><span class="faint small">{s.tps ? `${s.tps.toFixed(1)} t/s en moyenne` : ''}</span></div>
  </div>
{/snippet}

<Page title="Tester">
  {#snippet meta()}{#if active}<span class="num">run {active.id} · {active.preset}{#if liveCase} · test {liveIndex}/{live?.cases.length} : {liveCase.title} · {dur(elapsed(liveCase) ?? 0)}{#if liveCase.reason} · {liveCase.reason}{/if}{/if}</span>{/if}{/snippet}
  {#snippet actions()}
    {#if active}
      <Button variant="danger" icon={Square} onclick={stop}>Stop</Button>
    {:else}
      <Button variant="primary" icon={Play} onclick={() => launch()} disabled={busy}>Tout lancer ({defs?.length ?? 15})</Button>
    {/if}
  {/snippet}
  {#snippet tabs()}
    <Segmented
      label="Vue"
      bind:value={tab}
      options={[{ value: 'tests', label: 'Tests' }, { value: 'results', label: 'Résultats' }, { value: 'compare', label: 'Comparaison' }]}
      onchange={(v) => { if (v === 'compare') showCompare(); }}
    />
  {/snippet}
  {#if error}<p class="err-line" role="alert">{error}</p>{/if}

  {#if tab === 'tests'}
    {#if !defs}
      <div class="pad"><Skeleton height="120px" /></div>
    {:else}
      <details class="howto pad">
        <summary><strong>Comment lire les résultats</strong></summary>
        <ul>
          <li><strong>Résultat</strong> (machine) : chaque contrôle vaut des points ; le test affiche les points gagnés sur 20. Les contrôles des tests cachés comptent au prorata.</li>
          <li><strong>Réussi</strong> : tout passe. <strong>Partiel</strong> : les contrôles essentiels passent, d’autres non. <strong>Échoué</strong> : un contrôle essentiel rate.</li>
          <li><strong>Visuel</strong> (vous, tests visuels seulement) : 1 à 5, {VISUAL_SCALE.map((s) => `${s.value} ${s.label.toLowerCase()}`).join(' · ')}. Jamais mélangé aux points.</li>
          <li>Les alertes (fichier long, commande répétée, paquet ajouté) sont pour information.</li>
        </ul>
      </details>
      {#each FAMILIES as fam (fam)}
        <h3 class="pad fam">{FAMILY[fam].label} <span class="faint small">· {defs.filter((d) => d.family === fam).length} tests</span></h3>
        <p class="faint small pad hint">{FAMILY[fam].hint}</p>
        <ul class="cases">
          {#each defs.filter((d) => d.family === fam) as d (d.key)}
            <li class="case">
              <div class="grow">
                <strong>{d.title}</strong>
                <span class="faint small"> · {d.points_max} pts · plafond {Math.round(d.timeout_s / 60)} min</span>
                <p class="small sum">Contrôle : {d.summary}.</p>
                <details>
                  <summary class="faint small">Consigne et contrôles</summary>
                  <p class="prompt">{d.prompt}</p>
                  <ul class="plain small">
                    {#each d.checks as k (k.name)}<li><span class="num">{k.points} pts</span> · {k.name}{#if k.essential} <em class="faint">(essentiel)</em>{/if}</li>{/each}
                  </ul>
                </details>
              </div>
              {#if liveByKey.has(d.key)}
                {@const lc = liveByKey.get(d.key)!}
                <span class="num faint small">{elapsed(lc) != null ? dur(elapsed(lc)!) : ''}{lc.status === 'running' && lc.reason ? ` · ${lc.reason}` : ''}</span>
                {#if lc.verdict}<Badge tone={VERDICT[lc.verdict].tone}>{VERDICT[lc.verdict].label} · {pts(lc.points)}/{lc.points_max}</Badge>
                {:else}<Badge tone={tone(lc.status)} dot={lc.status === 'running'}>{STATUS[lc.status]}</Badge>{/if}
              {/if}
              <Button size="sm" icon={Play} onclick={() => launch([d.key])} disabled={busy || !!active}>Lancer</Button>
            </li>
          {/each}
        </ul>
      {/each}
    {/if}
  {:else if tab === 'results'}
    {#if selected && current}
      <CaseDetail runId={current.id} c={selected} elapsed={elapsed(selected)} onback={() => (selectedId = null)} onsaved={saved} />
    {:else}
      <div class="split">
        <ul class="runs">
          {#each runs as r (r.id)}
            <li class:sel={current?.id === r.id}>
              <button type="button" class="runbtn" onclick={() => open(r.id)}>
                <strong>Run {r.id}</strong> · {r.preset}
                <span class="faint small">{stamp(r.started_at)}</span>
                <span class="small num">{line(r.summary)}</span>
              </button>
              <Badge tone={r.status === 'done' ? 'ok' : r.status === 'running' ? 'warn' : 'neutral'}>{STATUS[r.status]}</Badge>
              {#if r.status !== 'running'}<Button variant="ghost" size="sm" icon={Trash2} label="Supprimer le run" onclick={() => remove(r)} />{/if}
            </li>
          {:else}
            <li class="pad"><EmptyState icon={FlaskConical} title="Aucun run" text="Lancez un test depuis l’onglet Tests." /></li>
          {/each}
        </ul>
        {#if current}
          <div class="runview">
            {@render summaryBlock(current.summary)}
            {#if toRate.length}
              <Button variant="primary" icon={Star} onclick={() => (selectedId = toRate[0].id)}>Noter les visuels ({toRate.length})</Button>
            {/if}
            <table class="grid">
              <thead><tr><th>Test</th><th>Verdict</th><th class="num">Points</th><th class="num">Visuel</th><th class="num">Durée</th></tr></thead>
              <tbody>
                {#each FAMILIES as fam (fam)}
                  {@const rows = current.cases.filter((c) => c.family === fam)}
                  {#if rows.length}
                    <tr class="famrow"><td colspan="5">{FAMILY[fam].label}</td></tr>
                    {#each rows as c (c.id)}
                      <tr class="row" onclick={() => (selectedId = c.id)} onkeydown={(e) => e.key === 'Enter' && (selectedId = c.id)} tabindex="0">
                        <td>{c.title}</td>
                        <td>
                          {#if c.verdict}<Badge tone={VERDICT[c.verdict].tone}>{VERDICT[c.verdict].label}</Badge>
                          {:else}<Badge tone={tone(c.status)} dot={c.status === 'running'}>{STATUS[c.status]}</Badge>{/if}
                        </td>
                        <td class="num">{c.points == null ? '—' : `${pts(c.points)} / ${c.points_max}`}</td>
                        <td class="num">{c.family !== 'visual' ? '' : c.visual != null ? `${c.visual} / 5` : c.verdict ? 'à noter' : '—'}</td>
                        <td class="num">{elapsed(c) != null ? dur(elapsed(c)!) : '—'}</td>
                      </tr>
                    {/each}
                  {/if}
                {/each}
              </tbody>
            </table>
          </div>
        {:else}
          <p class="faint pad">Choisissez un run.</p>
        {/if}
      </div>
    {/if}
  {:else}
    <Compare rows={compare} defs={defs ?? []} />
  {/if}
</Page>

<style>
  .pad { padding: 0 var(--sp-4, 16px); }
  .howto { margin: 12px 0; }
  .howto ul { margin: 8px 0; padding-left: 20px; display: grid; gap: 4px; font-size: var(--fs-sm); }
  .fam { margin: 16px 0 2px; font-size: var(--fs-md, 1rem); }
  .hint { margin: 0 0 8px; }
  .cases { list-style: none; margin: 0; padding: 0 var(--sp-4, 16px) var(--sp-2, 8px); display: grid; gap: 8px; }
  .case { display: flex; align-items: flex-start; gap: 12px; border: 1px solid var(--line); border-radius: var(--r); padding: 12px; }
  .grow { flex: 1; min-width: 0; }
  .sum { margin: 4px 0 0; color: var(--muted); }
  .prompt { margin: 6px 0; white-space: pre-wrap; font-size: var(--fs-sm); }
  .plain { list-style: none; margin: 0; padding: 0; display: grid; gap: 2px; }
  .split { display: grid; grid-template-columns: minmax(240px, 320px) 1fr; gap: 16px; padding: 0 var(--sp-4, 16px) var(--sp-4, 16px); }
  @media (max-width: 800px) { .split { grid-template-columns: 1fr; } }
  .runs { list-style: none; margin: 0; padding: 0; display: grid; gap: 6px; align-content: start; }
  .runs li { display: flex; align-items: center; gap: 6px; border: 1px solid var(--line); border-radius: var(--r); padding: 6px 8px; }
  .runs li.sel { border-color: var(--accent); }
  .runbtn { all: unset; cursor: pointer; flex: 1; display: grid; gap: 2px; min-width: 0; }
  .runview { display: grid; gap: 12px; align-content: start; min-width: 0; }
  .kpis { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 8px; }
  .kpis > div { display: grid; gap: 2px; border: 1px solid var(--line); border-radius: var(--r); padding: 10px 12px; }
  .kpis .k { font-size: var(--fs-sm); color: var(--muted); }
  .kpis strong { font-size: 1.4rem; }
  .grid { width: 100%; border-collapse: collapse; font-size: var(--fs-sm); }
  .grid th, .grid td { text-align: left; padding: 8px; border-bottom: 1px solid var(--line); }
  .grid .num { text-align: right; font-variant-numeric: tabular-nums; white-space: nowrap; }
  .famrow td { font-weight: 600; color: var(--muted); padding-top: 14px; }
  .row { cursor: pointer; }
  .row:hover, .row:focus-visible { background: var(--raised); }
  .err-line { margin: 8px var(--sp-4, 16px); }
  .faint { color: var(--faint, var(--muted)); }
  .small { font-size: var(--fs-sm); }
</style>
