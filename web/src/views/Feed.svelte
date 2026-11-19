<script lang="ts">
  import { onDestroy } from 'svelte';
  import {
    feed, feedRead, feedDelete, feedClear, pushStatus, pushSubscribe, pushUnsubscribe, pushMode, pushTest,
    webhooks, createWebhook, patchWebhook, deleteWebhook, agents, message,
    type FeedItem, type FeedLevel, type PushStatus, type PushMode, type Webhook, type Agent,
  } from '../lib/api';
  import { openEvents } from '../lib/sse';

  let { sub }: { sub: string } = $props();
  let tab = $derived(sub === 'push' || sub === 'webhooks' ? sub : 'feed');

  let items = $state<FeedItem[] | null>(null);
  let unread = $state(0);
  let onlyUnread = $state(false);
  let more = $state(false);
  let error = $state('');

  const LEVEL: Record<FeedLevel, string> = { info: 'info', success: 'réussi', warning: 'à voir', error: 'erreur' };
  const date = (t: number) => new Date(t * 1000).toLocaleString('fr-FR', { dateStyle: 'short', timeStyle: 'short' });

  async function load() {
    try {
      const r = await feed({ limit: 50, unread: onlyUnread });
      items = r.items;
      unread = r.unread;
      more = r.items.length === 50;
      error = '';
    } catch (e) {
      error = message(e);
    }
  }

  async function loadMore() {
    if (!items?.length) return;
    const r = await feed({ limit: 50, before: items[items.length - 1].id, unread: onlyUnread });
    items = [...items, ...r.items];
    more = r.items.length === 50;
  }

  const act = async (f: () => Promise<{ unread: number }>) => {
    try {
      unread = (await f()).unread;
      await load();
    } catch (e) {
      error = message(e);
    }
  };

  async function open(i: FeedItem) {
    if (!i.read) await act(() => feedRead({ ids: [i.id] }));
    if (i.link) location.hash = i.link.replace(/^#/, '');
  }

  load();
  const close = openEvents('/api/feed/events', {
    feed: (d) => {
      if (typeof d.unread === 'number') unread = d.unread;
      if (d.item && items && !onlyUnread) items = [d.item, ...items.filter((x) => x.id !== d.item.id)];
      else if (d.item || d.reload) load();
    },
  });
  onDestroy(close);

  // ---- Push ----
  let push = $state<PushStatus | null>(null);
  let pushMsg = $state('');
  let here = $state<PushSubscription | null>(null);
  const secure = window.isSecureContext && 'serviceWorker' in navigator && 'PushManager' in window;

  async function loadPush() {
    try {
      push = await pushStatus();
      if (secure) {
        const reg = await navigator.serviceWorker.getRegistration('/');
        here = (await reg?.pushManager.getSubscription()) ?? null;
      }
    } catch (e) {
      pushMsg = message(e);
    }
  }

  function key(b64: string) {
    const s = atob(b64.replace(/-/g, '+').replace(/_/g, '/') + '='.repeat((4 - (b64.length % 4)) % 4));
    return Uint8Array.from(s, (c) => c.charCodeAt(0));
  }

  async function enable() {
    pushMsg = '';
    try {
      if ((await Notification.requestPermission()) !== 'granted') {
        pushMsg = 'Notifications refusées par le navigateur (réglages du site).';
        return;
      }
      const reg = await navigator.serviceWorker.register('/sw.js', { scope: '/' });
      await navigator.serviceWorker.ready;
      const s = await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: key(push!.public_key) });
      await pushSubscribe(s.toJSON());
      pushMsg = 'Ce navigateur recevra les notifications.';
      await loadPush();
    } catch (e) {
      pushMsg = message(e);
    }
  }

  async function disable() {
    try {
      if (here) {
        await pushUnsubscribe({ endpoint: here.endpoint });
        await here.unsubscribe();
      }
      await loadPush();
    } catch (e) {
      pushMsg = message(e);
    }
  }

  async function setMode(m: PushMode) {
    try {
      await pushMode(m);
      await loadPush();
    } catch (e) {
      pushMsg = message(e);
    }
  }

  async function test() {
    try {
      const r = await pushTest();
      pushMsg = r.sent ? `Envoyée à ${r.sent} navigateur(s).` : 'Aucun navigateur abonné.';
      if (r.errors.length) pushMsg += ` ${r.errors.join(' ; ')}`;
      await loadPush();
    } catch (e) {
      pushMsg = message(e);
    }
  }

  // ---- Webhooks ----
  let hooks = $state<Webhook[] | null>(null);
  let agentList = $state<Agent[]>([]);
  let hookMsg = $state('');
  let fresh = $state<{ name: string; url: string } | null>(null);
  let form = $state({ name: '', level: 'info' as FeedLevel, push: false });

  async function loadHooks() {
    try {
      hooks = await webhooks();
      agentList = await agents().catch(() => agentList);
    } catch (e) {
      hookMsg = message(e);
    }
  }

  async function addHook(e: SubmitEvent) {
    e.preventDefault();
    try {
      const r = await createWebhook({ name: form.name.trim(), level: form.level, push: form.push });
      fresh = { name: form.name.trim(), url: location.origin + r.path };
      form = { name: '', level: 'info', push: false };
      await loadHooks();
    } catch (err) {
      hookMsg = message(err);
    }
  }

  const hookAct = async (f: () => Promise<unknown>) => {
    try {
      await f();
      await loadHooks();
    } catch (e) {
      hookMsg = message(e);
    }
  };

  $effect(() => {
    if (tab === 'push' && !push) loadPush();
    if (tab === 'webhooks' && !hooks) loadHooks();
  });
