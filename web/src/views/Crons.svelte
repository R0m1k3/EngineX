<script lang="ts">
  import { onDestroy } from 'svelte';
  import {
    crons, createCron, updateCron, deleteCron, runCron, cronRuns, cronPreview, projects, presets, message,
    type Cron, type CronBody, type CronRun, type Project, type Preset,
  } from '../lib/api';
  import {
    Button, Input, Select, Textarea, Switch, Segmented, Dialog, Badge, StatusDot, EmptyState, Skeleton,
    confirmDialog, toast,
  } from '../lib/ui';
  import type { DotKind, Tone } from '../lib/ui';
  import Page from '../lib/ui/Page.svelte';
  import { ahead, since, stamp, span, scheduleText } from '../lib/ui/format';
  import Plus from '@lucide/svelte/icons/plus';
  import Play from '@lucide/svelte/icons/play';
  import Pencil from '@lucide/svelte/icons/pencil';
  import Trash2 from '@lucide/svelte/icons/trash-2';
  import Clock from '@lucide/svelte/icons/clock';
  import ArrowLeft from '@lucide/svelte/icons/arrow-left';
  import MessageSquare from '@lucide/svelte/icons/message-square';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';

  let list = $state<Cron[] | null>(null);
  let tree = $state<Project[]>([]);
  let presetList = $state<Preset[]>([]);
  presets().then((p) => (presetList = p)).catch(() => {});
  let error = $state('');
  let form = $state<(CronBody & { id: number | null }) | null>(null);
  let formOpen = $state(false);
  let formError = $state('');
  let preview = $state<number[]>([]);
  let previewError = $state('');
  let history = $state<{ cron: Cron; runs: CronRun[] } | null>(null);
  /** Exécutions dépliées (sortie visible). */
  let openRuns = $state<Set<number>>(new Set());

  const examples = [
    { s: 'every:30', l: 'toutes les 30 min' },
    { s: '0 9 * * *', l: 'chaque jour à 9 h' },
    { s: '0 8 * * 1-5', l: 'en semaine à 8 h' },
    { s: '0 7 * * 1', l: 'le lundi à 7 h' },
  ];
  const statusLabel: Record<string, string> = { running: 'en cours', ok: 'réussi', failed: 'échec', skipped: 'sauté' };
  const statusTone: Record<string, Tone> = { running: 'ok', ok: 'ok', failed: 'danger', skipped: 'neutral' };
  const duration = (r: { started_at: number; ended_at: number | null }) => (r.ended_at ? span(r.ended_at - r.started_at) : '');
  const presetName = (id: number | null | undefined) => (id == null ? 'Par défaut' : (presetList.find((p) => p.id === id)?.name ?? `preset ${id}`));
  const dotOf = (c: Cron): DotKind => (!c.enabled ? 'off' : c.running ? 'ok' : c.last?.status === 'failed' ? 'err' : 'ok');
  const stateOf = (c: Cron) => (!c.enabled ? 'désactivé' : c.running ? 'en cours' : c.last?.status === 'failed' ? 'dernier départ en échec' : 'actif');

  async function load() {
    try {
      list = await crons();
      error = '';
      if (history) {
        const c = list.find((x) => x.id === history!.cron.id);
        if (c) history = { cron: c, runs: await cronRuns(c.id) };
      }
    } catch (e) {
      error = message(e);
    }
  }
  load();
  projects().then((p) => (tree = p)).catch((e) => (error = message(e)));
  const timer = setInterval(() => {
    if (!form && document.visibilityState === 'visible') load();
  }, 5000);
  onDestroy(() => clearInterval(timer));

  async function act(f: () => Promise<unknown>, done?: string) {
    error = '';
    try {
      await f();
      if (done) toast(done, 'ok');
    } catch (e) {
      error = message(e);
      toast(error, 'error');
    }
    await load();
  }

  function edit(c: Cron | null) {
    formError = '';
    const first = tree[0]?.topics[0]?.id ?? 0;
    form = c
      ? { id: c.id, name: c.name, topic_id: c.topic_id, kind: c.kind, body: c.body, schedule: c.schedule, tz: c.tz, timeout_s: c.timeout_s, enabled: c.enabled, preset_id: c.preset_id }
      : { id: null, name: '', topic_id: first, kind: 'prompt', body: '', schedule: '0 9 * * *', tz: 'Europe/Paris', timeout_s: 1800, enabled: true, preset_id: null };
    formOpen = true;
  }
  function closeForm() {
    formOpen = false;
    form = null;
  }

  // Aperçu des 3 prochaines exécutions pendant la saisie.
  let previewTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const s = form?.schedule;
    const tz = form?.tz;
    clearTimeout(previewTimer);
    if (!s) return;
    previewTimer = setTimeout(async () => {
      try {
        preview = (await cronPreview(s, tz)).next;
        previewError = '';
      } catch (e) {
        preview = [];
        previewError = message(e);
      }
    }, 300);
  });

  async function save(e: SubmitEvent) {
    e.preventDefault();
    if (!form) return;
    const { id, ...body } = form;
    formError = '';
    try {
      if (id === null) await createCron(body);
      else await updateCron(id, body);
      toast(id === null ? 'Cron créé' : 'Cron enregistré', 'ok');
      closeForm();
      await load();
    } catch (err) {
      formError = message(err);
    }
  }

  async function remove(c: Cron) {
    if (!(await confirmDialog(`Supprimer le cron « ${c.name} » et son historique ? Sa session reste.`, { title: 'Supprimer le cron', confirm: 'Supprimer', danger: true }))) return;
    if (history?.cron.id === c.id) history = null;
    act(() => deleteCron(c.id), 'Cron supprimé');
  }

  async function showHistory(c: Cron) {
    try {
      const runs = await cronRuns(c.id);
      history = { cron: c, runs };
      openRuns = new Set(runs.length ? [runs[0].id] : []);
    } catch (e) {
      error = message(e);
    }
  }
  function toggleRun(id: number) {
    const s = new Set(openRuns);
    if (s.has(id)) s.delete(id);
    else s.add(id);
    openRuns = s;
  }

  const toggle = (c: Cron) =>
    act(() => updateCron(c.id, { name: c.name, topic_id: c.topic_id, kind: c.kind, body: c.body, schedule: c.schedule, tz: c.tz, timeout_s: c.timeout_s, enabled: !c.enabled, preset_id: c.preset_id }));

  const sel = $derived(history?.cron ?? null);
