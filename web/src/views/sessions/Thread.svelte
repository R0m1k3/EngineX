<script lang="ts">
  import type { SideTab } from '../../lib/api';
  import { onDestroy, tick, untrack } from 'svelte';
  import {
    sessionDetail, postTurn, interruptTurn, patchSession, presets, message,
    revertTurn, restoreUndo, restoreGreen, runVerify, bringBack,
    type Session, type StoredItem, type TurnInfo, type ItemData, type PendingRequest, type Usage, type Approval as Mode,
    type Verification,
  } from '../../lib/api';
  import { openEvents } from '../../lib/sse';
  import Item from './Item.svelte';
  import Approval from './Approval.svelte';
  import Composer from './Composer.svelte';
  import Changes from './Changes.svelte';
  import Verify from './Verify.svelte';
  import Revert from './Revert.svelte';

  let { id, changed, panel }: { id: string; changed: () => void; panel: (tab: SideTab) => void } = $props();

  interface Live {
    id: string | null;
    items: ItemData[];
    partial: Record<string, Record<string, string>>;
    usage: Usage | null;
  }
  interface Sandbox {
    state: string;
    detail?: string | null;
    progress?: { done: number; total: number | null };
  }

  let session = $state<Session | null>(null);
  let history = $state<StoredItem[]>([]);
  let turns = $state<Record<string, TurnInfo>>({});
  let more = $state(false);
  let live = $state<Live | null>(null);
  let requests = $state<PendingRequest[]>([]);
  let sandbox = $state<Sandbox>({ state: 'stopped' });
  let connected = $state(true);
  let error = $state('');
  let notice = $state('');
  let preset = $state('');
  let scroller: HTMLDivElement | undefined = $state();
  let green = $state<string | null>(null);
  let draft = $state('');
  let reverting = $state<string | null>(null);
  // « Rétablir » vient d'être fait : le bouton défait le rétablissement.
  let redone = $state(false);

  async function loadHistory() {
    try {
      const d = await sessionDetail(id);
      session = d.session;
      history = d.items;
      more = d.more;
      turns = Object.fromEntries(d.turns.map((t) => [t.id, t]));
      green = d.green_turn;
    } catch (e) {
      error = message(e);
    }
  }

  async function older() {
    if (!history.length) return;
    try {
      const d = await sessionDetail(id, history[0].seq);
      history = [...d.items, ...history];
      more = d.more;
    } catch (e) {
      error = message(e);
    }
  }

  const FIELD: Record<string, string> = {
    'item/agentMessage/delta': 'text',
    'item/reasoning/textDelta': 'reasoning',
    'item/reasoning/summaryTextDelta': 'reasoning',
    'item/commandExecution/outputDelta': 'output',
  };

  function onCodex({ method, params }: { method: string; params: any }) {
    if (method === 'error') {
      notice = params?.error?.message ?? 'Erreur de Codex';
      if (params?.willRetry) notice += ' (nouvel essai en cours)';
      return;
    }
    if (!live) return;
    const field = FIELD[method];
    if (field) {
      const p = (live.partial[params.itemId] ??= {});
      p[field] = (p[field] ?? '') + (params.delta ?? '');
    } else if (method === 'item/started' || method === 'item/completed') {
      const item = params.item;
      if (!item || item.type === 'userMessage') return;
      const i = live.items.findIndex((x) => x.id === item.id);
      if (i >= 0) live.items[i] = item;
      else live.items.push(item);
      if (method === 'item/completed') delete live.partial[item.id];
    } else if (method === 'turn/started') {
      live.id = params.turn?.id ?? live.id;
    } else if (method === 'thread/tokenUsage/updated') {
      live.usage = params.tokenUsage;
    }
    follow();
  }

  // Le parent recrée ce composant à chaque changement de session ({#key}).
  const sid = untrack(() => id);
  const close = openEvents(
    `/api/sessions/${sid}/events`,
    {
      snapshot: (s) => {
        if (s.session) session = s.session;
        sandbox = s.sandbox ?? { state: 'stopped' };
        requests = s.requests ?? [];
        live = s.turn
          ? { id: s.turn.id, items: s.turn.items ?? [], partial: s.turn.partial ?? {}, usage: null }
          : null;
        loadHistory().then(() => follow());
      },
      sandbox: (d) => (sandbox = d),
      user_message: (d) => {
        notice = '';
        live = { id: null, items: [d.item], partial: {}, usage: null };
        follow(true);
      },
      codex: onCodex,
      request: (r) => {
        requests = [...requests.filter((x) => x.rid !== r.rid), r];
        follow();
      },
      request_resolved: (r) => (requests = requests.filter((x) => x.rid !== r.rid)),
      changes: (d) => {
        const t = turns[d.turn_id];
        if (t) {
          t.changes = d.changes;
          t.has_post = true;
        }
      },
      verify: (d: Verification & { turn_id: string }) => {
        const t = turns[d.turn_id];
        if (t) {
          const list = (t.verifications ??= []);
          const i = list.findIndex((v) => v.id === d.id);
          if (i >= 0) list[i] = d;
          else list.push(d);
        }
        if (d.status === 'passed') green = d.turn_id;
        follow();
      },
      session: (d) => {
        session = d;
        if (!d.undo) redone = false;
      },
      history: () => loadHistory(),
      notice: () => loadHistory().then(() => follow()),
      turn_stored: async (d) => {
        turns[d.turn.id] = { ...turns[d.turn.id], ...d.turn };
        await loadHistory();
        live = null;
        requests = [];
        changed();
        follow();
      },
    },
    (open) => (connected = open),
  );
  onDestroy(close);

  presets()
    .then((ps) => (preset = ps.find((p) => p.active)?.name ?? ''))
    .catch(() => {});

  // Défile en bas si l'on y était déjà (ou si l'on vient d'envoyer).
  async function follow(force = false) {
    const el = scroller;
    if (!el) return;
    const near = el.scrollHeight - el.scrollTop - el.clientHeight < 120;
    await tick();
    if (force || near) el.scrollTop = el.scrollHeight;
  }

  function shown(it: ItemData): ItemData {
    const p = live?.partial[it.id];
    if (!p) return it;
    if (it.type === 'agentMessage' && p.text) return { ...it, text: p.text };
    if (it.type === 'commandExecution' && p.output) return { ...it, aggregatedOutput: (it.aggregatedOutput ?? '') + p.output };
    if (it.type === 'reasoning' && p.reasoning) return { ...it, _live: p.reasoning };
    return it;
  }

  // Éléments du tour en cours, plus les textes en flux dont l'élément n'est pas encore annoncé.
  let liveItems = $derived.by(() => {
    if (!live) return [];
    // Les éléments déjà stockés (relus avec l'historique) ne sont pas répétés.
    const stored = new Set(history.map((h) => h.item_id));
    const out = live.items.filter((x) => !stored.has(x.id)).map(shown);
    for (const [iid, p] of Object.entries(live.partial)) {
      if (stored.has(iid) || live.items.some((x) => x.id === iid)) continue;
      if (p.text) out.push({ type: 'agentMessage', id: iid, text: p.text });
      else if (p.reasoning) out.push({ type: 'reasoning', id: iid, _live: p.reasoning });
    }
    return out;
  });

  // Bas de tour (erreur, interruption, jetons) après le dernier élément de chaque tour.
  let rows = $derived(
    history.map((it, i) => {
      const next = history[i + 1];
      const end = it.turn_id && (!next || next.turn_id !== it.turn_id) ? turns[it.turn_id] : undefined;
      return { it, end };
    }),
  );

  const tokens = (u: Usage | null) =>
    u?.last ? `${u.last.inputTokens.toLocaleString('fr-FR')} jetons lus, ${u.last.outputTokens.toLocaleString('fr-FR')} écrits` : '';

  const SANDBOX: Record<string, string> = {
    stopped: 'Sandbox arrêtée',
    creating: 'Création de la sandbox…',
    pulling: 'Téléchargement de l’image devbox…',
    starting: 'Démarrage de la sandbox…',
    restarting: 'Redémarrage de la sandbox…',
    ready: 'Sandbox prête',
    error: 'Sandbox en erreur',
  };
  let pull = $derived(
    sandbox.state === 'pulling' && sandbox.progress?.total
      ? ` ${Math.floor((100 * sandbox.progress.done) / sandbox.progress.total)} %`
      : '',
  );

  async function send(text: string, attachments: string[]) {
    error = '';
    try {
      await postTurn(id, text, attachments);
      return true;
    } catch (e) {
      error = message(e);
      return false;
    }
  }

  async function interrupt() {
    try {
      await interruptTurn(id);
    } catch (e) {
      error = message(e);
    }
  }

  async function setMode(mode: Mode) {
    try {
      await patchSession(id, { approval: mode });
      if (session) session.approval = mode;
    } catch (e) {
      error = message(e);
    }
  }

  async function revert(conversation: boolean) {
    const tid = reverting;
    reverting = null;
    if (!tid) return;
    error = '';
    try {
      const r = await revertTurn(id, tid, conversation);
      if (conversation && r.user?.text && !draft.trim()) draft = r.user.text;
      redone = false;
      await loadHistory();
      changed();
    } catch (e) {
      error = message(e);
    }
  }

  async function redo() {
    error = '';
    try {
      await restoreUndo(id);
      redone = !redone;
    } catch (e) {
      error = message(e);
    }
  }

  async function toGreen() {
    if (!confirm('Remettre les fichiers dans l’état du dernier tour vérifié ?')) return;
    error = '';
    try {
      await restoreGreen(id);
      redone = false;
    } catch (e) {
      error = message(e);
    }
  }

  async function rerun() {
    error = '';
    try {
      await runVerify(id);
    } catch (e) {
      error = message(e);
    }
  }

  let bringing = $state(false);
  let brought = $state('');
  async function bring() {
    bringing = true;
    error = '';
    brought = '';
    try {
      const r = await bringBack(id);
      brought =
        r.result === 'merged'
          ? `Branche ${r.branch} fusionnée dans le topic.`
          : `La branche ${r.branch} est dans le dépôt du topic, sans fusion : ${r.reason === 'topic_dirty' ? 'le topic a des modifications non commitées' : 'la fusion a des conflits'}. Fusionnez-la depuis une session du topic.`;
    } catch (e) {
      error = message(e);
    } finally {
      bringing = false;
    }
  }

  // Tour lancé par EngineX (vérification, mémoire de fin de session) : message replié.
  const autoOrigin = (it: { turn_id: string | null; data: ItemData }) =>
    (it.data as { origin?: string }).origin ?? (it.turn_id ? turns[it.turn_id]?.origin : undefined);
  const AUTO_LABEL: Record<string, string> = {
    verify: 'Vérification EngineX : rapport d’échec envoyé à Codex',
    memory: 'Mise à jour de la mémoire en fin de session',
  };

  async function rename() {
    const t = prompt('Titre de la session', session?.title ?? '');
    if (!t?.trim()) return;
    try {
      await patchSession(id, { title: t.trim() });
      if (session) session.title = t.trim();
      changed();
    } catch (e) {
      error = message(e);
    }
  }