</script>

<section>
  <div class="row head">
    <h1>Notifications</h1>
    <span class="grow"></span>
    <div class="tabs">
      <a href="#feed" class:on={tab === 'feed'}>Fil{unread ? ` (${unread})` : ''}</a>
      <a href="#feed/push" class:on={tab === 'push'}>Push</a>
      <a href="#feed/webhooks" class:on={tab === 'webhooks'}>Webhooks</a>
    </div>
  </div>
  {#if error}<p class="error">{error}</p>{/if}

  {#if tab === 'feed'}
    <div class="row tools small">
      <label class="check"><input type="checkbox" bind:checked={onlyUnread} onchange={load} /> Non lues seulement</label>
      <span class="grow"></span>
      <button class="link" onclick={() => act(() => feedRead({ all: true }))} disabled={!unread}>Tout marquer lu</button>
      <button class="link error" onclick={() => confirm('Vider le fil ?') && act(feedClear)} disabled={!items?.length}>Vider</button>
    </div>
    {#if items && !items.length}
      <p class="muted">Rien pour l’instant. Les crons finis, les tours longs, les vérifications rouges, les approbations attendues, les téléchargements et les webhooks arrivent ici.</p>
    {:else if items}
      <ul class="feed">
        {#each items as i (i.id)}
          <li class="lvl-{i.level}" class:unread={!i.read}>
            <button class="item" onclick={() => open(i)}>
              <span class="top"><strong>{i.title}</strong><span class="muted small">{LEVEL[i.level]} · {i.source} · {date(i.at)}</span></span>
              {#if i.body}<span class="body small">{i.body}</span>{/if}
            </button>
            <button class="link small" title="Supprimer" onclick={() => act(() => feedDelete(i.id))}>✕</button>
          </li>
        {/each}
      </ul>
      {#if more}<button class="link" onclick={loadMore}>Plus ancien…</button>{/if}
    {:else}
      <p class="muted">Chargement…</p>
    {/if}
  {:else if tab === 'push'}
    <div class="card">
      <h2>Notifications sur ce navigateur</h2>
      {#if !secure}
        <p class="error small">
          Le navigateur n’autorise le push que sur une adresse HTTPS (ou localhost). Passez par votre reverse proxy HTTPS ou
          Tailscale (voir le README), puis revenez ici.
        </p>
      {:else if here}
        <p class="small">Ce navigateur est abonné. <button class="link" onclick={disable}>Désabonner</button></p>
      {:else}
        <button onclick={enable} disabled={!push}>Recevoir les notifications ici</button>
      {/if}
      {#if push}
        <label class="mode">Ce qui part en push
          <select value={push.mode} onchange={(e) => setMode((e.currentTarget as HTMLSelectElement).value as PushMode)}>
            <option value="important">Important : erreurs, approbations, vérifications rouges, réponses longues, webhooks marqués</option>
            <option value="all">Tout le fil</option>
            <option value="off">Rien</option>
          </select>
        </label>
        <p class="row small"><span class="muted">{push.subscriptions.length} navigateur(s) abonné(s)</span><button class="link" onclick={test}>Envoyer un essai</button></p>
        {#each push.subscriptions as s (s.id)}
          <p class="small sub">
            <span title={s.agent}>{s.host ?? s.endpoint}</span>
            <span class="muted">depuis {date(s.created_at)}{s.last_ok ? ` · reçu ${date(s.last_ok)}` : ''}</span>
            <button class="link error" onclick={async () => { await pushUnsubscribe({ id: s.id }); await loadPush(); }}>Retirer</button>
          </p>
        {/each}
      {/if}
      {#if pushMsg}<p class="small">{pushMsg}</p>{/if}
      <p class="muted small">
        Le contenu est chiffré pour votre navigateur ; il passe par le service de push de son éditeur (Google, Mozilla,
        Apple), seul envoi d’EngineX vers l’extérieur.
      </p>
    </div>
  {:else}
    <div class="card">
      <h2>Webhooks entrants</h2>
      <p class="muted small">
        Une adresse par service (GitHub, Gitea, un script, une alarme…) : chaque appel POST écrit une ligne dans le fil.
        GitHub et Gitea sont résumés (push, PR, issue, workflow) ; sinon <code>title</code> et <code>message</code> du JSON,
        ou le texte brut. L’adresse est le secret : elle n’est montrée qu’une fois. Un webhook peut aussi lancer un agent,
        avec l’appel reçu comme contexte.
      </p>
      <form class="row add" onsubmit={addHook}>
        <input placeholder="Nom (ex. GitHub app)" bind:value={form.name} maxlength="80" required />
        <select bind:value={form.level}>
          {#each Object.entries(LEVEL) as [v, l] (v)}<option value={v}>{l}</option>{/each}
        </select>
        <label class="check"><input type="checkbox" bind:checked={form.push} /> push</label>
        <button type="submit">Créer</button>
      </form>
      {#if fresh}
        <p class="banner small">
          Adresse de « {fresh.name} » (copiez-la maintenant) :<br /><code class="url">{fresh.url}</code>
          <button class="link" onclick={() => navigator.clipboard?.writeText(fresh!.url)}>Copier</button>
        </p>
      {/if}
      {#if hookMsg}<p class="error small">{hookMsg}</p>{/if}
      {#if hooks?.length}
        <table class="hooks">
          <thead><tr><th>Nom</th><th>Niveau</th><th>Appels</th><th>Dernier</th><th>Lance l’agent</th><th></th></tr></thead>
          <tbody>
            {#each hooks as h (h.id)}
              <tr class:off={!h.enabled}>
                <td>{h.name}{#if h.push}<span class="badge">push</span>{/if}</td>
                <td>{LEVEL[h.level]}</td>
                <td>{h.count}</td>
                <td class="muted">{h.last_at ? date(h.last_at) : 'jamais'}</td>
                <td>
                  <select value={h.agent_id} onchange={(e) => {
                    const v = (e.currentTarget as HTMLSelectElement).value;
                    hookAct(() => patchWebhook(h.id, { agent_id: v === '' ? null : Number(v) }));
                  }}>
                    <option value="">—</option>
                    {#each agentList as a (a.id)}<option value={a.id}>{a.name}</option>{/each}
                  </select>
                </td>
                <td class="actions">
                  <button class="link" onclick={() => hookAct(() => patchWebhook(h.id, { push: !h.push }))}>{h.push ? 'Sans push' : 'Avec push'}</button>
                  <button class="link" onclick={() => hookAct(() => patchWebhook(h.id, { enabled: !h.enabled }))}>{h.enabled ? 'Désactiver' : 'Activer'}</button>
                  <button class="link error" onclick={() => confirm(`Supprimer « ${h.name} » ?`) && hookAct(() => deleteWebhook(h.id))}>Supprimer</button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      {:else if hooks}
        <p class="muted small">Aucun webhook.</p>
      {/if}
    </div>
  {/if}
</section>

<style>
  .head { align-items: center; }
  .head h1 { margin: 0; }
  .tabs { display: flex; gap: var(--s4); }
  .tabs a { color: var(--muted); text-decoration: none; }
  .tabs a.on { color: var(--text); font-weight: 600; }
  .tools { margin: var(--s4) 0 var(--s2); gap: var(--s4); }
  .grow { flex: 1; }
  .check { display: flex; gap: var(--s2); align-items: center; }
  .feed { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: var(--s1); }
  .feed li { display: flex; gap: var(--s2); align-items: flex-start; border-left: 3px solid var(--line); background: var(--panel); border-radius: var(--radius); padding: var(--s2) var(--s3); }
  .feed li.unread { background: color-mix(in srgb, var(--panel) 80%, var(--accent) 8%); }
  .feed li.lvl-success { border-left-color: var(--accent); }
  .feed li.lvl-warning { border-left-color: #d9a54a; }
  .feed li.lvl-error { border-left-color: var(--error); }
  .item { all: unset; cursor: pointer; flex: 1; display: flex; flex-direction: column; gap: var(--s1); min-width: 0; }
  .top { display: flex; gap: var(--s3); align-items: baseline; flex-wrap: wrap; }
  .body { white-space: pre-wrap; overflow-wrap: anywhere; color: var(--muted); max-height: 7.5em; overflow: hidden; }
  .card { margin-top: var(--s4); display: flex; flex-direction: column; gap: var(--s3); }
  .card h2 { margin: 0; }
  .mode { display: flex; flex-direction: column; gap: var(--s1); }
  .sub { display: flex; gap: var(--s3); align-items: baseline; flex-wrap: wrap; margin: 0; }
  .add { gap: var(--s2); justify-content: flex-start; flex-wrap: wrap; }
  .url { overflow-wrap: anywhere; }
  .hooks { width: 100%; border-collapse: collapse; }
  .hooks th { text-align: left; font-weight: 500; color: var(--muted); font-size: 13px; }
  .hooks th, .hooks td { padding: var(--s2); border-bottom: 1px solid var(--line); }
  .hooks tr.off td { opacity: 0.6; }
  .actions { white-space: nowrap; display: flex; gap: var(--s2); }
</style>