</script>

<Page title="Crons">
  {#snippet meta()}{#if list?.length}<span class="num">{list.filter((c) => c.enabled).length} actif{list.filter((c) => c.enabled).length > 1 ? 's' : ''} sur {list.length}</span>{/if}{/snippet}
  {#snippet actions()}
    <Button variant="primary" icon={Plus} onclick={() => edit(null)} disabled={!tree.length}>Nouveau cron</Button>
  {/snippet}
  {#if error}<p class="err-line" role="alert">{error}</p>{/if}

  {#if list && list.length === 0}
    <div class="pad">
      <EmptyState icon={Clock} title="Aucun cron" text="Un cron lance à heure fixe une consigne à l’IA (dans la session « Cron : nom » du topic) ou une commande dans la sandbox.">
        {#snippet action()}<Button variant="primary" icon={Plus} onclick={() => edit(null)} disabled={!tree.length}>Nouveau cron</Button>{/snippet}
      </EmptyState>
    </div>
  {:else if list}
    <div class="split" class:has-detail={!!sel}>
      <div class="master">
        <ul class="rows" aria-label="Crons">
          {#each list as c (c.id)}
            <li>
              <button class="row" class:on={sel?.id === c.id} class:off={!c.enabled} onclick={() => showHistory(c)} aria-current={sel?.id === c.id ? 'true' : undefined}>
                <StatusDot kind={dotOf(c)} label={stateOf(c)} />
                <span class="main">
                  <span class="t ell">{c.name}</span>
                  <span class="sub ell">{c.kind === 'prompt' ? 'consigne' : 'commande'}, {scheduleText(c.schedule)}</span>
                </span>
                <span class="when">{c.running ? 'en cours' : c.enabled ? ahead(c.next_run) : 'désactivé'}</span>
                <ChevronRight size={16} class="chev" />
              </button>
            </li>
          {/each}
        </ul>
        <p class="note">
          Une exécution à la fois par cron ; après un arrêt d’EngineX, une seule exécution de rattrapage.
        </p>
      </div>

      <div class="detail">
        {#if sel && history}
          {@const c = sel}
          <div class="d-head">
            <Button variant="ghost" icon={ArrowLeft} label="Retour à la liste" class="back" onclick={() => (history = null)} />
            <div class="d-title">
              <h2>{c.name}</h2>
              <div class="badges">
                <Badge>{c.kind === 'prompt' ? 'consigne' : 'commande'}</Badge>
                <Badge tone={!c.enabled ? 'neutral' : c.last?.status === 'failed' && !c.running ? 'danger' : 'ok'} dot>{stateOf(c)}</Badge>
              </div>
            </div>
          </div>
          <div class="d-actions">
            <Button variant="primary" icon={Play} onclick={() => act(() => runCron(c.id), 'Cron lancé')} disabled={c.running}>Lancer</Button>
            <Button icon={Pencil} onclick={() => edit(c)}>Modifier</Button>
            {#if c.session_id}<Button variant="ghost" icon={MessageSquare} href="#sessions/{c.session_id}">Session</Button>{/if}
            <span class="spacer"></span>
            <Switch checked={c.enabled} label="Activé" onchange={() => toggle(c)} />
            <Button variant="ghost" icon={Trash2} label="Supprimer" onclick={() => remove(c)} />
          </div>
          <dl class="kv">
            <dt>Quand</dt><dd>{scheduleText(c.schedule)} <code class="raw">{c.schedule}</code> <span class="faint">({c.tz})</span></dd>
            <dt>Prochaine</dt><dd>{c.enabled ? (c.next_run ? `${stamp(c.next_run)}, ${ahead(c.next_run).split(', ')[1] ?? ''}` : '—') : 'désactivé'}</dd>
            <dt>Topic</dt><dd>{c.topic ?? '?'}</dd>
            {#if c.kind === 'prompt'}<dt>Modèle</dt><dd>{presetName(c.preset_id)}</dd>{/if}
            <dt>Délai max</dt><dd>{span(c.timeout_s)}</dd>
            <dt>{c.kind === 'prompt' ? 'Consigne' : 'Commande'}</dt>
            <dd><pre class="body" class:mono={c.kind === 'shell'}>{c.body}</pre></dd>
          </dl>

          <h3 class="sec">Exécutions <span class="faint num">{history.runs.length}</span></h3>
          {#if history.runs.length}
            <table class="runs">
              <thead><tr><th>Statut</th><th>Début</th><th class="hide-s">Départ</th><th class="hide-s">Durée</th><th><span class="sr-only">Session</span></th></tr></thead>
              <tbody>
                {#each history.runs as r (r.id)}
                  {@const isOpen = openRuns.has(r.id)}
                  <tr class="run" class:open={isOpen}>
                    <td>
                      <button class="exp" aria-expanded={isOpen} onclick={() => toggleRun(r.id)}>
                        <ChevronRight size={16} class="chev2" />
                        <Badge tone={statusTone[r.status]}>{statusLabel[r.status]}</Badge>
                      </button>
                    </td>
                    <td class="num" title={stamp(r.started_at)}>{since(r.started_at)}</td>
                    <td class="hide-s">{r.trigger === 'manual' ? 'à la main' : 'planifié'}</td>
                    <td class="num hide-s">{duration(r)}</td>
                    <td class="r">{#if r.session_id}<a href="#sessions/{r.session_id}">session</a>{/if}</td>
                  </tr>
                  {#if isOpen}
                    <tr class="out"><td colspan="5">{#if r.output}<pre>{r.output}</pre>{:else}<p class="faint">Pas de sortie.</p>{/if}</td></tr>
                  {/if}
                {/each}
              </tbody>
            </table>
          {:else}
            <p class="faint small">Pas encore d’exécution.</p>
          {/if}
        {:else}
          <div class="pick"><EmptyState icon={Clock} title="Choisissez un cron" text="Son réglage et l’historique de ses exécutions s’affichent ici." /></div>
        {/if}
      </div>
    </div>
  {:else}
    <div class="pad"><Skeleton lines={4} /></div>
  {/if}
</Page>

<Dialog bind:open={formOpen} title={form?.id == null ? 'Nouveau cron' : 'Modifier le cron'} width="680px" onclose={() => (form = null)}>
  {#if form}
    <form id="cron-form" class="form" onsubmit={save}>
      <div class="two">
        <Input label="Nom" bind:value={form.name} maxlength={80} required />
        <Select label="Topic" bind:value={form.topic_id}>
          {#each tree as p}
            <optgroup label={p.name}>
              {#each p.topics as t}<option value={t.id}>{p.slug} / {t.slug}</option>{/each}
            </optgroup>
          {/each}
        </Select>
      </div>
      <div class="field">
        <span class="lbl">Ce qui part</span>
        <Segmented label="Type de cron" bind:value={form.kind} options={[{ value: 'prompt', label: 'Consigne à l’IA' }, { value: 'shell', label: 'Commande shell' }]} />
      </div>
      <Textarea label={form.kind === 'prompt' ? 'Consigne' : 'Commande (bash)'} rows={5} mono={form.kind === 'shell'} bind:value={form.body} required
        placeholder={form.kind === 'prompt' ? 'Fais le point sur les tests qui échouent et propose un correctif.' : 'df -h /workspace'} />
      <div class="when-blk">
        <Input label="Quand" class="mono-in" bind:value={form.schedule} required error={previewError || undefined} />
        <div class="chips" role="group" aria-label="Exemples d’horaires">
          {#each examples as ex}<button type="button" class="chip" class:on={form.schedule === ex.s} onclick={() => (form!.schedule = ex.s)}>{ex.l}</button>{/each}
        </div>
        <p class="help">
          Cron à 5 champs (minute heure jour mois jour-semaine) ou <code>every:N</code> (toutes les N minutes).
          {#if !previewError && preview.length}<br />Prochaines : <span class="num">{preview.map((t) => stamp(t)).join(', ')}</span>{/if}
        </p>
      </div>
      <div class="two">
        <Input label="Fuseau" bind:value={form.tz} />
        <Input label="Délai max" unit="s" type="number" min={10} max={7200} bind:value={form.timeout_s} />
      </div>
      {#if form.kind === 'prompt'}
        <Select label="Modèle" hint="Un autre modèle que celui chargé est chargé pour ce cron, après vos tours." bind:value={() => (form!.preset_id == null ? '' : String(form!.preset_id)), (v) => (form!.preset_id = v === '' ? null : Number(v))}
          options={[{ value: '', label: 'Par défaut' }, ...presetList.map((p) => ({ value: String(p.id), label: p.name }))]} />
      {/if}
      <Switch bind:checked={form.enabled} label="Activé" />
      {#if formError}<p class="ferr" role="alert">{formError}</p>{/if}
    </form>
  {/if}
  {#snippet actions()}
    <Button onclick={closeForm}>Annuler</Button>
    <Button variant="primary" type="submit" form="cron-form">{form?.id == null ? 'Créer le cron' : 'Enregistrer'}</Button>
  {/snippet}
</Dialog>

<style>
  .err-line { margin: 0; padding: var(--s2) var(--s5); background: var(--danger-soft); color: var(--danger); font-size: var(--fs-sm); border-bottom: 1px solid var(--line); }
  .pad { padding: var(--s5); }

  .split { display: grid; grid-template-columns: minmax(320px, 400px) minmax(0, 1fr); flex: 1; }
  .master { background: var(--panel); border-right: 1px solid var(--line); display: flex; flex-direction: column; }
  .rows { list-style: none; margin: 0; padding: 0; }
  .rows li { border-bottom: 1px solid var(--line); }
  .row { all: unset; box-sizing: border-box; width: 100%; display: flex; align-items: center; gap: var(--s3); padding: var(--s3) var(--s4); min-height: 60px; cursor: pointer; border-left: 2px solid transparent; }
  .row:hover { background: var(--raised); }
  .row:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  .row.on { background: var(--accent-soft); border-left-color: var(--accent); }
  .row.off .t { color: var(--muted); }
  .main { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .t { font-size: var(--fs-md); color: var(--text); }
  .sub { font-size: var(--fs-sm); color: var(--muted); }
  .ell { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; display: block; }
  .when { font-size: var(--fs-sm); color: var(--muted); white-space: nowrap; font-variant-numeric: tabular-nums; }
  .row :global(.chev) { color: var(--faint); display: none; flex: none; }
  .note { margin: 0; padding: var(--s4); color: var(--muted); font-size: var(--fs-sm); }

  .detail { min-width: 0; padding: var(--s5) var(--s6); display: flex; flex-direction: column; gap: var(--s4); max-width: 960px; }
  .pick { padding-top: var(--s4); }
  .d-head { display: flex; align-items: flex-start; gap: var(--s2); }
  .d-head :global(.back) { display: none; }
  .d-title { display: flex; flex-direction: column; gap: var(--s2); min-width: 0; }
  .d-title h2 { margin: 0; font-size: var(--fs-xl); font-weight: 600; line-height: 1.25; overflow-wrap: anywhere; }
  .badges { display: flex; gap: var(--s2); flex-wrap: wrap; }
  .d-actions { display: flex; align-items: center; gap: var(--s2); flex-wrap: wrap; padding-bottom: var(--s4); border-bottom: 1px solid var(--line); }
  .spacer { flex: 1; }
  .kv { display: grid; grid-template-columns: 140px minmax(0, 1fr); gap: var(--s3) var(--s4); margin: 0; font-size: var(--fs-sm); }
  .kv dt { color: var(--muted); }
  .kv dd { margin: 0; min-width: 0; }
  .raw { color: var(--muted); margin-left: var(--s2); }
  .body { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; font-family: var(--font); font-size: var(--fs-sm); background: var(--panel); border: 1px solid var(--line); border-radius: var(--r); padding: var(--s3); max-height: 240px; overflow: auto; }
  .body.mono { font-family: var(--mono); font-size: var(--fs-code); }
  .sec { margin: var(--s2) 0 0; font-size: var(--fs-md); font-weight: 600; display: flex; gap: var(--s2); align-items: baseline; }

  .runs { width: 100%; border-collapse: collapse; font-size: var(--fs-sm); }
  .runs th { text-align: left; color: var(--muted); font-weight: 400; padding: var(--s2) var(--s3) var(--s2) 0; border-bottom: 1px solid var(--line); }
  .runs td { padding: var(--s2) var(--s3) var(--s2) 0; border-bottom: 1px solid var(--line); vertical-align: middle; }
  .runs tbody tr:hover { background: none; }
  .runs tr.open td { border-bottom-color: transparent; }
  .runs .r { text-align: right; padding-right: 0; }
  .runs a { color: var(--muted); text-decoration: none; }
  .runs a:hover { color: var(--accent); }
  .exp { all: unset; cursor: pointer; display: inline-flex; align-items: center; gap: var(--s1); border-radius: var(--r-sm); }
  .exp:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .exp :global(.chev2) { color: var(--faint); transition: transform 0.12s; }
  .exp[aria-expanded='true'] :global(.chev2) { transform: rotate(90deg); }
  .out td { padding-top: 0; }
  .out pre { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; max-height: 320px; overflow: auto; background: var(--panel); border: 1px solid var(--line); border-radius: var(--r); padding: var(--s3); }
  .out p { margin: 0 0 var(--s2); }
  .faint { color: var(--faint); }
  .small { font-size: var(--fs-sm); }

  .form { display: flex; flex-direction: column; gap: var(--s4); }
  .two { display: grid; grid-template-columns: 1fr 1fr; gap: var(--s3); }
  .when-blk { display: flex; flex-direction: column; gap: var(--s2); }
  .form > :global(.sw) { align-self: flex-start; }
  .form :global(.mono-in input) { font-family: var(--mono); font-size: var(--fs-code); }
  .field { display: flex; flex-direction: column; gap: var(--s1); align-items: flex-start; }
  .lbl { color: var(--muted); font-size: var(--fs-sm); }
  .chips { display: flex; flex-wrap: wrap; gap: var(--s2); }
  .chip { all: unset; cursor: pointer; font-size: var(--fs-sm); color: var(--muted); padding: 2px var(--s2); border: 1px solid var(--line-strong); border-radius: var(--r); }
  .chip:hover { color: var(--text); background: var(--raised); }
  .chip.on { color: var(--accent); border-color: var(--accent); background: var(--accent-soft); }
  .chip:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
  .help { margin: 0; color: var(--muted); font-size: var(--fs-sm); }
  .ferr { margin: 0; color: var(--danger); font-size: var(--fs-sm); }

  @media (max-width: 767px) {
    .split { grid-template-columns: minmax(0, 1fr); min-height: 0; }
    .master { border-right: 0; background: var(--bg); }
    .has-detail .master { display: none; }
    .split:not(.has-detail) .detail { display: none; }
    .row { padding: var(--s3) var(--s4); }
    .row.on { background: none; border-left-color: transparent; }
    .row :global(.chev) { display: block; }
    .detail { padding: var(--s3) var(--s4) var(--s5); }
    .d-head :global(.back) { display: inline-flex; margin-left: calc(-1 * var(--s2)); }
    .d-title h2 { font-size: var(--fs-lg); }
    .d-actions :global(.btn) { height: var(--touch); }
    .kv { grid-template-columns: minmax(0, 1fr); gap: var(--s1); }
    .kv dd { margin-bottom: var(--s2); }
    .hide-s { display: none; }
    .exp { min-height: var(--touch); }
    .two { grid-template-columns: minmax(0, 1fr); }
    .chip { padding: var(--s2) var(--s3); }
  }
</style>
