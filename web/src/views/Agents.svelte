<script lang="ts">
  import { onDestroy } from 'svelte';
  import {
    agents, createAgent, updateAgent, deleteAgent, runAgent, stopAgent, agentRuns, cronPreview, projects, presets, message,
    type Agent, type AgentBody, type AgentRun, type AgentTrigger, type FeedLevel, type Project, type Preset,
  } from '../lib/api';
  import {
    Button, Input, Select, Textarea, Switch, Segmented, Dialog, Badge, StatusDot, EmptyState, Skeleton, ProgressBar,
    confirmDialog, toast, int,
  } from '../lib/ui';
  import type { DotKind, Tone } from '../lib/ui';
  import Page from '../lib/ui/Page.svelte';
  import { ahead, since, stamp, span, scheduleText } from '../lib/ui/format';
  import Plus from '@lucide/svelte/icons/plus';
  import Play from '@lucide/svelte/icons/play';
  import Square from '@lucide/svelte/icons/square';
  import Pencil from '@lucide/svelte/icons/pencil';
  import Trash2 from '@lucide/svelte/icons/trash-2';
  import Bot from '@lucide/svelte/icons/bot';
  import ArrowLeft from '@lucide/svelte/icons/arrow-left';
  import MessageSquare from '@lucide/svelte/icons/message-square';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import ShieldCheck from '@lucide/svelte/icons/shield-check';

  let { sub }: { sub: string } = $props();

  let list = $state<Agent[] | null>(null);
  let tree = $state<Project[]>([]);
  let presetList = $state<Preset[]>([]);
  let error = $state('');
  let form = $state<(AgentBody & { id: number | null; useSchedule: boolean; useFeed: boolean }) | null>(null);
  let formOpen = $state(false);
  let formError = $state('');
  let preview = $state<number[]>([]);
  let previewError = $state('');
  let history = $state<{ agent: Agent; runs: AgentRun[] } | null>(null);
  let openRuns = $state<Set<number>>(new Set());
  let launching = $state<Agent | null>(null);
  let launchOpen = $state(false);
  let launchCtx = $state('');

  const SOURCES: Record<string, string> = {
    webhook: 'webhooks', cron: 'crons', session: 'sessions', approval: 'approbations', agent: 'agents',
    download: 'téléchargements', engine: 'moteur', verify: 'vérifications', disk: 'disque',
  };
  const LEVEL: Record<FeedLevel, string> = { info: 'info', success: 'succès', warning: 'alerte', error: 'erreur' };
  const TRIGGER: Record<AgentTrigger, string> = {
    schedule: 'horaire', manual: 'à la main', webhook: 'webhook', feed: 'fil', agent: 'autre agent', session: 'session',
  };
  const statusLabel: Record<string, string> = { running: 'en cours', ok: 'réussi', failed: 'échec', skipped: 'sauté' };
  const statusTone: Record<string, Tone> = { running: 'ok', ok: 'ok', failed: 'danger', skipped: 'neutral' };
  const examples = [
    { s: 'every:60', l: 'toutes les heures' },
    { s: '0 8 * * *', l: 'chaque jour à 8 h' },
    { s: '0 7 * * 1-5', l: 'en semaine à 7 h' },
    { s: '0 18 * * 5', l: 'le vendredi à 18 h' },
  ];
  const mins = (s: number) => (s < 60 ? `${s} s` : `${Math.floor(s / 60)} min`);
  const duration = (r: { started_at: number; ended_at: number | null }) => (r.ended_at ? span(r.ended_at - r.started_at) : '');
  const kTok = (n: number) => (n >= 10000 ? `${Math.round(n / 1000)}k` : int(n));
  const presetName = (id: number | null | undefined) => (id == null ? 'Par défaut' : (presetList.find((p) => p.id === id)?.name ?? `preset ${id}`));

  function triggerList(a: Agent): string[] {
    const t: string[] = [];
    if (a.schedule) t.push(scheduleText(a.schedule));
    if (a.feed_filter) {
      const f = a.feed_filter;
      const src = f.sources.length ? f.sources.map((s) => SOURCES[s] ?? s).join(', ') : 'tout';
      t.push(`fil (${src}${f.min_level !== 'info' ? `, ${LEVEL[f.min_level]} ou plus` : ''}${f.contains ? `, contient « ${f.contains} »` : ''})`);
    }
    for (const h of a.webhooks) t.push(`webhook « ${h.name} »`);
    return t;
  }
  const triggers = (a: Agent) => {
    const t = triggerList(a);
    return t.length ? t.join(', ') : 'à la main ou par un autre agent';
  };

  const overBudget = (a: Agent) =>
    (a.budget_tokens > 0 && a.today.tokens >= a.budget_tokens) || (a.budget_gpu_min > 0 && a.today.gpu_s >= a.budget_gpu_min * 60);
  const dotOf = (a: Agent): DotKind =>
    !a.enabled ? 'off' : a.running ? 'ok' : overBudget(a) ? 'warn' : a.last?.status === 'failed' ? 'err' : 'ok';
  const stateOf = (a: Agent) =>
    !a.enabled ? 'désactivé' : a.running ? 'en cours' : overBudget(a) ? 'budget du jour atteint' : a.last?.status === 'failed' ? 'dernier départ en échec' : 'actif';

  async function load() {
    try {
      list = await agents();
      error = '';
      const want = history?.agent.id ?? (Number(sub) || null);
      if (want) {
        const a = list.find((x) => x.id === want);
        if (a) {
          const first = !history;
          history = { agent: a, runs: await agentRuns(a.id) };
          if (first && history.runs.length) openRuns = new Set([history.runs[0].id]);
        }
      }
    } catch (e) {
      error = message(e);
    }
  }
  load();
  projects().then((p) => (tree = p)).catch((e) => (error = message(e)));
  presets().then((p) => (presetList = p)).catch(() => {});
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

  const bodyOf = (a: Agent): AgentBody => ({
    name: a.name, topic_id: a.topic_id, preset_id: a.preset_id, role: a.role, schedule: a.schedule, tz: a.tz,
    feed_filter: a.feed_filter, approval: a.approval, timeout_s: a.timeout_s, budget_tokens: a.budget_tokens,
    budget_gpu_min: a.budget_gpu_min, notify: a.notify, enabled: a.enabled,
  });

  function edit(a: Agent | null) {
    formError = '';
    const first = tree[0]?.topics[0]?.id ?? 0;
    const b: AgentBody = a
      ? bodyOf(a)
      : {
          name: '', topic_id: first, preset_id: null, role: '', schedule: null, tz: 'Europe/Paris', feed_filter: null,
          approval: 'auto', timeout_s: 1800, budget_tokens: 0, budget_gpu_min: 0, notify: false, enabled: true,
        };
    form = {
      ...b,
      id: a?.id ?? null,
      useSchedule: !!b.schedule,
      useFeed: !!b.feed_filter,
      schedule: b.schedule ?? '0 8 * * *',
      feed_filter: b.feed_filter ?? { sources: ['webhook'], min_level: 'info', contains: '' },
    };
    formOpen = true;
  }
  function closeForm() {
    formOpen = false;
    form = null;
  }

  let previewTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const s = form?.useSchedule ? form.schedule : null;
    const tz = form?.tz;
    clearTimeout(previewTimer);
    if (!s) {
      preview = [];
      previewError = '';
      return;
    }
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

  function toggleSource(s: string) {
    if (!form?.feed_filter) return;
    const f = form.feed_filter;
    f.sources = f.sources.includes(s) ? f.sources.filter((x) => x !== s) : [...f.sources, s];
  }

  async function save(e: SubmitEvent) {
    e.preventDefault();
    if (!form) return;
    const { id, useSchedule, useFeed, ...rest } = form;
    const body: AgentBody = {
      ...rest,
      schedule: useSchedule ? rest.schedule : null,
      feed_filter: useFeed ? rest.feed_filter : null,
    };
    formError = '';
    try {
      if (id === null) await createAgent(body);
      else await updateAgent(id, body);
      toast(id === null ? 'Agent créé' : 'Agent enregistré', 'ok');
      closeForm();
      await load();
    } catch (err) {
      formError = message(err);
    }
  }

  async function remove(a: Agent) {
    if (!(await confirmDialog(`Supprimer l’agent « ${a.name} » et son historique ? Ses sessions restent.`, { title: 'Supprimer l’agent', confirm: 'Supprimer', danger: true }))) return;
    if (history?.agent.id === a.id) closeHistory();
    act(() => deleteAgent(a.id), 'Agent supprimé');
  }

  function launch(a: Agent) {
    launching = a;
    launchCtx = '';
    launchOpen = true;
  }
  function confirmLaunch(e: SubmitEvent) {
    e.preventDefault();
    const a = launching;
    if (!a) return;
    const ctx = launchCtx;
    launchOpen = false;
    launching = null;
    act(() => runAgent(a.id, ctx), `« ${a.name} » lancé`);
  }

  async function showHistory(a: Agent) {
    try {
      const runs = await agentRuns(a.id);
      history = { agent: a, runs };
      openRuns = new Set(runs.length ? [runs[0].id] : []);
      if (location.hash !== `#agents/${a.id}`) history_replace(`#agents/${a.id}`);
    } catch (e) {
      error = message(e);
    }
  }
  const history_replace = (h: string) => window.history.replaceState(null, '', h);
  function closeHistory() {
    history = null;
    history_replace('#agents');
  }
  function toggleRun(id: number) {
    const s = new Set(openRuns);
    if (s.has(id)) s.delete(id);
    else s.add(id);
    openRuns = s;
  }

  const toggle = (a: Agent) => act(() => updateAgent(a.id, { ...bodyOf(a), enabled: !a.enabled }));
  const sel = $derived(history?.agent ?? null);
