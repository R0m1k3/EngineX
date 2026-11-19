<script lang="ts">
  import { onDestroy } from 'svelte';
  import {
    agents, createAgent, updateAgent, deleteAgent, runAgent, stopAgent, agentRuns, cronPreview, projects, presets, message,
    type Agent, type AgentBody, type AgentRun, type AgentTrigger, type FeedLevel, type Project, type Preset,
  } from '../lib/api';

  let { sub }: { sub: string } = $props();

  let list = $state<Agent[] | null>(null);
  let tree = $state<Project[]>([]);
  let presetList = $state<Preset[]>([]);
  let error = $state('');
  let form = $state<(AgentBody & { id: number | null; useSchedule: boolean; useFeed: boolean }) | null>(null);
  let formError = $state('');
  let preview = $state<number[]>([]);
  let previewError = $state('');
  let history = $state<{ agent: Agent; runs: AgentRun[] } | null>(null);

  const SOURCES: Record<string, string> = {
    webhook: 'webhooks', cron: 'crons', session: 'sessions', approval: 'approbations', agent: 'agents',
    download: 'téléchargements', engine: 'moteur', verify: 'vérifications', disk: 'disque',
  };
  const LEVEL: Record<FeedLevel, string> = { info: 'info', success: 'succès', warning: 'alerte', error: 'erreur' };
  const TRIGGER: Record<AgentTrigger, string> = {
    schedule: 'horaire', manual: 'à la main', webhook: 'webhook', feed: 'fil', agent: 'autre agent', session: 'session',
  };
  const statusLabel: Record<string, string> = { running: 'en cours', ok: 'réussi', failed: 'échec', skipped: 'sauté' };
  const examples = [
    { s: 'every:60', l: 'toutes les heures' },
    { s: '0 8 * * *', l: 'chaque jour à 8 h' },
    { s: '0 7 * * 1-5', l: 'en semaine à 7 h' },
    { s: '0 18 * * 5', l: 'le vendredi à 18 h' },
  ];
  const date = (t: number | null) =>
    t ? new Date(t * 1000).toLocaleString('fr-FR', { dateStyle: 'short', timeStyle: 'short' }) : '—';
  const mins = (s: number) => (s < 60 ? `${s} s` : `${Math.floor(s / 60)} min`);
  const duration = (r: { started_at: number; ended_at: number | null }) => (r.ended_at ? mins(r.ended_at - r.started_at) : '');
  const kTok = (n: number) => (n >= 10000 ? `${Math.round(n / 1000)}k` : String(n));

  function triggers(a: Agent): string {
    const t: string[] = [];
    if (a.schedule) {
      const m = /^every:(\d+)$/.exec(a.schedule);
      t.push(m ? `toutes les ${m[1]} min` : (examples.find((e) => e.s === a.schedule)?.l ?? a.schedule));
    }
    if (a.feed_filter) {
      const f = a.feed_filter;
      const src = f.sources.length ? f.sources.map((s) => SOURCES[s] ?? s).join(', ') : 'tout';
      t.push(`fil (${src}${f.min_level !== 'info' ? ` ≥ ${LEVEL[f.min_level]}` : ''}${f.contains ? ` « ${f.contains} »` : ''})`);
    }
    for (const h of a.webhooks) t.push(`webhook « ${h.name} »`);
    return t.length ? t.join(' · ') : 'à la main ou par un autre agent';
  }

  function budget(a: Agent): string {
    const p: string[] = [];
    p.push(a.budget_tokens ? `${kTok(a.today.tokens)} / ${kTok(a.budget_tokens)} jetons` : `${kTok(a.today.tokens)} jetons`);
    p.push(a.budget_gpu_min ? `${mins(a.today.gpu_s)} / ${a.budget_gpu_min} min GPU` : `${mins(a.today.gpu_s)} GPU`);
    return p.join(' · ');
  }
  const overBudget = (a: Agent) =>
    (a.budget_tokens > 0 && a.today.tokens >= a.budget_tokens) || (a.budget_gpu_min > 0 && a.today.gpu_s >= a.budget_gpu_min * 60);

  async function load() {
    try {
      list = await agents();
      error = '';
      const want = history?.agent.id ?? (Number(sub) || null);
      if (want) {
        const a = list.find((x) => x.id === want);
        if (a) history = { agent: a, runs: await agentRuns(a.id) };
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

  async function act(f: () => Promise<unknown>) {
    error = '';
    try {
      await f();
    } catch (e) {
      error = message(e);
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
      form = null;
      await load();
    } catch (err) {
      formError = message(err);
    }
  }

  function remove(a: Agent) {
    if (!confirm(`Supprimer l’agent « ${a.name} » et son historique ? Ses sessions restent.`)) return;
    if (history?.agent.id === a.id) history = null;
    act(() => deleteAgent(a.id));
  }

  function launch(a: Agent) {
    const ctx = prompt(`Lancer « ${a.name} ». Contexte en plus (facultatif) :`, '');
    if (ctx === null) return;
    act(() => runAgent(a.id, ctx));
  }

  async function showHistory(a: Agent) {
    try {
      history = { agent: a, runs: await agentRuns(a.id) };
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

  const toggle = (a: Agent) => act(() => updateAgent(a.id, { ...bodyOf(a), enabled: !a.enabled }));
</script>

<section>
  <div class="row head">
    <h1>Agents</h1>
    <span class="grow"></span>
    <button onclick={() => edit(null)} disabled={!tree.length}>Nouvel agent</button>
  </div>
  <p class="muted small">
    Un agent est un rôle durable dans un topic. Il part sur un horaire, une notification du fil, un webhook, à la main ou à la
    demande d’un autre agent (outil <code>agent_send</code>). Chaque départ ouvre une session « Agent : nom » ; ce qui doit
    durer passe par la mémoire du topic. Ses tours passent après les vôtres dans la file.
  </p>
  {#if error}<p class="error">{error}</p>{/if}

  {#if list && list.length === 0}
    <p class="muted">Aucun agent.</p>
  {:else if list}
    <table class="agents">
      <thead>
        <tr><th>Agent</th><th>Départs</th><th>Aujourd’hui</th><th>Dernier</th><th></th></tr>
      </thead>
      <tbody>
        {#each list as a (a.id)}
          <tr class:off={!a.enabled}>
            <td>
              <button class="link strong" onclick={() => showHistory(a)}>{a.name}</button>
              {#if a.approval === 'prudent'}<span class="badge" title="Les commandes et fichiers sensibles attendent votre accord (1 h au plus)">prudent</span>{/if}
              <div class="muted small">{a.topic ?? '?'}</div>
            </td>
            <td class="small">
              {triggers(a)}
              {#if a.enabled && a.next_run}<div class="muted">prochain : {date(a.next_run)}</div>{/if}
            </td>
            <td class="small" class:error={overBudget(a)}>{budget(a)}</td>
            <td>
              {#if a.running}
                <span class="ok">en cours…</span>
              {:else if a.last}
                <span class:ok={a.last.status === 'ok'} class:error={a.last.status === 'failed'}>{statusLabel[a.last.status]}</span>
                <span class="muted small">{date(a.last.started_at)} · {TRIGGER[a.last.trigger]}</span>
              {:else}<span class="muted">jamais</span>{/if}
            </td>
            <td>
              <div class="actions">
                {#if a.running}
                  <button class="link error" onclick={() => act(() => stopAgent(a.id))}>Couper</button>
                {:else}
                  <button class="link" onclick={() => launch(a)}>Lancer</button>
                {/if}
                <button class="link" onclick={() => edit(a)}>Modifier</button>
                <button class="link" onclick={() => toggle(a)}>{a.enabled ? 'Désactiver' : 'Activer'}</button>
                {#if a.last?.session_id}<a href="#sessions/{a.last.session_id}">Session</a>{/if}
                <button class="link error" onclick={() => remove(a)} disabled={a.running}>Supprimer</button>
              </div>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {:else}
    <p class="muted">Chargement…</p>
  {/if}

  {#if history}
    <div class="card history">
      <div class="row">
        <h2>Départs : {history.agent.name}</h2>
        <span class="grow"></span>
        <button class="link" onclick={closeHistory}>Fermer</button>
      </div>
      {#each history.runs as r (r.id)}
        <details open={r === history.runs[0]}>
          <summary>
            <span class:ok={r.status === 'ok'} class:error={r.status === 'failed'}>{statusLabel[r.status]}</span>
            · {date(r.started_at)} · {TRIGGER[r.trigger]}{#if r.depth > 0} (chaîne {r.depth}){/if}
            {#if duration(r)}· {duration(r)}{/if}
            {#if r.tokens}· {kTok(r.tokens)} jetons · {mins(r.gpu_s)} GPU{/if}
            {#if r.session_id}· <a href="#sessions/{r.session_id}">session</a>{/if}
          </summary>
          {#if r.context}<p class="small muted ctx">{r.context}</p>{/if}
          {#if r.output}<pre>{r.output}</pre>{:else}<p class="muted small">Pas de sortie.</p>{/if}
        </details>
      {:else}
        <p class="muted">Aucun départ.</p>
      {/each}
    </div>
  {/if}
</section>

{#if form}
  <div class="overlay" role="presentation" onclick={(e) => e.target === e.currentTarget && (form = null)}>
    <form class="card" onsubmit={save}>
      <h2>{form.id === null ? 'Nouvel agent' : 'Modifier l’agent'}</h2>
      <div class="row">
        <label class="grow">Nom <input bind:value={form.name} maxlength="80" required /></label>
        <label class="grow">Topic
          <select bind:value={form.topic_id}>
            {#each tree as p}
              <optgroup label={p.name}>
                {#each p.topics as t}<option value={t.id}>{p.slug} / {t.slug}</option>{/each}
              </optgroup>
            {/each}
          </select>
        </label>
        <label>Modèle
          <select bind:value={form.preset_id}>
            <option value={null}>Par défaut</option>
            {#each presetList as p (p.id)}<option value={p.id}>{p.name}</option>{/each}
          </select>
        </label>
      </div>
      <label>Rôle
        <textarea rows="6" bind:value={form.role} required
          placeholder="Chaque matin, lis les ventes de la veille dans ventes.csv, compare à la semaine passée et écris un résumé de 5 lignes."></textarea>
      </label>

      <fieldset>
        <legend><label class="check"><input type="checkbox" bind:checked={form.useSchedule} /> Sur un horaire</label></legend>
        {#if form.useSchedule}
          <div class="row">
            <label class="grow">Quand <input class="mono" bind:value={form.schedule} required /></label>
            <label>Fuseau <input bind:value={form.tz} /></label>
          </div>
          <p class="small">
            {#each examples as ex}<button type="button" class="link" onclick={() => (form!.schedule = ex.s)}>{ex.l}</button> {/each}
          </p>
          <p class="small muted">
            {#if previewError}<span class="error">{previewError}</span>
            {:else if preview.length}Prochains : {preview.map(date).join(' · ')}{/if}
          </p>
        {/if}
      </fieldset>

      <fieldset>
        <legend><label class="check"><input type="checkbox" bind:checked={form.useFeed} /> Sur une notification du fil</label></legend>
        {#if form.useFeed && form.feed_filter}
          <p class="small sources">
            {#each Object.entries(SOURCES) as [s, l] (s)}
              <label class="check"><input type="checkbox" checked={form.feed_filter.sources.includes(s)} onchange={() => toggleSource(s)} /> {l}</label>
            {/each}
          </p>
          <div class="row frow">
            <label>Niveau minimal
              <select bind:value={form.feed_filter.min_level}>
                {#each Object.entries(LEVEL) as [v, l] (v)}<option value={v}>{l}</option>{/each}
              </select>
            </label>
            <label class="grow">Contient <input bind:value={form.feed_filter.contains} maxlength="200" placeholder="vente, échec, main…" /></label>
          </div>
          <p class="small muted">Aucune source cochée = toutes. Une minute au moins entre deux départs par le fil ; un agent ne part pas sur ses propres notifications.</p>
        {/if}
      </fieldset>

      <div class="row">
        <label>Approbation
          <select bind:value={form.approval}>
            <option value="auto">Auto : agit seul dans sa sandbox</option>
            <option value="prudent">Prudent : demande votre accord (fil + push), 1 h au plus</option>
          </select>
        </label>
        <label>Délai max (s) <input type="number" min="60" max="7200" bind:value={form.timeout_s} /></label>
      </div>
      <div class="row">
        <label>Jetons par jour <input type="number" min="0" step="1000" bind:value={form.budget_tokens} /></label>
        <label>Minutes de GPU par jour <input type="number" min="0" max="1440" bind:value={form.budget_gpu_min} /></label>
      </div>
      <p class="small muted">0 = sans limite. Budget atteint : plus de départ jusqu’à minuit, une notification.</p>
      <label class="check"><input type="checkbox" bind:checked={form.notify} /> Pousser le résultat même réussi (les échecs partent toujours)</label>
      <label class="check"><input type="checkbox" bind:checked={form.enabled} /> Activé</label>
      {#if formError}<p class="error">{formError}</p>{/if}
      <div class="row">
        <button type="submit">Enregistrer</button>
        <button type="button" class="link" onclick={() => (form = null)}>Annuler</button>
      </div>
    </form>
  </div>
{/if}

<style>
  .head { align-items: center; }
  .head h1 { margin: 0; }
  .agents { width: 100%; border-collapse: collapse; margin-top: var(--s4); }
  .agents th { text-align: left; font-weight: 500; color: var(--muted); font-size: var(--fs-sm); }
  .agents th, .agents td { padding: var(--s2); border-bottom: 1px solid var(--line); vertical-align: top; }
  .agents tr.off td { opacity: 0.6; }
  .actions { white-space: nowrap; display: flex; gap: var(--s2); }
  .strong { font-weight: 600; }
  .history { margin-top: var(--s6); }
  .history pre { white-space: pre-wrap; overflow-wrap: anywhere; max-height: 320px; overflow: auto; background: var(--bg); padding: var(--s2); border-radius: var(--radius); font-size: var(--fs-sm); }
  .ctx { white-space: pre-wrap; margin: var(--s1) 0; }
  summary { cursor: pointer; padding: var(--s1) 0; }
  .overlay { position: fixed; inset: 0; background: rgb(0 0 0 / 0.5); display: grid; place-items: center; z-index: 10; }
  .overlay form { width: min(720px, 94vw); max-height: 92vh; overflow: auto; display: flex; flex-direction: column; gap: var(--s3); }
  fieldset { border: 1px solid var(--line); border-radius: var(--radius); padding: var(--s2) var(--s3); margin: 0; }
  legend { padding: 0 var(--s1); }
  .frow { gap: var(--s3); }
  .sources { display: flex; flex-wrap: wrap; gap: var(--s3); }
  .check { display: flex; gap: var(--s2); align-items: center; }
  textarea { width: 100%; }
  .mono { font-family: ui-monospace, monospace; }
</style>
