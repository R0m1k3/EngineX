<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import {
    benchCases, benchRuns, benchRun, startBench, stopBench, deleteBench, rateBench, benchCompare, sessionDetail, message,
    type BenchCaseDef, type BenchRunSummary, type BenchRun, type BenchCase, type BenchCompare, type ChangedFile,
  } from '../lib/api';
  import { Button, Select, Textarea, Segmented, Badge, EmptyState, Skeleton, confirmDialog, toast } from '../lib/ui';
  import type { Tone } from '../lib/ui';
  import Page from '../lib/ui/Page.svelte';
  import { since, stamp } from '../lib/ui/format';
  import Preview from './sessions/Preview.svelte';
  import Play from '@lucide/svelte/icons/play';
  import Square from '@lucide/svelte/icons/square';
  import Trash2 from '@lucide/svelte/icons/trash-2';
  import FlaskConical from '@lucide/svelte/icons/flask-conical';
  import ArrowLeft from '@lucide/svelte/icons/arrow-left';
  import MessageSquare from '@lucide/svelte/icons/message-square';

  type Tab = 'tests' | 'results' | 'compare';
  let tab = $state<Tab>('tests');
  let defs = $state<BenchCaseDef[] | null>(null);
  let runs = $state<BenchRunSummary[]>([]);
  let current = $state<BenchRun | null>(null);
  let selected = $state<BenchCase | null>(null);
  let compare = $state<BenchCompare[]>([]);
  let error = $state('');
  let busy = $state(false);

  const active = $derived(runs.find((r) => r.status === 'running') ?? null);
  // Run en cours, avec ses cas : sert au bandeau et à la liste des tests.
  let live = $state<BenchRun | null>(null);
  const liveCase = $derived(live?.cases.find((c) => c.status === 'running') ?? null);
  const liveIndex = $derived(live && liveCase ? live.cases.indexOf(liveCase) + 1 : 0);
  const liveByKey = $derived(new Map((live?.cases ?? []).map((c) => [c.key, c])));
  // Horloge à la seconde pour les temps qui avancent.
  let nowS = $state(Math.floor(Date.now() / 1000));
  const elapsed = (c: BenchCase | null | undefined) =>
    c?.started_at == null ? null : Math.max(0, (c.finished_at ?? (c.status === 'running' ? nowS : c.started_at)) - c.started_at);

  async function refresh() {
    try {
      const [r, d] = await Promise.all([benchRuns(), defs ? Promise.resolve(defs) : benchCases()]);
      runs = r;
      defs = d;
      const run = r.find((x) => x.status === 'running');
      live = run ? await benchRun(run.id) : null;
      error = '';
      if (current) {
        current = await benchRun(current.id);
        if (selected) selected = current.cases.find((c) => c.id === selected!.id) ?? null;
      }
    } catch (e) {
      error = message(e);
    }
  }
  refresh();
  // Pendant un run, la page se met à jour toute seule.
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
      selected = null;
      tab = 'results';
    } catch (e) {
      toast(message(e), 'error');
    }
  }

  async function remove(r: BenchRunSummary) {
    if (!(await confirmDialog(`Supprimer le run ${r.id} ? Ses sessions de test sont supprimées avec lui.`, { title: 'Supprimer le run', confirm: 'Supprimer', danger: true }))) return;
    try {
      await deleteBench(r.id);
      if (current?.id === r.id) { current = null; selected = null; }
      await refresh();
    } catch (e) {
      toast(message(e), 'error');
    }
  }

  async function showCompare() {
    tab = 'compare';
    try { compare = await benchCompare(); } catch (e) { error = message(e); }
  }

  // Dernier résultat connu d'un cas (run le plus récent qui l'a exécuté).
  const lastByKey = $derived.by(() => {
    const m = new Map<string, number>();
    if (current) for (const c of current.cases) if (c.final != null) m.set(c.key, c.final);
    return m;
  });

  const score = (v: number | null | undefined) => (v == null ? '—' : `${Math.round(v)}`);
  const tone = (s: string): Tone => (s === 'done' ? 'ok' : s === 'failed' ? 'danger' : s === 'running' ? 'warn' : 'neutral');
  const label: Record<string, string> = {
    pending: 'en attente', running: 'en cours', done: 'terminé', failed: 'échec', stopped: 'arrêté',
  };
  const dur = (s: number) => (s >= 60 ? `${Math.floor(s / 60)} min ${s % 60} s` : `${s} s`);

  // ---- Détail d'un cas ----
  let files = $state<ChangedFile[]>([]);
  let axes = $state({ fonctionne: 3, qualite: 3, rendu: 3, consigne: 3 });
  let comment = $state('');
  let saving = $state(false);
  const scale = [1, 2, 3, 4, 5].map((n) => ({ value: n, label: `${n}` }));
  const axisLabels: [keyof typeof axes, string][] = [
    ['fonctionne', 'Fonctionne'], ['qualite', 'Qualité du code'], ['rendu', 'Rendu visuel'], ['consigne', 'Suivi de la consigne'],
  ];

  // Le rafraîchissement remplace `selected` toutes les 3 s pendant un run :
  // l'effet ne repart qu'au changement de cas ou à la fin de son audit, pour ne pas
  // effacer une note en cours de saisie.
  const selKey = $derived(selected ? `${selected.id}:${selected.audited_at ?? ''}` : '');
  $effect(() => {
    if (!selKey) { files = []; return; }
    const c = untrack(() => selected)!;
    files = [];
    axes = c.manual ? { ...c.manual } : { fonctionne: 3, qualite: 3, rendu: 3, consigne: 3 };
    comment = c.comment ?? '';
    if (c.session_id) {
      sessionDetail(c.session_id)
        .then((d) => {
          const seen = new Map<string, ChangedFile>();
          for (const t of d.turns) for (const f of t.changes?.files ?? []) seen.set(f.path, f);
          files = [...seen.values()];
        })
        .catch(() => {});
    }
  });

  async function save() {
    if (!current || !selected) return;
    saving = true;
    try {
      await rateBench(current.id, selected.id, { ...axes, comment });
      toast('Note enregistrée.');
      await refresh();
    } catch (e) {
      toast(message(e), 'error');
    } finally {
      saving = false;
    }
  }
