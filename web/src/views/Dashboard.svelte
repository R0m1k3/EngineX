<script lang="ts">
  import { onDestroy } from 'svelte';
  import {
    engine, presets, queueState, board, crons, agents, feed, sessions, interruptTurn, message,
    type EngineInfo, type Preset, type QueueState, type Task, type Cron, type Agent, type FeedItem, type Session,
  } from '../lib/api';
  import { openEvents } from '../lib/sse';

  let eng = $state<EngineInfo | null>(null);
  let presetList = $state<Preset[]>([]);
  let queue = $state<QueueState | null>(null);
  let tasks = $state<Task[]>([]);
  let next = $state<Cron[]>([]);
  let notes = $state<FeedItem[]>([]);
  let recent = $state<Session[]>([]);
  let bots = $state<Agent[]>([]);
  let error = $state('');

  const name = (id: number | null | undefined) => presetList.find((p) => p.id === id)?.name ?? (id == null ? '—' : `preset ${id}`);
  const when = (t: number | null) =>
    t ? new Date(t * 1000).toLocaleString('fr-FR', { weekday: 'short', hour: '2-digit', minute: '2-digit' }) : '—';
  const ago = (t: number) => {
    const m = Math.max(0, Math.round((Date.now() / 1000 - t) / 60));
    return m < 1 ? 'à l’instant' : m < 60 ? `${m} min` : `${Math.floor(m / 60)} h ${m % 60} min`;
  };
  const today = () => {
    const d = new Date();
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
  };
  const STATE: Record<string, string> = {
    stopped: 'arrêté', starting: 'chargement', ready: 'prêt', failed: 'en échec', stopping: 'arrêt…',
  };

  async function load() {
    const r = await Promise.allSettled([engine(), queueState(), presets(), board(), crons(), feed({ limit: 6 }), sessions({}), agents()]);
    const [e, q, p, b, c, f, s, a] = r;
    if (e.status === 'fulfilled') eng = e.value;
    if (q.status === 'fulfilled') queue = q.value;
    if (p.status === 'fulfilled') presetList = p.value;
    if (b.status === 'fulfilled') {
      const t = today();
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
  }

  load();
  const t = setInterval(load, 5000);
  const close = openEvents('/api/feed/events', { feed: (d) => d.item && (notes = [d.item, ...notes].slice(0, 6)) });
  onDestroy(() => {
    clearInterval(t);
    close();
  });

  const st = $derived(eng?.state);
  const loaded = $derived(st && 'preset_id' in st ? st.preset_id : null);
  const gib = (m: number) => (m / 1024).toFixed(1);
</script>

<section>
  <h1>Tableau de bord</h1>
  {#if error}<p class="error small">{error}</p>{/if}
  <div class="grid">
    <div class="card engine">
      <h2><a href="#models">Moteur</a></h2>
      {#if eng && st}
        <p class="big">
          <span class="dot s-{st.state}"></span>{STATE[st.state] ?? st.state}
          {#if loaded !== null}<span class="muted"> · {name(loaded)}</span>{/if}
        </p>
        {#if st.state === 'starting'}<p class="small muted">Chargement {st.progress} %</p>{/if}
        {#if st.state === 'failed'}<p class="small error">{st.cause}</p>{/if}
        <p class="small muted">
          {#if eng.n_ctx}contexte {Math.round(eng.n_ctx / 1024)}k{/if}
          {#if eng.last_gen?.tokens_per_s} · dernier tour {eng.last_gen.tokens_per_s.toFixed(1)} t/s{/if}
          {#if eng.build} · llama.cpp {eng.build}{/if}
        </p>
        {#each eng.gpus as g (g.index)}
          <div class="gpu">
            <span class="small">GPU{g.index} {g.name}</span>
            <span class="meter" title="{gib(g.used_mib)} / {gib(g.total_mib)} Gio"><span style="width: {Math.min(100, (100 * g.used_mib) / Math.max(1, g.total_mib))}%"></span></span>
            <span class="small muted num">{gib(g.used_mib)}/{gib(g.total_mib)} Gio · {g.temp_c} °C</span>
          </div>
        {:else}
          <p class="small muted">Aucune carte NVIDIA vue.</p>
        {/each}
      {:else}
        <p class="muted">…</p>
      {/if}
    </div>

    <div class="card">
      <h2>File d’attente</h2>
      {#if queue}
        {#if queue.swapping}<p class="small">Chargement de « {name(queue.swapping)} »…</p>{/if}
        {#each queue.holders as h, i (i)}
          <p class="small line">
            <span>▶ {#if h.session_id}<a href="#sessions/{h.session_id}">{h.label}</a>{:else}{h.label}{/if}</span>
            <span class="muted">{ago(h.since)}</span>
            {#if h.session_id && h.priority > 0}<button class="link" onclick={() => interruptTurn(h.session_id!).then(load)}>Interrompre</button>{/if}
          </p>
        {/each}
        {#each queue.waiting as w, i (i)}
          <p class="small line muted">
            <span>{i + 1}. {#if w.session_id}<a href="#sessions/{w.session_id}">{w.label}</a>{:else}{w.label}{/if}</span>
            <span>{w.needs_swap ? `veut ${name(w.preset_id)}` : ''} · attend depuis {ago(w.since)}</span>
          </p>
        {/each}
        {#if !queue.holders.length && !queue.waiting.length}<p class="small muted">Libre ({queue.slots} slot{queue.slots > 1 ? 's' : ''}).</p>{/if}
      {/if}
    </div>

    <div class="card">
      <h2><a href="#feed">Notifications</a></h2>
      {#each notes as n (n.id)}
        <p class="small line lvl-{n.level}" class:unread={!n.read}>
          {#if n.link}<a href={n.link}>{n.title}</a>{:else}<span>{n.title}</span>{/if}
          <span class="muted">{ago(n.at)}</span>
        </p>
      {:else}
        <p class="small muted">Rien de neuf.</p>
      {/each}
    </div>

    <div class="card">
      <h2><a href="#tasks">Tâches du jour</a></h2>
      {#each tasks as t (t.id)}
        <p class="small line"><span>{t.status === 'doing' ? '◐' : '○'} {t.title}</span><span class="muted">{t.due === today() ? 'aujourd’hui' : t.due ?? ''}</span></p>
      {:else}
        <p class="small muted">Rien d’urgent.</p>
      {/each}
    </div>

    <div class="card">
      <h2><a href="#crons">Prochains crons</a></h2>
      {#each next as c (c.id)}
        <p class="small line"><span>{c.name}</span><span class="muted">{when(c.next_run)}</span></p>
      {:else}
        <p class="small muted">Aucun cron actif.</p>
      {/each}
    </div>

    <div class="card">
      <h2><a href="#agents">Agents</a></h2>
      {#each bots as b (b.id)}
        <p class="small line" class:muted={!b.enabled}>
          <a href="#agents/{b.id}">{b.running ? '▶ ' : ''}{b.name}</a>
          <span class="muted">
            {#if b.running}en cours{:else if b.last}<span class:lvl-error={b.last.status === 'failed'}>{b.last.status === 'ok' ? 'réussi' : b.last.status === 'failed' ? 'échec' : b.last.status}</span> · {ago(b.last.started_at)}{:else}jamais{/if}
            {#if b.budget_tokens}· {Math.round((100 * b.today.tokens) / b.budget_tokens)} % du budget{/if}
          </span>
        </p>
      {:else}
        <p class="small muted">Aucun agent.</p>
      {/each}
    </div>

    <div class="card">
      <h2><a href="#sessions">Sessions récentes</a></h2>
      {#each recent as s (s.id)}
        <p class="small line"><a href="#sessions/{s.id}">{s.running ? '▶ ' : ''}{s.title}</a><span class="muted">{ago(s.updated_at)}</span></p>
      {:else}
        <p class="small muted">Aucune session.</p>
      {/each}
    </div>
  </div>
</section>

<style>
  h1 { margin-top: 0; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(320px, 1fr)); gap: var(--s4); }
  .card { display: flex; flex-direction: column; gap: var(--s2); min-width: 0; }
  .card h2 { margin: 0 0 var(--s1); font-size: 15px; }
  .card h2 a { color: inherit; text-decoration: none; }
  .engine { grid-row: span 2; }
  .big { font-size: 18px; margin: 0; }
  .dot { display: inline-block; width: 9px; height: 9px; border-radius: 50%; margin-right: var(--s2); background: var(--muted); }
  .s-ready { background: var(--accent); }
  .s-starting, .s-stopping { background: var(--warn); }
  .s-failed { background: var(--error); }
  .gpu { display: grid; grid-template-columns: 1fr; gap: 2px; margin-top: var(--s2); }
  .meter { height: 6px; background: var(--bg); border-radius: 3px; overflow: hidden; }
  .meter span { display: block; height: 100%; background: var(--accent); }
  .num { font-variant-numeric: tabular-nums; }
  .line { display: flex; justify-content: space-between; gap: var(--s3); margin: 0; }
  .line > :first-child { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .line > :last-child { white-space: nowrap; }
  .lvl-error a, .lvl-error span:first-child { color: var(--error); }
  span.lvl-error { color: var(--error); }
  .unread { font-weight: 600; }
</style>