</script>

<Page title="Agents">
  {#snippet meta()}{#if list?.length}<span class="num">{list.filter((a) => a.running).length ? `${list.filter((a) => a.running).length} en cours, ` : ''}{list.filter((a) => a.enabled).length} actif{list.filter((a) => a.enabled).length > 1 ? 's' : ''} sur {list.length}</span>{/if}{/snippet}
  {#snippet actions()}
    <Button variant="primary" icon={Plus} onclick={() => edit(null)} disabled={!tree.length}>Nouvel agent</Button>
  {/snippet}
  {#if error}<p class="err-line" role="alert">{error}</p>{/if}

  {#if list && list.length === 0}
    <div class="pad">
      <EmptyState icon={Bot} title="Aucun agent" text="Un agent est un rôle durable dans un topic. Il part sur un horaire, une notification du fil, un webhook, à la main ou à la demande d’un autre agent.">
        {#snippet action()}<Button variant="primary" icon={Plus} onclick={() => edit(null)} disabled={!tree.length}>Nouvel agent</Button>{/snippet}
      </EmptyState>
    </div>
  {:else if list}
    <div class="split" class:has-detail={!!sel}>
      <div class="master">
        <ul class="rows" aria-label="Agents">
          {#each list as a (a.id)}
            <li>
              <button class="row" class:on={sel?.id === a.id} class:off={!a.enabled} onclick={() => showHistory(a)} aria-current={sel?.id === a.id ? 'true' : undefined}>
                <StatusDot kind={dotOf(a)} label={stateOf(a)} />
                <span class="main">
                  <span class="t ell">{a.name}{#if a.approval === 'prudent'}<span class="pru" aria-label="prudent"><ShieldCheck size={14} /></span>{/if}</span>
                  <span class="sub ell">{triggers(a)}</span>
                </span>
                <span class="when">
                  {#if a.running}en cours{:else if a.last}{since(a.last.started_at)}{:else}jamais{/if}
                </span>
                <ChevronRight size={16} class="chev" />
              </button>
            </li>
          {/each}
        </ul>
        <p class="note">
          Chaque départ ouvre une session « Agent : nom » ; ce qui doit durer passe par la mémoire du topic. Les tours des agents
          passent après les tiens dans la file. Un autre agent peut en lancer un avec l’outil <code>agent_send</code>.
        </p>
      </div>

      <div class="detail">
        {#if sel && history}
          {@const a = sel}
          <div class="d-head">
            <Button variant="ghost" icon={ArrowLeft} label="Retour à la liste" class="back" onclick={closeHistory} />
            <div class="d-title">
              <h2>{a.name}</h2>
              <div class="badges">
                <Badge tone={dotOf(a) === 'err' ? 'danger' : dotOf(a) === 'warn' ? 'warn' : dotOf(a) === 'off' ? 'neutral' : 'ok'} dot>{stateOf(a)}</Badge>
                {#if a.approval === 'prudent'}<Badge title="Les commandes et fichiers sensibles attendent votre accord (1 h au plus)">prudent</Badge>{/if}
                <Badge>{a.topic ?? '?'}</Badge>
              </div>
            </div>
          </div>
          <div class="d-actions">
            {#if a.running}
              <Button variant="danger" icon={Square} onclick={() => act(() => stopAgent(a.id), 'Agent coupé')}>Couper</Button>
            {:else}
              <Button variant="primary" icon={Play} onclick={() => launch(a)}>Lancer</Button>
            {/if}
            <Button icon={Pencil} onclick={() => edit(a)}>Modifier</Button>
            {#if a.last?.session_id}<Button variant="ghost" icon={MessageSquare} href="#sessions/{a.last.session_id}">Session</Button>{/if}
            <span class="spacer"></span>
            <Switch checked={a.enabled} label="Activé" onchange={() => toggle(a)} />
            <Button variant="ghost" icon={Trash2} label="Supprimer" onclick={() => remove(a)} disabled={a.running} />
          </div>

          <div class="budget">
            <div class="b-item">
              <div class="b-top"><span>Jetons aujourd’hui</span><span class="num" class:over={a.budget_tokens > 0 && a.today.tokens >= a.budget_tokens}>{kTok(a.today.tokens)}{a.budget_tokens ? ` / ${kTok(a.budget_tokens)}` : ''}</span></div>
              {#if a.budget_tokens}<ProgressBar value={a.today.tokens} max={a.budget_tokens} tone={a.today.tokens >= a.budget_tokens ? 'danger' : 'accent'} label="Jetons du jour" />{:else}<span class="faint small">sans limite</span>{/if}
            </div>
            <div class="b-item">
              <div class="b-top"><span>GPU aujourd’hui</span><span class="num" class:over={a.budget_gpu_min > 0 && a.today.gpu_s >= a.budget_gpu_min * 60}>{mins(a.today.gpu_s)}{a.budget_gpu_min ? ` / ${a.budget_gpu_min} min` : ''}</span></div>
              {#if a.budget_gpu_min}<ProgressBar value={a.today.gpu_s} max={a.budget_gpu_min * 60} tone={a.today.gpu_s >= a.budget_gpu_min * 60 ? 'danger' : 'accent'} label="Minutes de GPU du jour" />{:else}<span class="faint small">sans limite</span>{/if}
            </div>
          </div>

          <dl class="kv">
            <dt>Départs</dt>
            <dd>
              {#each triggerList(a) as t}<div>{t}</div>{:else}<div>à la main ou par un autre agent</div>{/each}
              {#if a.enabled && a.next_run}<div class="faint">prochain : {stamp(a.next_run)}, {ahead(a.next_run).split(', ')[1] ?? ''}</div>{/if}
            </dd>
            <dt>Modèle</dt><dd>{presetName(a.preset_id)}</dd>
            <dt>Approbation</dt><dd>{a.approval === 'prudent' ? 'prudent : demande votre accord, 1 h au plus' : 'auto : agit seul dans sa sandbox'}</dd>
            <dt>Délai max</dt><dd>{span(a.timeout_s)}</dd>
            <dt>Résultat</dt><dd>{a.notify ? 'poussé même réussi' : 'poussé seulement en cas d’échec'}</dd>
            <dt>Rôle</dt><dd><pre class="body">{a.role}</pre></dd>
          </dl>

          <h3 class="sec">Départs récents <span class="faint num">{history.runs.length}</span></h3>
          {#if history.runs.length}
            <table class="runs">
              <thead>
                <tr><th>Statut</th><th>Début</th><th class="hide-s">Départ</th><th class="hide-s">Durée</th><th class="hide-s r">Jetons</th><th class="hide-s r">GPU</th><th><span class="sr-only">Session</span></th></tr>
              </thead>
              <tbody>
                {#each history.runs as r (r.id)}
                  {@const isOpen = openRuns.has(r.id)}
                  <tr class:open={isOpen}>
                    <td>
                      <button class="exp" aria-expanded={isOpen} onclick={() => toggleRun(r.id)}>
                        <ChevronRight size={16} class="chev2" />
                        <Badge tone={statusTone[r.status]}>{statusLabel[r.status]}</Badge>
                      </button>
                    </td>
                    <td class="num" title={stamp(r.started_at)}>{since(r.started_at)}</td>
                    <td class="hide-s">{TRIGGER[r.trigger]}{#if r.depth > 0}<span class="faint">, chaîne {r.depth}</span>{/if}</td>
                    <td class="num hide-s">{duration(r)}</td>
                    <td class="num hide-s r">{r.tokens ? kTok(r.tokens) : ''}</td>
                    <td class="num hide-s r">{r.tokens ? mins(r.gpu_s) : ''}</td>
                    <td class="r">{#if r.session_id}<a href="#sessions/{r.session_id}">session</a>{/if}</td>
                  </tr>
                  {#if isOpen}
                    <tr class="out">
                      <td colspan="7">
                        {#if r.context}<div class="ctx"><span class="faint">Contexte</span><p>{r.context}</p></div>{/if}
                        {#if r.output}<pre>{r.output}</pre>{:else}<p class="faint">Pas de sortie.</p>{/if}
                      </td>
                    </tr>
                  {/if}
                {/each}
              </tbody>
            </table>
          {:else}
            <p class="faint small">Pas encore de départ.</p>
          {/if}
        {:else}
          <div class="pick"><EmptyState icon={Bot} title="Choisissez un agent" text="Son rôle, son budget du jour et ses derniers départs s’affichent ici." /></div>
        {/if}
      </div>
    </div>
  {:else}
    <div class="pad"><Skeleton lines={4} /></div>
  {/if}
</Page>

<Dialog bind:open={launchOpen} title={launching ? `Lancer « ${launching.name} »` : 'Lancer'} width="520px" onclose={() => (launching = null)}>
  <form id="launch-form" class="form" onsubmit={confirmLaunch}>
    <Textarea label="Contexte en plus" hint="Facultatif : ajouté à son rôle pour ce départ." rows={4} bind:value={launchCtx} />
  </form>
  {#snippet actions()}
    <Button onclick={() => (launchOpen = false)}>Annuler</Button>
    <Button variant="primary" icon={Play} type="submit" form="launch-form">Lancer</Button>
  {/snippet}
</Dialog>

<Dialog bind:open={formOpen} title={form?.id == null ? 'Nouvel agent' : 'Modifier l’agent'} width="720px" onclose={() => (form = null)}>
  {#if form}
    <form id="agent-form" class="form" onsubmit={save}>
      <div class="three">
        <Input label="Nom" bind:value={form.name} maxlength={80} required />
        <Select label="Topic" bind:value={form.topic_id}>
          {#each tree as p}
            <optgroup label={p.name}>
              {#each p.topics as t}<option value={t.id}>{p.slug} / {t.slug}</option>{/each}
            </optgroup>
          {/each}
        </Select>
        <Select label="Modèle" bind:value={() => (form!.preset_id == null ? '' : String(form!.preset_id)), (v) => (form!.preset_id = v === '' ? null : Number(v))}
          options={[{ value: '', label: 'Par défaut' }, ...presetList.map((p) => ({ value: String(p.id), label: p.name }))]} />
      </div>
      <Textarea label="Rôle" rows={6} bind:value={form.role} required
        placeholder="Chaque matin, lis les ventes de la veille dans ventes.csv, compare à la semaine passée et écris un résumé de 5 lignes." />

      <section class="grp">
        <Switch bind:checked={form.useSchedule} label="Sur un horaire" />
        {#if form.useSchedule}
          <div class="two">
            <Input label="Quand" class="mono-in" bind:value={form.schedule} required error={previewError || undefined} />
            <Input label="Fuseau" bind:value={form.tz} />
          </div>
          <div class="chips" role="group" aria-label="Exemples d’horaires">
            {#each examples as ex}<button type="button" class="chip" class:on={form.schedule === ex.s} onclick={() => (form!.schedule = ex.s)}>{ex.l}</button>{/each}
          </div>
          {#if !previewError && preview.length}<p class="help">Prochains : <span class="num">{preview.map((t) => stamp(t)).join(', ')}</span></p>{/if}
        {/if}
      </section>

      <section class="grp">
        <Switch bind:checked={form.useFeed} label="Sur une notification du fil" />
        {#if form.useFeed && form.feed_filter}
          <div class="chips" role="group" aria-label="Sources">
            {#each Object.entries(SOURCES) as [s, l] (s)}
              <button type="button" class="chip" class:on={form.feed_filter.sources.includes(s)} aria-pressed={form.feed_filter.sources.includes(s)} onclick={() => toggleSource(s)}>{l}</button>
            {/each}
          </div>
          <div class="two">
            <Select label="Niveau minimal" bind:value={form.feed_filter.min_level} options={Object.entries(LEVEL).map(([v, l]) => ({ value: v as FeedLevel, label: l }))} />
            <Input label="Contient" bind:value={form.feed_filter.contains} maxlength={200} placeholder="vente, échec, main…" />
          </div>
          <p class="help">Aucune source choisie = toutes. Une minute au moins entre deux départs par le fil ; un agent ne part pas sur ses propres notifications.</p>
        {/if}
      </section>

      <div class="field">
        <span class="lbl">Approbation</span>
        <Segmented label="Approbation" bind:value={form.approval} options={[{ value: 'auto', label: 'Auto' }, { value: 'prudent', label: 'Prudent' }]} />
        <span class="help">{form.approval === 'auto' ? 'Agit seul dans sa sandbox.' : 'Demande votre accord (fil et push) pour les commandes et fichiers sensibles, 1 h au plus.'}</span>
      </div>
      <div class="three">
        <Input label="Délai max" unit="s" type="number" min={60} max={7200} bind:value={form.timeout_s} />
        <Input label="Jetons par jour" type="number" min={0} step={1000} bind:value={form.budget_tokens} />
        <Input label="GPU par jour" unit="min" type="number" min={0} max={1440} bind:value={form.budget_gpu_min} />
      </div>
      <p class="help">0 = sans limite. Budget atteint : plus de départ jusqu’à minuit, et une notification.</p>
      <Switch bind:checked={form.notify} label="Pousser le résultat même réussi (les échecs partent toujours)" />
      <Switch bind:checked={form.enabled} label="Activé" />
      {#if formError}<p class="ferr" role="alert">{formError}</p>{/if}
    </form>
  {/if}
  {#snippet actions()}
    <Button onclick={closeForm}>Annuler</Button>
    <Button variant="primary" type="submit" form="agent-form">{form?.id == null ? 'Créer l’agent' : 'Enregistrer'}</Button>
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
  .pru { display: inline-flex; vertical-align: -2px; margin-left: var(--s2); color: var(--faint); }
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

  .budget { display: grid; grid-template-columns: 1fr 1fr; gap: var(--s5); }
  .b-item { display: flex; flex-direction: column; gap: var(--s2); }
  .b-top { display: flex; justify-content: space-between; gap: var(--s3); font-size: var(--fs-sm); color: var(--muted); }
  .b-top .num { color: var(--text); }
  .b-top .over { color: var(--danger); }

  .kv { display: grid; grid-template-columns: 140px minmax(0, 1fr); gap: var(--s3) var(--s4); margin: 0; font-size: var(--fs-sm); padding-top: var(--s4); border-top: 1px solid var(--line); }
  .kv dt { color: var(--muted); }
  .kv dd { margin: 0; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .body { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; font-family: var(--font); font-size: var(--fs-sm); line-height: 1.5; background: var(--panel); border: 1px solid var(--line); border-radius: var(--r); padding: var(--s3); max-height: 240px; overflow: auto; }
  .sec { margin: var(--s2) 0 0; font-size: var(--fs-md); font-weight: 600; display: flex; gap: var(--s2); align-items: baseline; }

  .runs { width: 100%; border-collapse: collapse; font-size: var(--fs-sm); }
  .runs th { text-align: left; color: var(--muted); font-weight: 400; padding: var(--s2) var(--s3) var(--s2) 0; border-bottom: 1px solid var(--line); }
  .runs td { padding: var(--s2) var(--s3) var(--s2) 0; border-bottom: 1px solid var(--line); vertical-align: middle; }
  .runs tbody tr:hover { background: none; }
  .runs tr.open td { border-bottom-color: transparent; }
  .runs .r { text-align: right; }
  .runs td:last-child, .runs th:last-child { padding-right: 0; }
  .runs a { color: var(--muted); text-decoration: none; }
  .runs a:hover { color: var(--accent); }
  .exp { all: unset; cursor: pointer; display: inline-flex; align-items: center; gap: var(--s1); border-radius: var(--r-sm); }
  .exp:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .exp :global(.chev2) { color: var(--faint); transition: transform 0.12s; }
  .exp[aria-expanded='true'] :global(.chev2) { transform: rotate(90deg); }
  .out td { padding-top: 0; }
  .out pre { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; max-height: 320px; overflow: auto; background: var(--panel); border: 1px solid var(--line); border-radius: var(--r); padding: var(--s3); }
  .out p { margin: 0 0 var(--s2); }
  .ctx { margin-bottom: var(--s2); }
  .ctx p { margin: 2px 0 0; white-space: pre-wrap; overflow-wrap: anywhere; color: var(--muted); }
  .faint { color: var(--faint); }
  .small { font-size: var(--fs-sm); }

  .form { display: flex; flex-direction: column; gap: var(--s4); }
  .two { display: grid; grid-template-columns: 1.4fr 1fr; gap: var(--s3); }
  .three { display: grid; grid-template-columns: 1fr 1fr 1fr; gap: var(--s3); }
  .grp { display: flex; flex-direction: column; gap: var(--s3); padding: var(--s3) 0 0; border-top: 1px solid var(--line); }
  .form :global(.mono-in input) { font-family: var(--mono); font-size: var(--fs-code); }
  .form > :global(.sw), .grp > :global(.sw) { align-self: flex-start; }
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
    .row.on { background: none; border-left-color: transparent; }
    .row :global(.chev) { display: block; }
    .detail { padding: var(--s3) var(--s4) var(--s5); }
    .d-head :global(.back) { display: inline-flex; margin-left: calc(-1 * var(--s2)); }
    .d-title h2 { font-size: var(--fs-lg); }
    .d-actions :global(.btn) { height: var(--touch); }
    .budget { grid-template-columns: minmax(0, 1fr); gap: var(--s3); }
    .kv { grid-template-columns: minmax(0, 1fr); gap: var(--s1); }
    .kv dd { margin-bottom: var(--s2); }
    .hide-s { display: none; }
    .exp { min-height: var(--touch); }
    .two, .three { grid-template-columns: minmax(0, 1fr); }
    .chip { padding: var(--s2) var(--s3); }
  }
</style>
