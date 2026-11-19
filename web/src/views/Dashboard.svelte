<script lang="ts">
  import { onDestroy } from 'svelte';
  import {
    engine, presets, queueState, board, crons, agents, feed, sessions, projects, interruptTurn, message,
    type EngineInfo, type Preset, type QueueState, type Task, type Cron, type Agent, type FeedItem, type Session, type Project,
  } from '../lib/api';
  import { openEvents } from '../lib/sse';
  import { Card, Button, EmptyState, Gauge, StatusDot, ProgressBar, Skeleton, int, dec1, toast } from '../lib/ui';
  import type { DotKind } from '../lib/ui';
  import { gpuRows } from './shell/status.svelte';
  import Page from './ops/Page.svelte';
  import { ago, ahead, scheduleText } from './ops/time';
  import Cpu from '@lucide/svelte/icons/cpu';
  import Layers from '@lucide/svelte/icons/layers';
  import MessageSquare from '@lucide/svelte/icons/message-square';
  import SquareCheck from '@lucide/svelte/icons/square-check';
  import Clock from '@lucide/svelte/icons/clock';
  import Bell from '@lucide/svelte/icons/bell';
  import Bot from '@lucide/svelte/icons/bot';
  import Check from '@lucide/svelte/icons/check';
  import ShieldAlert from '@lucide/svelte/icons/shield-alert';
  import LoaderCircle from '@lucide/svelte/icons/loader-circle';
  import CircleAlert from '@lucide/svelte/icons/circle-alert';
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert';

  let eng = $state<EngineInfo | null>(null);
  let presetList = $state<Preset[]>([]);
  let queue = $state<QueueState | null>(null);
  let tasks = $state<Task[]>([]);
  let taskProjects = $state<{ id: string; name: string }[]>([]);
  let next = $state<Cron[]>([]);
  let notes = $state<FeedItem[]>([]);
  let recent = $state<Session[]>([]);
  let bots = $state<Agent[]>([]);
  let tree = $state<Project[]>([]);
  let loaded = $state(false);
  let error = $state('');

  const name = (id: number | null | undefined) => presetList.find((p) => p.id === id)?.name ?? (id == null ? '—' : `preset ${id}`);
  const today = () => {
    const d = new Date();
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
  };
  const STATE: Record<string, string> = {
    stopped: 'Arrêté', starting: 'Chargement', ready: 'Prêt', failed: 'En échec', stopping: 'Arrêt en cours',
  };
  const topicPath = (id: number) => {
    for (const p of tree) {
      const t = p.topics.find((x) => x.id === id);
      if (t) return `${p.slug} / ${t.slug}`;
    }
    return '';
  };
  const dueLabel = (d: string | null) => {
    if (!d) return '';
    if (d === today()) return 'aujourd’hui';
    if (d < today()) return 'en retard';
    return new Date(`${d}T12:00`).toLocaleDateString('fr-FR', { day: 'numeric', month: 'short' });
  };

  async function load() {
    const r = await Promise.allSettled([engine(), queueState(), presets(), board(), crons(), feed({ limit: 6 }), sessions({}), agents()]);
    const [e, q, p, b, c, f, s, a] = r;
    if (e.status === 'fulfilled') eng = e.value;
    if (q.status === 'fulfilled') queue = q.value;
    if (p.status === 'fulfilled') presetList = p.value;
    if (b.status === 'fulfilled') {
      const t = today();
      taskProjects = b.value.projects;
      tasks = b.value.tasks
        .filter((x) => x.status === 'doing' || (x.status === 'todo' && x.due !== null && x.due <= t))
        .sort((a, z) => (a.status === z.status ? (a.due ?? '').localeCompare(z.due ?? '') : a.status === 'doing' ? -1 : 1))
        .slice(0, 8);
    }
    if (c.status === 'fulfilled')
      next = c.value.filter((x) => x.enabled && x.next_run).sort((a, z) => a.next_run! - z.next_run!).slice(0, 5);
    if (f.status === 'fulfilled') notes = f.value.items;
    if (a.status === 'fulfilled')
      bots = [...a.value].sort((x, y) => Number(y.running) - Number(x.running) || (y.last?.started_at ?? 0) - (x.last?.started_at ?? 0)).slice(0, 6);
    if (s.status === 'fulfilled') recent = [...s.value].sort((a, z) => z.updated_at - a.updated_at).slice(0, 5);
    const bad = r.find((x) => x.status === 'rejected') as PromiseRejectedResult | undefined;
    error = bad ? message(bad.reason) : '';
    loaded = true;
  }

  // Téléphone : jauge compacte (valeurs sans « Go »).
  const mq = window.matchMedia('(max-width: 767px)');
  let phone = $state(mq.matches);
  const onMq = (e: MediaQueryListEvent) => (phone = e.matches);
  mq.addEventListener('change', onMq);

  load();
  // Chemins « projet / topic » des sessions (lus une fois).
  projects().then((p) => (tree = p)).catch(() => {});
  const t = setInterval(load, 5000);
  const close = openEvents('/api/feed/events', { feed: (d) => d.item && (notes = [d.item, ...notes].slice(0, 6)) });
  onDestroy(() => {
    clearInterval(t);
    close();
    mq.removeEventListener('change', onMq);
  });

  async function interrupt(id: string) {
    try {
      await interruptTurn(id);
      toast('Tour interrompu', 'ok');
    } catch (e) {
      toast(message(e), 'error');
    }
    load();
  }

  const st = $derived(eng?.state);
  const loadedPreset = $derived(st && 'preset_id' in st ? st.preset_id : null);
  const dot = $derived<DotKind>(
    !st ? 'off' : st.state === 'ready' ? 'ok' : st.state === 'failed' ? 'err' : st.state === 'stopped' ? 'off' : 'warn',
  );
  const rows = $derived(gpuRows(eng).map((r) => ({ ...r, sub: r.sub?.replace(/^NVIDIA (GeForce )?/, '') })));
  const tps = $derived(st?.state === 'ready' ? (eng?.last_gen?.tokens_per_s ?? null) : null);
  const mtp = $derived.by(() => {
    const g = eng?.last_gen;
    return g?.draft_n ? Math.round((100 * (g.draft_accepted ?? 0)) / g.draft_n) : null;
  });
  // Demandes d'accord encore non lues de la dernière heure (au-delà, le tour prudent est coupé).
  const approvals = $derived(notes.filter((n) => n.source === 'approval' && !n.read && Date.now() / 1000 - n.at < 3600));
  const waitingSessions = $derived(new Set(approvals.map((n) => n.link?.replace(/^#sessions\//, ''))));
  const dateLine = new Date().toLocaleDateString('fr-FR', { weekday: 'long', day: 'numeric', month: 'long' });
  const agentDot = (b: Agent): DotKind => (!b.enabled ? 'off' : b.last?.status === 'failed' && !b.running ? 'err' : 'ok');
</script>

<Page title="Tableau de bord">
  {#snippet meta()}<span>{dateLine}</span>{/snippet}
  {#if error}<p class="err-line" role="alert">{error}</p>{/if}

  <div class="ui-tiles dash">
    {#each approvals as n (n.id)}
      <section class="approval" aria-label="Demande d’accord">
        <div class="ap-head"><ShieldAlert size={18} /><span>{n.title}</span><span class="when">{ago(n.at)}</span></div>
        {#if n.body}<p class="ap-body">{n.body}</p>{/if}
        {#if n.link}<Button variant="primary" href={n.link}>Ouvrir la session</Button>{/if}
      </section>
    {/each}

    <!-- Moteur -->
    <Card title="Moteur" icon={Cpu} span={2} more="Modèles" moreHref="#models">
      {#if !eng || !st}
        <Skeleton lines={4} />
      {:else}
        <div class="engine">
          {#if st.state === 'stopped' && loadedPreset === null}
            <EmptyState icon={Cpu} title="Aucun modèle chargé" text="Choisis un preset dans Modèles : il se charge sur les cartes et les sessions peuvent démarrer.">
              {#snippet action()}<Button variant="primary" href="#models">Choisir un modèle</Button>{/snippet}
            </EmptyState>
          {:else}
          <div>
            <div class="model">{loadedPreset !== null ? name(loadedPreset) : 'Aucun modèle'}</div>
            <div class="state s-{dot}"><StatusDot kind={dot} />{STATE[st.state] ?? st.state}{#if st.state === 'starting'}, {st.progress} %{/if}</div>
            {#if st.state === 'starting'}<div class="prog"><ProgressBar value={st.progress} label="Chargement du modèle" /></div>{/if}
            {#if st.state === 'failed'}<p class="cause">{st.cause}</p>{/if}
            <dl class="kv">
              {#if eng.n_ctx}<dt>Contexte</dt><dd>{int(eng.n_ctx)} jetons</dd>{/if}
              {#if eng.build}<dt>llama.cpp</dt><dd>{eng.build}{#if eng.pending}<span class="faint"> ({eng.pending.build} au prochain repos)</span>{/if}</dd>{/if}
              {#if mtp !== null}<dt>Brouillon accepté</dt><dd>{mtp} %</dd>{/if}
              {#if eng.last_gen?.cpu_cores != null}<dt>Cœurs CPU</dt><dd>{dec1(eng.last_gen.cpu_cores)}</dd>{/if}
            </dl>
          </div>
          {/if}
          <div>
            <div class="rate">
              {#if tps != null}<span class="big">{tps >= 10 ? Math.round(tps) : dec1(tps)}</span><span class="unit">t/s en génération</span>
              {:else if st.state === 'ready'}<span class="unit">Pas encore de réponse mesurée</span>
              {:else}<span class="unit">Mémoire des cartes</span>{/if}
            </div>
            {#if rows.length}
              <Gauge {rows} size="md" compact={phone} />
            {:else}
              <p class="faint small">Aucune carte NVIDIA vue.</p>
            {/if}
          </div>
        </div>
      {/if}
    </Card>

    <!-- File d'attente -->
    <Card title="File d’attente" icon={Layers}>
      {#if !queue}
        <Skeleton lines={2} />
      {:else}
        {#if queue.swapping}
          <div class="free"><span class="slot warn"><LoaderCircle size={18} /></span><div><div class="t">Changement de modèle</div><div class="sub">Chargement de « {name(queue.swapping)} »</div></div></div>
        {:else if !queue.holders.length && !queue.waiting.length}
          <div class="free"><span class="slot"><Check size={18} /></span><div><div class="t">Libre</div><div class="sub">Un tour envoyé maintenant démarre tout de suite{queue.slots > 1 ? ` (${queue.slots} places)` : ''}.</div></div></div>
        {/if}
        {#if queue.holders.length || queue.waiting.length}
          <ul class="list">
            {#each queue.holders as h, i (i)}
              <li>
                <StatusDot kind="ok" label="en cours" />
                <div class="grow">
                  <div class="t ell">{#if h.session_id}<a href="#sessions/{h.session_id}">{h.label}</a>{:else}{h.label}{/if}</div>
                  <div class="sub">en cours depuis {ago(h.since)}</div>
                </div>
                {#if h.session_id && h.priority > 0}<Button size="sm" variant="ghost" onclick={() => interrupt(h.session_id!)}>Interrompre</Button>{/if}
              </li>
            {/each}
            {#each queue.waiting as w, i (i)}
              <li>
                <span class="pos num">{i + 1}</span>
                <div class="grow">
                  <div class="t ell">{#if w.session_id}<a href="#sessions/{w.session_id}">{w.label}</a>{:else}{w.label}{/if}</div>
                  <div class="sub">attend depuis {ago(w.since)}{w.needs_swap ? `, veut ${name(w.preset_id)}` : ''}</div>
                </div>
              </li>
            {/each}
          </ul>
        {/if}
      {/if}
    </Card>

    <!-- Sessions récentes -->
    <Card title="Sessions récentes" icon={MessageSquare} more="Toutes" moreHref="#sessions">
      {#if !loaded}
        <Skeleton lines={3} />
      {:else if recent.length}
        <ul class="list">
          {#each recent as s (s.id)}
            {@const waiting = waitingSessions.has(s.id)}
            <li>
              <a class="row-link" href="#sessions/{s.id}">
                <StatusDot kind={waiting ? 'warn' : s.running ? 'ok' : 'off'} label={waiting ? 'attend ton accord' : s.running ? 'en cours' : 'au repos'} />
                <span class="grow">
                  <span class="t ell">{s.title}</span>
                  <span class="sub ell">{topicPath(s.topic_id)}{waiting ? ', attend ton accord' : s.running ? ', en cours' : ''}</span>
                </span>
                <span class="when">{ago(s.updated_at)}</span>
              </a>
            </li>
          {/each}
        </ul>
      {:else}
        <EmptyState icon={MessageSquare} title="Aucune session" text="Une session est une conversation avec le modèle dans un topic, avec sa sandbox.">
          {#snippet action()}<Button href="#sessions">Nouvelle session</Button>{/snippet}
        </EmptyState>
      {/if}
    </Card>

    <!-- Tâches du jour -->
    <Card title="Tâches du jour" icon={SquareCheck} more="Tâches" moreHref="#tasks">
      {#if !loaded}
        <Skeleton lines={3} />
      {:else if tasks.length}
        <ul class="list">
          {#each tasks as t (t.id)}
            {@const pname = taskProjects.find((p) => p.id === t.project_id)?.name}
            <li>
              <a class="row-link" href="#tasks">
                <span class="chk" class:doing={t.status === 'doing'} aria-label={t.status === 'doing' ? 'en cours' : 'à faire'} role="img"></span>
                <span class="grow t ell">{t.title}</span>
                {#if t.due && t.status === 'todo'}<span class="when" class:late={t.due < today()}>{dueLabel(t.due)}</span>{/if}
                {#if pname}<span class="tag">{pname}</span>{/if}
              </a>
            </li>
          {/each}
        </ul>
      {:else}
        <EmptyState icon={SquareCheck} title="Rien d’urgent" text="Les tâches en cours et celles dues aujourd’hui s’affichent ici." />
      {/if}
    </Card>

    <!-- Prochains crons -->
    <Card title="Prochains crons" icon={Clock} more="Crons" moreHref="#crons">
      {#if !loaded}
        <Skeleton lines={3} />
      {:else if next.length}
        <ul class="list">
          {#each next as c (c.id)}
            <li>
              <a class="row-link" href="#crons">
                <span class="grow">
                  <span class="t ell">{c.name}</span>
                  <span class="sub ell">{c.kind === 'prompt' ? 'consigne' : 'commande'}, {scheduleText(c.schedule)}</span>
                </span>
                <span class="when">{ahead(c.next_run)}</span>
              </a>
            </li>
          {/each}
        </ul>
      {:else}
        <EmptyState icon={Clock} title="Aucun cron actif" text="Un cron lance une consigne ou une commande à heure fixe dans un topic.">
          {#snippet action()}<Button href="#crons">Créer un cron</Button>{/snippet}
        </EmptyState>
      {/if}
    </Card>

    <!-- Notifications -->
    <Card title="Notifications" icon={Bell} span={2}>
      {#snippet actions()}
        <a class="more" href="#feed">Tout le fil</a>
      {/snippet}
      {#if !loaded}
        <Skeleton lines={3} />
      {:else if notes.length}
        <ul class="list">
          {#each notes as n (n.id)}
            <li>
              <a class="row-link" href={n.link ?? '#feed'}>
                {#if n.level === 'error'}<span class="lv err" role="img" aria-label="erreur"><CircleAlert size={16} /></span>
                {:else if n.level === 'warning'}<span class="lv warn" role="img" aria-label="à voir"><TriangleAlert size={16} /></span>
                {:else}<span class="lv"><span class="mark" class:on={!n.read} aria-label={n.read ? undefined : 'non lue'} role={n.read ? undefined : 'img'}></span></span>{/if}
                <span class="grow t ell">{n.title}</span>
                <span class="when">{ago(n.at)}</span>
              </a>
            </li>
          {/each}
        </ul>
      {:else}
        <EmptyState icon={Bell} title="Rien de neuf" text="Les crons finis, les tours longs, les demandes d’accord et les webhooks arrivent ici." />
      {/if}
    </Card>

    <!-- Agents -->
    <Card title="Agents" icon={Bot} more="Agents" moreHref="#agents">
      {#if !loaded}
        <Skeleton lines={3} />
      {:else if bots.length}
        <ul class="list">
          {#each bots as b (b.id)}
            <li>
              <a class="row-link" href="#agents/{b.id}">
                <StatusDot kind={agentDot(b)} label={!b.enabled ? 'désactivé' : 'actif'} />
                <span class="grow">
                  <span class="t ell">{b.name}</span>
                  <span class="sub ell">
                    {#if b.running}en cours{:else if b.last}{b.last.status === 'ok' ? 'réussi' : b.last.status === 'failed' ? 'échec' : b.last.status === 'skipped' ? 'sauté' : 'en cours'}, {ago(b.last.started_at)}{:else}jamais lancé{/if}{#if b.budget_tokens}, {Math.round((100 * b.today.tokens) / b.budget_tokens)} % du budget{/if}{#if !b.enabled}, désactivé{/if}
                  </span>
                </span>
              </a>
            </li>
          {/each}
        </ul>
      {:else}
        <EmptyState icon={Bot} title="Aucun agent" text="Un agent est un rôle durable qui part seul : sur un horaire, une notification ou un webhook.">
          {#snippet action()}<Button href="#agents">Créer un agent</Button>{/snippet}
        </EmptyState>
      {/if}
    </Card>
  </div>
</Page>

<style>
  .dash { border-width: 0 0 1px; }
  .err-line { margin: 0; padding: var(--s2) var(--s5); background: var(--danger-soft); color: var(--danger); font-size: var(--fs-sm); border-bottom: 1px solid var(--line); }

  .approval {
    grid-column: 1 / -1;
    background: var(--warn-soft);
    padding: var(--s4) var(--s5);
    display: flex;
    align-items: center;
    gap: var(--s3) var(--s4);
    flex-wrap: wrap;
    border-left: 2px solid var(--warn);
  }
  .ap-head { display: flex; align-items: center; gap: var(--s2); flex: 1; min-width: 0; font-weight: 600; }
  .ap-head :global(svg) { color: var(--warn); flex: none; }
  .ap-head .when { font-weight: 400; margin-left: var(--s2); }
  .ap-body { margin: 0; flex-basis: 100%; color: var(--muted); font-size: var(--fs-sm); white-space: pre-wrap; }

  .engine { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1.25fr); gap: var(--s6); }
  .model { font-size: var(--fs-xl); font-weight: 600; line-height: 1.25; overflow-wrap: anywhere; }
  .state { display: flex; align-items: center; gap: var(--s2); font-size: var(--fs-sm); margin-top: var(--s1); color: var(--muted); }
  .state.s-ok { color: var(--accent); }
  .state.s-warn { color: var(--warn); }
  .state.s-err { color: var(--danger); }
  .prog { margin-top: var(--s3); }
  .cause { margin: var(--s2) 0 0; color: var(--danger); font-size: var(--fs-sm); overflow-wrap: anywhere; }
  .kv { display: grid; grid-template-columns: auto 1fr; gap: var(--s2) var(--s4); margin: var(--s4) 0 0; font-size: var(--fs-sm); }
  .kv dt { color: var(--muted); }
  .kv dd { margin: 0; font-variant-numeric: tabular-nums; min-width: 0; overflow-wrap: anywhere; }
  .rate { display: flex; align-items: baseline; gap: var(--s2); margin-bottom: var(--s4); min-height: 32px; }
  .big { font-size: var(--fs-2xl); font-weight: 600; line-height: 1; font-variant-numeric: tabular-nums; }
  .unit { color: var(--muted); font-size: var(--fs-sm); }

  .free { display: flex; align-items: center; gap: var(--s3); }
  .slot { width: 36px; height: 36px; flex: none; border: 1px solid var(--line-strong); border-radius: var(--r); display: grid; place-items: center; color: var(--accent); }
  .slot.warn { color: var(--warn); }
  .free .t { font-size: var(--fs-md); }
  .free .sub { color: var(--muted); font-size: var(--fs-sm); }
  .free + .list { margin-top: var(--s2); }

  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .list li { display: flex; align-items: center; gap: var(--s3); padding: var(--s2) 0; border-top: 1px solid var(--line); font-size: var(--fs-sm); min-width: 0; }
  .list li:first-child { border-top: 0; padding-top: 0; }
  .list li:has(> .row-link) { padding: 0; }
  .row-link { display: flex; align-items: center; gap: var(--s3); flex: 1; min-width: 0; padding: var(--s2) 0; color: inherit; text-decoration: none; border-radius: var(--r-sm); }
  .list li:first-child .row-link { padding-top: 0; }
  .row-link:hover .t { color: var(--accent); }
  .grow { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .t { color: var(--text); font-size: var(--fs-md); }
  .t a { color: inherit; text-decoration: none; }
  .t a:hover { color: var(--accent); }
  .sub { color: var(--muted); font-size: var(--fs-xs); }
  .ell { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; display: block; }
  .when { color: var(--muted); white-space: nowrap; font-variant-numeric: tabular-nums; font-size: var(--fs-sm); flex: none; }
  .when.late { color: var(--danger); }
  .pos { width: 8px; text-align: center; color: var(--faint); }
  .tag { flex: none; padding: 0 var(--s2); height: 24px; display: inline-flex; align-items: center; border-radius: var(--r); background: var(--raised); color: var(--muted); font-size: var(--fs-xs); }
  .chk { width: 18px; height: 18px; flex: none; border: 1.5px solid var(--line-strong); border-radius: 3px; position: relative; }
  .chk.doing { border-color: var(--accent); }
  .chk.doing::after { content: ''; position: absolute; inset: 3px 50% 3px 3px; background: var(--accent-fill); border-radius: 1px; }
  .mark { width: 6px; height: 6px; border-radius: 50%; flex: none; }
  .mark.on { background: var(--accent); }
  .lv { width: 16px; flex: none; display: grid; place-items: center; }
  .lv.err { color: var(--danger); }
  .lv.warn { color: var(--warn); }
  .more { font-size: var(--fs-sm); color: var(--muted); text-decoration: none; }
  .more:hover { color: var(--accent); }
  .small { font-size: var(--fs-sm); }
  .faint { color: var(--faint); }

  @media (max-width: 1100px) {
    .engine { grid-template-columns: minmax(0, 1fr); gap: var(--s4); }
  }
  @media (max-width: 767px) {
    .dash { border: 0; }
    .approval { padding: var(--s4); }
    .approval :global(.btn) { width: 100%; height: var(--touch); }
    .row-link { min-height: var(--touch); }
    .list li:first-child .row-link { padding-top: var(--s2); }
    .model { font-size: var(--fs-lg); }
  }
</style>
