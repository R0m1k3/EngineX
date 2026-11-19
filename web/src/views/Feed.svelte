<script lang="ts">
  import { onDestroy } from 'svelte';
  import {
    feed, feedRead, feedDelete, feedClear, pushStatus, pushSubscribe, pushUnsubscribe, pushMode, pushTest,
    webhooks, createWebhook, patchWebhook, deleteWebhook, agents, message,
    type FeedItem, type FeedLevel, type PushStatus, type PushMode, type Webhook, type Agent,
  } from '../lib/api';
  import { openEvents } from '../lib/sse';
  import {
    Button, IconButton, Input, Select, Switch, Tabs, Badge, EmptyState, Skeleton, confirmDialog, toast, int,
  } from '../lib/ui';
  import Page from './ops/Page.svelte';
  import { since, stamp } from './ops/time';
  import Info from '@lucide/svelte/icons/info';
  import CircleCheck from '@lucide/svelte/icons/circle-check';
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
  import CircleAlert from '@lucide/svelte/icons/circle-alert';
  import X from '@lucide/svelte/icons/x';
  import Bell from '@lucide/svelte/icons/bell';
  import BellRing from '@lucide/svelte/icons/bell-ring';
  import Webhook_ from '@lucide/svelte/icons/webhook';
  import CheckCheck from '@lucide/svelte/icons/check-check';
  import Trash2 from '@lucide/svelte/icons/trash-2';
  import Copy from '@lucide/svelte/icons/copy';
  import Send from '@lucide/svelte/icons/send';
  import Plus from '@lucide/svelte/icons/plus';

  let { sub }: { sub: string } = $props();
  let tab = $derived(sub === 'push' || sub === 'webhooks' ? sub : 'feed');

  let items = $state<FeedItem[] | null>(null);
  let unread = $state(0);
  let onlyUnread = $state(false);
  let more = $state(false);
  let error = $state('');

  const LEVEL: Record<FeedLevel, string> = { info: 'info', success: 'réussi', warning: 'à voir', error: 'erreur' };
  const LEVEL_ICON = { info: Info, success: CircleCheck, warning: TriangleAlert, error: CircleAlert };
  const SOURCE: Record<string, string> = {
    webhook: 'webhook', cron: 'cron', session: 'session', approval: 'accord', agent: 'agent',
    download: 'téléchargement', engine: 'moteur', verify: 'vérification', disk: 'disque',
  };

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

  async function clearAll() {
    if (await confirmDialog('Vider le fil ? Toutes les notifications sont effacées.', { title: 'Vider le fil', confirm: 'Vider', danger: true })) act(feedClear);
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

  // Regroupement par jour pour la lecture.
  const dayKey = (t: number) => new Date(t * 1000).toDateString();
  function dayLabel(t: number) {
    const d = new Date(t * 1000);
    const now = new Date();
    const y = new Date(now);
    y.setDate(now.getDate() - 1);
    if (d.toDateString() === now.toDateString()) return 'Aujourd’hui';
    if (d.toDateString() === y.toDateString()) return 'Hier';
    const s = d.toLocaleDateString('fr-FR', { weekday: 'long', day: 'numeric', month: 'long' });
    return s.charAt(0).toUpperCase() + s.slice(1);
  }
  const groups = $derived.by(() => {
    const g: { key: string; label: string; items: FeedItem[] }[] = [];
    for (const i of items ?? []) {
      const k = dayKey(i.at);
      if (g.at(-1)?.key !== k) g.push({ key: k, label: dayLabel(i.at), items: [] });
      g.at(-1)!.items.push(i);
    }
    return g;
  });

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
        pushMsg = 'Notifications refusées par le navigateur : autorise-les dans les réglages du site, puis réessaie.';
        return;
      }
      const reg = await navigator.serviceWorker.register('/sw.js', { scope: '/' });
      await navigator.serviceWorker.ready;
      const s = await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: key(push!.public_key) });
      await pushSubscribe(s.toJSON());
      toast('Ce navigateur recevra les notifications', 'ok');
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
      const msg = r.sent ? `Essai envoyé à ${r.sent} navigateur${r.sent > 1 ? 's' : ''}` : 'Aucun navigateur abonné';
      toast(r.errors.length ? `${msg}. ${r.errors.join(' ; ')}` : msg, r.errors.length ? 'warn' : r.sent ? 'ok' : 'info');
      await loadPush();
    } catch (e) {
      pushMsg = message(e);
    }
  }

  async function removeSub(id: number) {
    try {
      await pushUnsubscribe({ id });
      await loadPush();
    } catch (e) {
      pushMsg = message(e);
    }
  }

  const MODES: { value: PushMode; label: string; text: string }[] = [
    { value: 'important', label: 'Important', text: 'Erreurs, demandes d’accord, vérifications rouges, réponses longues et webhooks marqués push.' },
    { value: 'all', label: 'Tout le fil', text: 'Chaque notification part aussi en push.' },
    { value: 'off', label: 'Rien', text: 'Le fil reste dans EngineX, rien ne part.' },
  ];

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
      hookMsg = '';
      await loadHooks();
    } catch (err) {
      hookMsg = message(err);
    }
  }

  const hookAct = async (f: () => Promise<unknown>) => {
    try {
      await f();
      hookMsg = '';
      await loadHooks();
    } catch (e) {
      hookMsg = message(e);
      toast(hookMsg, 'error');
    }
  };

  async function removeHook(h: Webhook) {
    if (await confirmDialog(`Supprimer « ${h.name} » ? Son adresse cessera de répondre.`, { title: 'Supprimer le webhook', confirm: 'Supprimer', danger: true }))
      hookAct(() => deleteWebhook(h.id));
  }

  async function copy(url: string) {
    try {
      await navigator.clipboard.writeText(url);
      toast('Adresse copiée', 'ok');
    } catch {
      toast('Copie impossible ici : sélectionne l’adresse à la main.', 'warn');
    }
  }

  $effect(() => {
    if (tab === 'push' && !push) loadPush();
    if (tab === 'webhooks' && !hooks) loadHooks();
  });

  const tabItems = $derived([
    { id: 'feed', label: 'Fil', href: '#feed', icon: Bell, count: unread },
    { id: 'push', label: 'Push', href: '#feed/push', icon: BellRing },
    { id: 'webhooks', label: 'Webhooks', href: '#feed/webhooks', icon: Webhook_ },
  ]);
  const levelOptions = Object.entries(LEVEL).map(([v, l]) => ({ value: v as FeedLevel, label: l }));