</script>

<Page title="Tester">
  {#snippet meta()}{#if active}<span class="num">run {active.id} · {active.preset}{#if liveCase} · test {liveIndex}/{live?.cases.length} : {liveCase.title} · {dur(elapsed(liveCase) ?? 0)}{#if liveCase.reason} · {liveCase.reason}{/if}{/if}</span>{/if}{/snippet}
  {#snippet actions()}
    {#if active}
      <Button variant="danger" icon={Square} onclick={stop}>Stop</Button>
    {:else}
      <Button variant="primary" icon={Play} onclick={() => launch()} disabled={busy}>Tout lancer</Button>
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
      <p class="faint pad">
        Dix tâches de codage lancées sur le preset actif, une session chacune. Après chaque test, une fiche d’audit montre
        ce que le modèle a fait ; la note se saisit ensuite.
      </p>
      <ul class="cases">
        {#each defs as d (d.key)}
          <li class="case">
            <div class="grow">
              <strong>{d.title}</strong>
              <span class="faint small"> · {d.criteria_count} critères · plafond {Math.round(d.timeout_s / 60)} min</span>
              <details><summary class="faint small">Consigne</summary><p class="prompt">{d.prompt}</p></details>
            </div>
            {#if liveByKey.has(d.key)}
              {@const lc = liveByKey.get(d.key)!}
              <span class="num faint small">{elapsed(lc) != null ? dur(elapsed(lc)!) : ''}{lc.status === 'running' && lc.reason ? ` · ${lc.reason}` : ''}</span>
              <Badge tone={tone(lc.status)} dot={lc.status === 'running'}>{label[lc.status]}</Badge>
            {:else if lastByKey.has(d.key)}<Badge tone="ok" title="Note finale du run affiché">{score(lastByKey.get(d.key))}</Badge>{/if}
            <Button size="sm" icon={Play} onclick={() => launch([d.key])} disabled={busy || !!active}>Lancer</Button>
          </li>
        {/each}
      </ul>
    {/if}
  {:else if tab === 'results'}
    {#if selected && current}
      {@const c = selected}
      <div class="detail">
        <div class="rowline">
          <Button variant="ghost" icon={ArrowLeft} onclick={() => (selected = null)}>Retour</Button>
          <h2 class="grow">{c.title}</h2>
          {#if elapsed(c) != null}<span class="num faint">{dur(elapsed(c)!)}</span>{/if}
          <Badge tone={tone(c.status)} dot>{label[c.status]}</Badge>
          {#if c.session_id}<Button size="sm" icon={MessageSquare} href={`#sessions/${c.session_id}`}>Ouvrir la session</Button>{/if}
        </div>
        {#if c.reason}<p class="warn-line">{c.reason}</p>{/if}

        <section>
          <h3>1. Ce que le modèle a fait</h3>
          {#if !c.audit}
            <p class="faint">
              {c.status === 'running' || c.status === 'pending' ? 'Pas encore d’audit : le test est en cours.' : 'Pas d’audit pour ce test.'}
            </p>
          {:else}
            <p class="metrics num">
              durée {dur(c.audit.metrics.duration_s)} · {c.audit.metrics.tokens} jetons ·
              {c.audit.metrics.tps ? `${c.audit.metrics.tps.toFixed(1)} t/s` : 't/s —'} ·
              {c.audit.metrics.commands} commandes · {c.audit.metrics.file_changes} écritures de fichiers
            </p>
            <ul class="crit">
              {#each c.audit.criteria as k (k.name)}
                <li>
                  <details>
                    <summary><Badge tone={k.ok ? 'ok' : 'danger'}>{k.ok ? 'OK' : 'KO'}</Badge> {k.name}</summary>
                    <pre class="proof">{k.proof}</pre>
                  </details>
                </li>
              {/each}
            </ul>
            <p class="faint small">
              Score auto : {c.auto_score == null ? 'n/a (aucun critère)' : `${c.audit.criteria.filter((k) => k.ok).length} critères sur ${c.audit.auto_count}, soit ${score(c.auto_score)}`}.
              Le rendu visuel se juge ci-dessous, à l’aperçu.
            </p>
            {#if c.audit.alerts.length}
              <h4>Alertes (sans effet sur la note)</h4>
              <ul class="alerts">
                {#each c.audit.alerts as a, i (i)}<li><Badge tone="warn">{a.kind === 'big_file' ? 'fichier long' : a.kind === 'repeated_command' ? 'commande répétée' : 'paquet ajouté'}</Badge> {a.text}</li>{/each}
              </ul>
            {/if}
          {/if}
          {#if files.length}
            <h4>Fichiers touchés</h4>
            <ul class="files">
              {#each files as f (f.path)}<li><code>{f.status}</code> {f.path} <span class="faint small">+{f.added} −{f.deleted}</span></li>{/each}
            </ul>
          {/if}
        </section>

        {#if c.session_id}
          <section>
            <h3>2. Aperçu</h3>
            <div class="preview"><Preview sessionId={c.session_id} /></div>
          </section>
        {/if}

        <section>
          <h3>3. Votre note</h3>
          {#if !c.audited_at}
            <p class="faint">La note est possible dès que l’audit est terminé.</p>
          {:else}
            <div class="axes">
              {#each axisLabels as [k, l] (k)}
                <Select label={l} bind:value={axes[k]} options={scale} />
              {/each}
            </div>
            <Textarea label="Commentaire" bind:value={comment} rows={3} />
            <div class="rowline">
              <Button variant="primary" onclick={save} busy={saving}>Enregistrer la note</Button>
              <span class="num faint">
                auto {score(c.auto_score)} · manuelle {score(c.manual_score)} · finale {score(c.final)}
              </span>
            </div>
          {/if}
        </section>
      </div>
    {:else}
      <div class="split">
        <ul class="runs">
          {#each runs as r (r.id)}
            <li class:sel={current?.id === r.id}>
              <button type="button" class="runbtn" onclick={() => open(r.id)}>
                <strong>Run {r.id}</strong> · {r.preset}
                <span class="faint small">{stamp(r.started_at)} · {r.rated}/{r.cases} notés · moyenne {score(r.average)}</span>
              </button>
              <Badge tone={r.status === 'done' ? 'ok' : r.status === 'running' ? 'warn' : 'neutral'}>{label[r.status]}</Badge>
              {#if r.status !== 'running'}<Button variant="ghost" size="sm" icon={Trash2} label="Supprimer le run" onclick={() => remove(r)} />{/if}
            </li>
          {:else}
            <li class="pad"><EmptyState icon={FlaskConical} title="Aucun run" text="Lancez un test depuis l’onglet Tests." /></li>
          {/each}
        </ul>
        {#if current}
          <table class="grid">
            <thead><tr><th>Test</th><th>État</th><th>Durée</th><th>Auto</th><th>Manuelle</th><th>Finale</th></tr></thead>
            <tbody>
              {#each current.cases as c (c.id)}
                <tr class="row" onclick={() => (selected = c)} onkeydown={(e) => e.key === 'Enter' && (selected = c)} tabindex="0">
                  <td>{c.title}</td>
                  <td><Badge tone={tone(c.status)}>{label[c.status]}</Badge>{#if c.audited_at && c.manual_score == null} <span class="faint small">à noter</span>{/if}</td>
                  <td class="num">{elapsed(c) != null ? dur(elapsed(c)!) : '—'}</td>
                  <td class="num">{score(c.auto_score)}</td>
                  <td class="num">{score(c.manual_score)}</td>
                  <td class="num">{score(c.final)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        {:else}
          <p class="faint pad">Choisissez un run.</p>
        {/if}
      </div>
    {/if}
  {:else}
    {#if compare.length === 0}
      <div class="pad"><EmptyState icon={FlaskConical} title="Rien à comparer" text="La comparaison se remplit avec les tests notés." /></div>
    {:else}
      <p class="faint pad">Moyenne des notes finales par preset (un run = un échantillon : le nombre de runs est indiqué).</p>
      <table class="grid">
        <thead>
          <tr><th>Preset</th><th>Runs</th><th>Moyenne</th>{#each defs ?? [] as d (d.key)}<th title={d.title}>{d.key}</th>{/each}</tr>
        </thead>
        <tbody>
          {#each compare as p (p.preset_id)}
            <tr>
              <td>{p.preset}</td><td class="num">{p.runs}</td><td class="num"><strong>{score(p.average)}</strong></td>
              {#each defs ?? [] as d (d.key)}
                {@const x = p.cases.find((k) => k.key === d.key)}
                <td class="num">{x ? score(x.average) : '—'}</td>
              {/each}
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  {/if}
</Page>

<style>
  .pad { padding: var(--sp-4, 16px); }
  .cases { list-style: none; margin: 0; padding: 0 var(--sp-4, 16px) var(--sp-4, 16px); display: grid; gap: 8px; }
  .case { display: flex; align-items: flex-start; gap: 12px; border: 1px solid var(--line); border-radius: var(--r); padding: 12px; }
  .grow { flex: 1; min-width: 0; }
  .prompt { margin: 6px 0 0; white-space: pre-wrap; font-size: var(--fs-sm); }
  .split { display: grid; grid-template-columns: minmax(240px, 320px) 1fr; gap: 16px; padding: 0 var(--sp-4, 16px); }
  @media (max-width: 800px) { .split { grid-template-columns: 1fr; } }
  .runs { list-style: none; margin: 0; padding: 0; display: grid; gap: 6px; align-content: start; }
  .runs li { display: flex; align-items: center; gap: 6px; border: 1px solid var(--line); border-radius: var(--r); padding: 6px 8px; }
  .runs li.sel { border-color: var(--accent); }
  .runbtn { all: unset; cursor: pointer; flex: 1; display: grid; gap: 2px; min-width: 0; }
  .grid { width: 100%; border-collapse: collapse; font-size: var(--fs-sm); }
  .grid th, .grid td { text-align: left; padding: 8px; border-bottom: 1px solid var(--line); }
  .grid .num { text-align: right; font-variant-numeric: tabular-nums; }
  .row { cursor: pointer; }
  .row:hover, .row:focus-visible { background: var(--hover, rgba(127, 127, 127, 0.1)); }
  .detail { display: grid; gap: 16px; padding: 0 var(--sp-4, 16px) var(--sp-4, 16px); }
  .rowline { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; }
  h2 { margin: 0; font-size: var(--fs-lg, 1.1rem); }
  h3 { margin: 0 0 8px; font-size: var(--fs-md, 1rem); }
  h4 { margin: 12px 0 4px; font-size: var(--fs-sm); }
  section { border: 1px solid var(--line); border-radius: var(--r); padding: 12px; }
  .crit, .alerts, .files { list-style: none; margin: 0; padding: 0; display: grid; gap: 6px; }
  .proof { margin: 6px 0 0; padding: 8px; background: var(--surface-2, rgba(127, 127, 127, 0.08)); border-radius: var(--r); overflow: auto; max-height: 220px; font-size: var(--fs-sm); white-space: pre-wrap; }
  .preview { height: 420px; border: 1px solid var(--line); border-radius: var(--r); overflow: hidden; }
  .axes { display: grid; grid-template-columns: repeat(auto-fit, minmax(160px, 1fr)); gap: 12px; margin-bottom: 12px; }
  .metrics { margin: 0 0 8px; }
  .err-line, .warn-line { margin: 8px var(--sp-4, 16px); }
  .faint { color: var(--faint, var(--muted)); }
  .small { font-size: var(--fs-sm); }
</style>