</script>

<section class="thread">
  <header class="row head">
    <button class="link title" title="Renommer" onclick={rename}>{session?.title ?? '…'}</button>
    <div class="actions small">
      {#if session?.copy_branch}
        <span class="muted" title="Copie séparée">⎇ {session.copy_branch}</span>
        <button class="link" onclick={bring} disabled={bringing || !!live}>{bringing ? 'Fusion…' : 'Ramener dans le topic'}</button>
      {/if}
      <button class="link" onclick={() => panel('files')}>Fichiers</button>
      <button class="link" onclick={() => panel('terminal')}>Terminal</button>
      <button class="link" onclick={() => panel('git')}>Git</button>
      <button class="link" onclick={() => panel('preview')}>Aperçu</button>
      <button class="link" onclick={() => panel('browser')}>Navigateur</button>
      <span class="pill state-{sandbox.state}" title={sandbox.detail ?? ''}>{SANDBOX[sandbox.state] ?? sandbox.state}{pull}</span>
      {#if preset}<span class="muted" title="Preset actif (Modèles)">{preset}</span>{/if}
      <label class="mode">
        Approbation
        <select value={session?.approval ?? 'auto'} onchange={(e) => setMode((e.currentTarget as HTMLSelectElement).value as Mode)}>
          <option value="auto">Automatique</option>
          <option value="prudent">Prudent (demande avant d’agir)</option>
        </select>
      </label>
    </div>
  </header>

  {#if !connected}<p class="banner">Connexion au serveur perdue, reconnexion…</p>{/if}
  {#if sandbox.detail && (sandbox.state === 'error' || sandbox.state === 'stopped' || sandbox.state === 'ready')}
    <p class="banner" class:bad={sandbox.state === 'error'}>{sandbox.detail}</p>
  {/if}

  {#if brought}<p class="banner small">{brought}</p>{/if}
  {#if session?.undo}
    <p class="banner small row">
      <span>{redone ? 'Fichiers rétablis.' : 'Fichiers remis dans un état antérieur.'}</span>
      <button class="link" onclick={redo}>{redone ? 'Annuler le rétablissement' : 'Rétablir'}</button>
    </p>
  {/if}
  {#if session?.pending_note}<p class="banner small">Une note sur ce retour arrière sera jointe à votre prochain message pour Codex.</p>{/if}
  {#if session?.pending_revert}<p class="banner small">Retour de conversation en attente : il sera appliqué à Codex au prochain message.</p>{/if}

  <div class="scroll" bind:this={scroller}>
    {#if more}<button class="link small" onclick={older}>Messages plus anciens</button>{/if}
    {#if !history.length && !live}
      <p class="muted empty">Écrivez une demande ci-dessous. La sandbox démarre au premier message ; elle s’arrête seule après un moment sans activité.</p>
    {/if}
    {#each rows as { it, end } (it.item_id)}
      {#if it.kind === 'userMessage' && AUTO_LABEL[autoOrigin(it) ?? '']}
        <details class="fix">
          <summary class="small">{AUTO_LABEL[autoOrigin(it) ?? '']}</summary>
          <div class="pre small">{it.data.text}</div>
        </details>
      {:else}
        <Item item={it.data} sessionId={id} />
      {/if}
      {#if it.kind === 'userMessage' && it.turn_id && (autoOrigin(it) ?? 'user') === 'user' && !live}
        {@const t = turns[it.turn_id]}
        <div class="msg-actions small">
          <button class="link" disabled={!t?.has_snapshot} title={t?.has_snapshot ? '' : 'Instantané absent : retour arrière indisponible pour ce message'}
            onclick={() => (reverting = it.turn_id)}>Revenir avant ce message</button>
        </div>
      {/if}
      {#if end}
        {#if end.changes?.files.length}
          <Changes sessionId={id} turnId={end.id} changes={end.changes} canUndo={!!end.has_snapshot && !!end.has_post && !live} />
        {/if}
        {#if end.verifications?.length}
          <Verify checks={end.verifications} green={green === end.id}
            rerun={live ? null : rerun} toGreen={green && green !== end.id && !live ? toGreen : null} />
        {/if}
        {#if end.status === 'failed'}<p class="banner bad small">Tour en échec : {end.error ?? 'erreur inconnue'}
          {#if end.error?.includes('max_output_tokens')}<br />Le modèle a écrit jusqu’à la limite (souvent une réflexion qui tourne en rond).
            Réglages du preset : « Réflexion max par réponse » et « Sortie max par réponse » (Modèles → Presets).{/if}</p>
        {:else if end.status === 'interrupted'}<p class="muted small">Tour interrompu.</p>{/if}
        {#if tokens(end.usage)}<p class="muted tiny">{tokens(end.usage)}</p>{/if}
      {/if}
    {/each}
    {#each liveItems as it (it.id)}
      <Item item={it} sessionId={id} />
    {/each}
    {#if live}<p class="muted small working">Codex travaille…{tokens(live.usage) ? ` ${tokens(live.usage)}` : ''}</p>{/if}
    {#each requests as r (r.rid)}
      <Approval req={r} sessionId={id} />
    {/each}
    {#if notice}<p class="banner small">{notice}</p>{/if}
  </div>

  {#if error}<p class="error small">{error}</p>{/if}
  <Composer sessionId={id} running={!!live} disabled={session?.archived ?? false} {send} {interrupt} bind:text={draft} />
  {#if reverting}<Revert close={() => (reverting = null)} choose={revert} />{/if}
</section>

<style>
  .thread { display: flex; flex-direction: column; gap: var(--s3); min-height: 0; flex: 1; }
  .head { gap: var(--s3); flex-wrap: wrap; }
  .title { font-size: 17px; font-weight: 600; color: var(--text); }
  .scroll { flex: 1; overflow: auto; display: flex; flex-direction: column; gap: var(--s3); padding-right: var(--s2); min-height: 0; }
  .pill { border: 1px solid var(--line); border-radius: 999px; padding: 0 var(--s2); }
  .pill.state-ready { border-color: var(--accent); color: var(--accent); }
  .pill.state-error { border-color: var(--error); color: var(--error); }
  .mode { flex-direction: row; align-items: center; gap: var(--s2); }
  .banner { border: 1px solid var(--line); border-radius: var(--radius); padding: var(--s2) var(--s3); margin: 0; white-space: pre-wrap; }
  .banner.bad { border-color: var(--error); color: var(--error); }
  .tiny { font-size: 12px; margin: 0; }
  .empty { margin-top: var(--s6); text-align: center; }
  .working { margin: 0; }
  .msg-actions { align-self: flex-end; margin-top: calc(-1 * var(--s2)); }
  .fix { border-left: 2px solid #d9b36c; padding-left: var(--s3); }
  .fix summary { cursor: pointer; }
  .fix .pre { white-space: pre-wrap; margin-top: var(--s1); max-height: 320px; overflow: auto; }
  .banner.row { justify-content: space-between; gap: var(--s3); }
</style>