</script>

<Page title="Notifications">
  {#snippet meta()}{#if unread}<span class="num">{unread} non lue{unread > 1 ? 's' : ''}</span>{/if}{/snippet}
  {#snippet tabs()}<Tabs items={tabItems} active={tab} label="Notifications" />{/snippet}
  {#if error}<p class="err-line" role="alert">{error}</p>{/if}

  {#if tab === 'feed'}
    <div class="col">
      <div class="tools">
        <Switch bind:checked={onlyUnread} label="Non lues seulement" onchange={load} />
        <span class="spacer"></span>
        <Button variant="ghost" size="sm" icon={CheckCheck} onclick={() => act(() => feedRead({ all: true }))} disabled={!unread}>Tout marquer lu</Button>
        <Button variant="ghost" size="sm" icon={Trash2} onclick={clearAll} disabled={!items?.length}>Vider</Button>
      </div>
      {#if items && !items.length}
        <EmptyState icon={Bell} title={onlyUnread ? 'Tout est lu' : 'Rien pour l’instant'} text="Les crons finis, les tours longs, les vérifications rouges, les demandes d’accord, les téléchargements et les webhooks arrivent ici." />
      {:else if items}
        {#each groups as g (g.key)}
          <h2 class="day">{g.label}</h2>
          <ul class="feed">
            {#each g.items as i (i.id)}
              {@const Icon = LEVEL_ICON[i.level]}
              <li class="lvl-{i.level}" class:unread={!i.read}>
                <button class="item" onclick={() => open(i)}>
                  <span class="ic" role="img" aria-label={LEVEL[i.level]}><Icon size={18} /></span>
                  <span class="main">
                    <span class="top">
                      <span class="title">{i.title}</span>
                      {#if !i.read}<span class="sr-only">, non lue</span>{/if}
                    </span>
                    {#if i.body}<span class="body">{i.body}</span>{/if}
                    <span class="meta"><span>{SOURCE[i.source] ?? i.source}</span><span class="num" title={stamp(i.at)}>{since(i.at)}</span></span>
                  </span>
                </button>
                <IconButton icon={X} label="Supprimer" size="sm" class="del" onclick={() => act(() => feedDelete(i.id))} />
              </li>
            {/each}
          </ul>
        {/each}
        {#if more}<div class="more"><Button variant="ghost" onclick={loadMore}>Plus ancien</Button></div>{/if}
      {:else}
        <Skeleton lines={5} />
      {/if}
    </div>
  {:else if tab === 'push'}
    <div class="col">
      <section class="sect">
        <div class="s-head"><h2>Ce navigateur</h2><p>Reçois les notifications même quand EngineX est fermé.</p></div>
        <div class="s-body">
          {#if !secure}
            <p class="callout">
              Le navigateur n’autorise le push que sur une adresse HTTPS (ou localhost). Passe par ton reverse proxy HTTPS ou
              Tailscale (voir le README), puis reviens ici.
            </p>
          {:else if here}
            <div class="line"><Badge tone="ok" dot>abonné</Badge><Button variant="ghost" onclick={disable}>Désabonner</Button></div>
          {:else}
            <div><Button variant="primary" icon={BellRing} onclick={enable} disabled={!push}>Recevoir les notifications ici</Button></div>
          {/if}
          {#if pushMsg}<p class="ferr" role="alert">{pushMsg}</p>{/if}
        </div>
      </section>

      {#if push}
        <section class="sect">
          <div class="s-head"><h2>Ce qui part en push</h2><p>Pour tous les navigateurs abonnés.</p></div>
          <div class="s-body" role="radiogroup" aria-label="Ce qui part en push">
            {#each MODES as m (m.value)}
              <label class="radio" class:on={push.mode === m.value}>
                <input type="radio" name="push-mode" value={m.value} checked={push.mode === m.value} onchange={() => setMode(m.value)} />
                <span><span class="r-t">{m.label}</span><span class="r-x">{m.text}</span></span>
              </label>
            {/each}
          </div>
        </section>

        <section class="sect">
          <div class="s-head">
            <h2>Navigateurs abonnés</h2>
            <p>{push.subscriptions.length ? `${push.subscriptions.length} navigateur${push.subscriptions.length > 1 ? 's' : ''}` : 'Aucun pour l’instant.'}</p>
          </div>
          <div class="s-body">
            {#if push.subscriptions.length}
              <ul class="subs">
                {#each push.subscriptions as s (s.id)}
                  <li>
                    <span class="main">
                      <span class="ell" title={s.agent}>{s.host ?? s.endpoint}</span>
                      <span class="faint small">depuis le {stamp(s.created_at)}{s.last_ok ? `, reçu ${since(s.last_ok)}` : ''}</span>
                    </span>
                    <Button variant="ghost" size="sm" onclick={() => removeSub(s.id)}>Retirer</Button>
                  </li>
                {/each}
              </ul>
            {/if}
            <div><Button icon={Send} onclick={test}>Envoyer un essai</Button></div>
          </div>
        </section>
      {:else if !pushMsg}
        <Skeleton lines={3} />
      {/if}
      <p class="foot">
        Le contenu est chiffré pour ton navigateur ; il passe par le service de push de son éditeur (Google, Mozilla, Apple),
        seul envoi d’EngineX vers l’extérieur.
      </p>
    </div>
  {:else}
    <div class="col wide">
      <section class="sect">
        <div class="s-head">
          <h2>Nouveau webhook</h2>
          <p>
            Une adresse par service (GitHub, Gitea, un script, une alarme…) : chaque appel POST écrit une ligne dans le fil.
            GitHub et Gitea sont résumés (push, PR, issue, workflow) ; sinon <code>title</code> et <code>message</code> du JSON,
            ou le texte brut.
          </p>
        </div>
        <div class="s-body">
          <form class="hook-form" onsubmit={addHook}>
            <Input label="Nom" placeholder="GitHub app" bind:value={form.name} maxlength={80} required />
            <Select label="Niveau" bind:value={form.level} options={levelOptions} />
            <div class="sw-cell"><Switch bind:checked={form.push} label="Push" /></div>
            <Button type="submit" variant="primary" icon={Plus}>Créer</Button>
          </form>
          {#if fresh}
            <div class="fresh" role="status">
              <p>Adresse de « {fresh.name} ». Copie-la maintenant : c’est le secret du webhook, elle ne sera plus montrée.</p>
              <div class="url-row"><code class="url">{fresh.url}</code><Button size="sm" icon={Copy} onclick={() => copy(fresh!.url)}>Copier</Button></div>
            </div>
          {/if}
          {#if hookMsg}<p class="ferr" role="alert">{hookMsg}</p>{/if}
        </div>
      </section>

      <section class="sect">
        <div class="s-head"><h2>Webhooks</h2><p>Un webhook peut aussi lancer un agent, avec l’appel reçu comme contexte.</p></div>
        <div class="s-body">
          {#if hooks?.length}
            <ul class="hooks">
              {#each hooks as h (h.id)}
                {@const Icon = LEVEL_ICON[h.level]}
                <li class:off={!h.enabled}>
                  <div class="h-top">
                    <span class="ic lvl-{h.level}" role="img" aria-label={LEVEL[h.level]}><Icon size={16} /></span>
                    <span class="main">
                      <span class="h-name">{h.name}</span>
                      <span class="faint small num">{LEVEL[h.level]}, {int(h.count)} appel{h.count > 1 ? 's' : ''}, {h.last_at ? `dernier ${since(h.last_at)}` : 'jamais appelé'}</span>
                    </span>
                    <IconButton icon={Trash2} label="Supprimer" size="sm" onclick={() => removeHook(h)} />
                  </div>
                  <div class="h-ctl">
                    <Select
                      label="Lance l’agent"
                      value={h.agent_id == null ? '' : String(h.agent_id)}
                      options={[{ value: '', label: 'Aucun' }, ...agentList.map((a) => ({ value: String(a.id), label: a.name }))]}
                      onchange={(e) => {
                        const v = (e.currentTarget as HTMLSelectElement).value;
                        hookAct(() => patchWebhook(h.id, { agent_id: v === '' ? null : Number(v) }));
                      }}
                    />
                    <Switch checked={h.push} label="Push" onchange={() => hookAct(() => patchWebhook(h.id, { push: !h.push }))} />
                    <Switch checked={h.enabled} label="Activé" onchange={() => hookAct(() => patchWebhook(h.id, { enabled: !h.enabled }))} />
                  </div>
                </li>
              {/each}
            </ul>
          {:else if hooks}
            <EmptyState icon={Webhook_} title="Aucun webhook" text="Crée une adresse ci-dessus et colle-la dans le service qui doit te prévenir." />
          {:else}
            <Skeleton lines={3} />
          {/if}
        </div>
      </section>
    </div>
  {/if}
</Page>

<style>
  .err-line { margin: 0; padding: var(--s2) var(--s5); background: var(--danger-soft); color: var(--danger); font-size: var(--fs-sm); border-bottom: 1px solid var(--line); }
  .col { width: 100%; max-width: 860px; padding: var(--s4) var(--s5) var(--s8); display: flex; flex-direction: column; gap: var(--s3); box-sizing: border-box; }
  .col.wide { max-width: 1040px; }
  .spacer { flex: 1; }
  .faint { color: var(--faint); }
  .small { font-size: var(--fs-sm); }
  .ell { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; display: block; }

  /* Fil */
  .tools { display: flex; align-items: center; gap: var(--s2); flex-wrap: wrap; }
  .day { margin: var(--s3) 0 0; font-size: var(--fs-sm); font-weight: 600; color: var(--muted); }
  .feed { list-style: none; margin: 0; padding: 0; border: 1px solid var(--line); border-radius: var(--r); overflow: hidden; }
  .feed li { display: flex; align-items: flex-start; gap: var(--s1); border-top: 1px solid var(--line); padding-right: var(--s2); }
  .feed li:first-child { border-top: 0; }
  .feed li.unread { background: var(--panel); box-shadow: inset 2px 0 0 var(--accent); }
  .item { all: unset; box-sizing: border-box; cursor: pointer; flex: 1; min-width: 0; display: flex; gap: var(--s3); padding: var(--s3) var(--s4); }
  .item:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  .item:hover .title { color: var(--accent); }
  .ic { flex: none; display: inline-flex; margin-top: 2px; color: var(--faint); }
  .lvl-success .ic, .ic.lvl-success { color: var(--accent); }
  .lvl-warning .ic, .ic.lvl-warning { color: var(--warn); }
  .lvl-error .ic, .ic.lvl-error { color: var(--danger); }
  .main { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .top { display: flex; align-items: center; gap: var(--s2); }
  .title { font-size: var(--fs-md); color: var(--text); overflow-wrap: anywhere; }
  .body { color: var(--muted); font-size: var(--fs-sm); white-space: pre-wrap; overflow-wrap: anywhere; display: -webkit-box; -webkit-line-clamp: 3; line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; }
  .meta { display: flex; gap: var(--s3); color: var(--faint); font-size: var(--fs-sm); margin-top: 2px; }
  .feed :global(.del) { margin-top: var(--s2); }
  @media (hover: hover) and (min-width: 768px) {
    .feed :global(.del) { opacity: 0; }
    .feed li:hover :global(.del), .feed li:focus-within :global(.del) { opacity: 1; }
  }
  .more { display: flex; justify-content: center; }

  /* Réglages en sections */
  .sect { display: grid; grid-template-columns: 240px minmax(0, 1fr); gap: var(--s5); padding: var(--s5) 0; border-top: 1px solid var(--line); }
  .sect:first-child { border-top: 0; padding-top: var(--s2); }
  .s-head h2 { margin: 0 0 var(--s1); font-size: var(--fs-md); font-weight: 600; }
  .s-head p { margin: 0; color: var(--muted); font-size: var(--fs-sm); }
  .s-body { display: flex; flex-direction: column; gap: var(--s3); min-width: 0; }
  .line { display: flex; align-items: center; gap: var(--s3); }
  .callout { margin: 0; padding: var(--s3) var(--s4); background: var(--warn-soft); border-left: 2px solid var(--warn); border-radius: var(--r); font-size: var(--fs-sm); }
  .ferr { margin: 0; color: var(--danger); font-size: var(--fs-sm); }
  .foot { margin: var(--s2) 0 0; padding-top: var(--s4); border-top: 1px solid var(--line); color: var(--muted); font-size: var(--fs-sm); }
  .radio { display: flex; flex-direction: row; align-items: flex-start; gap: var(--s3); padding: var(--s3) var(--s4); border: 1px solid var(--line); border-radius: var(--r); cursor: pointer; color: var(--text); font-size: var(--fs-md); }
  .radio:hover { border-color: var(--line-strong); }
  .radio.on { border-color: var(--accent); background: var(--accent-soft); }
  .radio input { margin-top: 3px; flex: none; }
  .radio > span { display: flex; flex-direction: column; gap: 2px; }
  .r-x { color: var(--muted); font-size: var(--fs-sm); }
  .subs { list-style: none; margin: 0; padding: 0; border: 1px solid var(--line); border-radius: var(--r); }
  .subs li { display: flex; align-items: center; gap: var(--s3); padding: var(--s2) var(--s3) var(--s2) var(--s4); border-top: 1px solid var(--line); }
  .subs li:first-child { border-top: 0; }

  .hook-form { display: grid; grid-template-columns: minmax(0, 1fr) 140px auto auto; gap: var(--s3); align-items: end; }
  .sw-cell { height: 32px; display: flex; align-items: center; }
  .fresh { padding: var(--s3) var(--s4); background: var(--warn-soft); border-left: 2px solid var(--warn); border-radius: var(--r); display: flex; flex-direction: column; gap: var(--s2); font-size: var(--fs-sm); }
  .fresh p { margin: 0; }
  .url-row { display: flex; align-items: center; gap: var(--s2); }
  .url { flex: 1; min-width: 0; overflow-wrap: anywhere; background: var(--bg); border: 1px solid var(--line); border-radius: var(--r); padding: var(--s1) var(--s2); }
  .hooks { list-style: none; margin: 0; padding: 0; border: 1px solid var(--line); border-radius: var(--r); }
  .hooks li { padding: var(--s3) var(--s4); border-top: 1px solid var(--line); display: flex; flex-direction: column; gap: var(--s3); }
  .hooks li:first-child { border-top: 0; }
  .hooks li.off .h-name { color: var(--muted); }
  .h-top { display: flex; align-items: flex-start; gap: var(--s3); }
  .h-name { font-size: var(--fs-md); }
  .h-ctl { display: flex; align-items: flex-end; gap: var(--s5); flex-wrap: wrap; padding-left: 28px; }
  .h-ctl > :global(:first-child) { min-width: 220px; }

  @media (max-width: 767px) {
    .col { padding: var(--s3) var(--s4) var(--s6); }
    .item { padding: var(--s3); }
    .sect { grid-template-columns: minmax(0, 1fr); gap: var(--s3); }
    .hook-form { grid-template-columns: minmax(0, 1fr); }
    .hook-form :global(.btn) { height: var(--touch); }
    .h-ctl { padding-left: 0; gap: var(--s3); }
    .h-ctl > :global(:first-child) { min-width: 0; width: 100%; }
    .tools > :global(:first-child) { flex-basis: 100%; height: var(--touch); }
    .tools .spacer { display: none; }
    .tools :global(.btn) { height: var(--touch); flex: 1; }
    .feed :global(.del) { width: var(--touch); height: var(--touch); margin-top: 0; }
  }
</style>
