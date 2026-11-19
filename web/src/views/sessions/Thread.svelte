<script lang="ts">
  import type { SideTab } from '../../lib/api';
  import { onDestroy, tick, untrack } from 'svelte';
  import {
    sessionDetail, postTurn, interruptTurn, patchSession, presets, message,
    revertTurn, restoreUndo, restoreGreen, runVerify, bringBack,
    type Session, type Effort, type StoredItem, type TurnInfo, type ItemData, type PendingRequest, type Usage, type Approval as Mode,
    type Verification, type Preset, type QueueWait,
  } from '../../lib/api';
  import { openEvents } from '../../lib/sse';
  import Item from './Item.svelte';
  import Approval from './Approval.svelte';
  import Composer from './Composer.svelte';
  import Changes from './Changes.svelte';
  import Verify from './Verify.svelte';
  import Revert from './Revert.svelte';
  import ChevronLeft from '@lucide/svelte/icons/chevron-left';
  import Ellipsis from '@lucide/svelte/icons/ellipsis';
  import PanelRight from '@lucide/svelte/icons/panel-right';
  import GitBranch from '@lucide/svelte/icons/git-branch';
  import GitMerge from '@lucide/svelte/icons/git-merge';
  import Box from '@lucide/svelte/icons/box';
  import Folder from '@lucide/svelte/icons/folder';
  import SquareTerminal from '@lucide/svelte/icons/square-terminal';
  import Eye from '@lucide/svelte/icons/eye';
  import Globe from '@lucide/svelte/icons/globe';
  import Pencil from '@lucide/svelte/icons/pencil';
  import Settings2 from '@lucide/svelte/icons/settings-2';
  import Undo2 from '@lucide/svelte/icons/undo-2';
  import Hourglass from '@lucide/svelte/icons/hourglass';
  import WifiOff from '@lucide/svelte/icons/wifi-off';
  import Info from '@lucide/svelte/icons/info';
  import CircleX from '@lucide/svelte/icons/circle-x';
  import LoaderCircle from '@lucide/svelte/icons/loader-circle';
  import MessageSquare from '@lucide/svelte/icons/message-square';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import Badge from '../../lib/ui/Badge.svelte';
  import Button from '../../lib/ui/Button.svelte';
  import IconButton from '../../lib/ui/IconButton.svelte';
  import Icon from '../../lib/ui/Icon.svelte';
  import Segmented from '../../lib/ui/Segmented.svelte';
  import Select from '../../lib/ui/Select.svelte';
  import Sheet from '../../lib/ui/Sheet.svelte';
  import Skeleton from '../../lib/ui/Skeleton.svelte';
  import EmptyState from '../../lib/ui/EmptyState.svelte';
  import { tooltip } from '../../lib/ui/tooltip';
  import { confirmDialog } from '../../lib/ui/dialog.svelte';
  import { toast } from '../../lib/ui/toast.svelte';
  import PopMenu from './PopMenu.svelte';
  import { ask } from './ask.svelte';
  import { screen } from './phone.svelte';

  let { id, changed, panel, togglePanel, sideOpen }: {
    id: string;
    changed: () => void;
    panel: (tab: SideTab) => void;
    togglePanel: () => void;
    sideOpen: boolean;
  } = $props();

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
  let presetList = $state<Preset[]>([]);
  let wait = $state<QueueWait | null>(null);
  let scroller: HTMLDivElement | undefined = $state();
  let green = $state<string | null>(null);
  let draft = $state('');
  let reverting = $state<string | null>(null);
  // « Rétablir » vient d'être fait : le bouton défait le rétablissement.
  let redone = $state(false);
  let loading = $state(true);

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
    } finally {
      loading = false;
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

  // Textes en flux groupés : un rendu au plus toutes les 50 ms au lieu d'un
  // par jeton (sorties de commandes très bavardes, Markdown re-rendu en entier).
  let buffered: Record<string, Record<string, string>> = {};
  let flushTimer: ReturnType<typeof setTimeout> | null = null;

  function flushDeltas() {
    if (flushTimer) {
      clearTimeout(flushTimer);
      flushTimer = null;
    }
    const b = buffered;
    buffered = {};
    if (!live) return;
    let any = false;
    for (const [iid, fields] of Object.entries(b)) {
      const p = (live.partial[iid] ??= {});
      for (const [f, v] of Object.entries(fields)) p[f] = (p[f] ?? '') + v;
      any = true;
    }
    if (any) follow();
  }

  function onCodex({ method, params }: { method: string; params: any }) {
    const field = FIELD[method];
    if (field && live) {
      const p = (buffered[params.itemId] ??= {});
      p[field] = (p[field] ?? '') + (params.delta ?? '');
      flushTimer ??= setTimeout(flushDeltas, 50);
      return;
    }
    // Tout autre événement passe après les textes déjà reçus.
    flushDeltas();
    if (method === 'error') {
      notice = params?.error?.message ?? 'Erreur de Codex';
      if (params?.willRetry) notice += ' (nouvel essai en cours)';
      return;
    }
    if (!live) return;
    if (method === 'item/started' || method === 'item/completed') {
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
        wait = s.queue ?? null;
        // L'instantané contient déjà les textes reçus jusque-là.
        buffered = {};
        live = s.turn
          ? { id: s.turn.id, items: s.turn.items ?? [], partial: s.turn.partial ?? {}, usage: null }
          : null;
        loadHistory().then(() => follow());
      },
      sandbox: (d) => (sandbox = d),
      queue: (d) => (wait = d.wait ?? null),
      user_message: (d) => {
        notice = '';
        buffered = {};
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
  onDestroy(() => {
    close();
    if (flushTimer) clearTimeout(flushTimer);
  });

  presets()
    .then((ps) => (presetList = ps))
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

  async function setPreset(v: string) {
    const preset_id = v === '' ? null : Number(v);
    try {
      await patchSession(id, { preset_id });
      if (session) session.preset_id = preset_id;
    } catch (e) {
      error = message(e);
    }
  }

  const presetName = (pid: number | null) => presetList.find((p) => p.id === pid)?.name ?? `preset ${pid}`;
  const defaultName = $derived(presetList.find((p) => p.default)?.name ?? '');
  const ago = (t: number) => {
    const m = Math.max(0, Math.round((Date.now() / 1000 - t) / 60));
    return m < 1 ? 'à l’instant' : `depuis ${m} min`;
  };

  async function interruptOther(sid: string) {
    try {
      await interruptTurn(sid);
    } catch (e) {
      error = message(e);
    }
  }

  async function setEffort(effort: Effort) {
    try {
      await patchSession(id, { effort });
      if (session) session.effort = effort;
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
    if (
      !(await confirmDialog('Les fichiers reprennent l’état du dernier tour dont la vérification a réussi.', {
        title: 'Revenir au dernier état vert ?',
        confirm: 'Revenir',
      }))
    )
      return;
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
  async function bring() {
    bringing = true;
    error = '';
    try {
      const r = await bringBack(id);
      if (r.result === 'merged') toast(`Branche ${r.branch} fusionnée dans le topic.`, 'ok');
      else
        toast(
          `La branche ${r.branch} est dans le dépôt du topic, sans fusion : ${r.reason === 'topic_dirty' ? 'le topic a des modifications non commitées' : 'la fusion a des conflits'}. Fusionnez-la depuis une session du topic.`,
          'warn',
          0,
        );
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
    const t = await ask({ title: 'Renommer la session', label: 'Titre', value: session?.title ?? '', confirm: 'Renommer' });
    if (!t) return;
    try {
      await patchSession(id, { title: t });
      if (session) session.title = t;
      changed();
    } catch (e) {
      error = message(e);
    }
  }

  // En-tête : état de la sandbox, réglages, menus.
  const sandboxTone = $derived(sandbox.state === 'ready' ? 'ok' : sandbox.state === 'error' ? 'danger' : 'neutral');
  const busySandbox = $derived(['creating', 'pulling', 'starting', 'restarting'].includes(sandbox.state));
  const EFFORTS: { value: Effort; label: string }[] = [
    { value: 'none', label: 'none' },
    { value: 'low', label: 'low' },
    { value: 'medium', label: 'medium' },
    { value: 'high', label: 'high' },
  ];
  const EFFORT_HINT: Record<Effort, string> = {
    none: 'Aucune réflexion : réponses les plus rapides',
    low: 'Réflexion courte',
    medium: 'Réflexion normale (budget du preset)',
    high: 'Réflexion longue',
  };
  const modelOptions = $derived([
    { value: '', label: `Par défaut${defaultName ? ` (${defaultName})` : ''}` },
    ...presetList.map((p) => ({ value: String(p.id), label: `${p.name}${p.active ? ' · chargé' : ''}` })),
  ]);
  const modeOptions: { value: Mode; label: string }[] = [
    { value: 'auto', label: 'Automatique' },
    { value: 'prudent', label: 'Prudent (demande avant d’agir)' },
  ];
  const modelLabel = $derived(session?.preset_id == null ? defaultName || 'Modèle par défaut' : presetName(session.preset_id));

  // Téléphone : menu ⋯ et feuille des réglages.
  let moreBtn = $state<HTMLElement>();
  let moreOpen = $state(false);
  let settings = $state(false);
  const moreItems = $derived([
    { label: 'Fichiers', icon: Folder, onclick: () => panel('files') },
    { label: 'Terminal', icon: SquareTerminal, onclick: () => panel('terminal') },
    { label: 'Git', icon: GitBranch, onclick: () => panel('git') },
    { label: 'Aperçu', icon: Eye, onclick: () => panel('preview') },
    { label: 'Navigateur', icon: Globe, onclick: () => panel('browser') },
    { label: 'Réglages de la session', icon: Settings2, onclick: () => (settings = true) },
    { label: 'Renommer', icon: Pencil, onclick: rename },
    ...(session?.copy_branch
      ? [{ label: bringing ? 'Fusion…' : 'Ramener dans le topic', icon: GitMerge, onclick: bring, disabled: bringing || !!live }]
      : []),
  ]);
</script>

{#snippet sandboxBadge()}
  <span class="sb" class:busy={busySandbox}>
    <Badge tone={sandboxTone} dot title={sandbox.detail ?? undefined}>{SANDBOX[sandbox.state] ?? sandbox.state}{pull}</Badge>
  </span>
{/snippet}

{#snippet controls(big: boolean)}
  <div class="ctl" class:big>
    <Select label={big ? 'Modèle' : undefined} aria-label="Modèle de la session" size={big ? 'lg' : 'md'}
      title="Modèle de la session : changé, il est chargé au prochain message (la sandbox redémarre)"
      value={session?.preset_id == null ? '' : String(session.preset_id)} options={modelOptions}
      onchange={(e) => setPreset((e.currentTarget as HTMLSelectElement).value)} />
    {#if big}<p class="hint">Changé, il est chargé au prochain message (la sandbox redémarre).</p>{/if}
  </div>
  <div class="ctl" class:big use:tooltip={big ? null : EFFORT_HINT[session?.effort ?? 'medium']}>
    <span class="lbl">Réflexion</span>
    <Segmented label="Réflexion" size={big ? 'lg' : 'md'} value={session?.effort ?? 'medium'} options={EFFORTS} onchange={(v) => setEffort(v)} />
    {#if big}<p class="hint">{EFFORT_HINT[session?.effort ?? 'medium']}. Moins = réponses plus rapides.</p>{/if}
  </div>
  <div class="ctl" class:big>
    {#if !big}<span class="lbl">Approbation</span>{/if}
    <Select label={big ? 'Approbation' : undefined} aria-label="Approbation" size={big ? 'lg' : 'md'}
      value={session?.approval ?? 'auto'} options={modeOptions}
      onchange={(e) => setMode((e.currentTarget as HTMLSelectElement).value as Mode)} />
  </div>
{/snippet}

<section class="thread" class:phone={screen.phone}>
  {#if screen.phone}
    <header class="ph-head">
      <IconButton icon={ChevronLeft} label="Retour aux sessions" size="lg" href="#sessions" tip="bottom" />
      <h1 class="ell">{session?.title ?? '…'}</h1>
      <span bind:this={moreBtn} class="anchor">
        <IconButton icon={Ellipsis} label="Outils et réglages de la session" size="lg" tip="bottom" onclick={() => (moreOpen = !moreOpen)} />
      </span>
    </header>
    <div class="chips">
      {@render sandboxBadge()}
      <button class="chip" onclick={() => (settings = true)}><Icon icon={Box} size={14} />{modelLabel}</button>
      <button class="chip" onclick={() => (settings = true)}>Réflexion {session?.effort ?? 'medium'}</button>
      <button class="chip" onclick={() => (settings = true)}>{session?.approval === 'prudent' ? 'Prudent' : 'Automatique'}</button>
      {#if session?.copy_branch}<span class="chip static"><Icon icon={GitBranch} size={14} />{session.copy_branch}</span>{/if}
    </div>
    <PopMenu bind:open={moreOpen} anchor={moreBtn} items={moreItems} label="Outils et réglages de la session" />
    <Sheet bind:open={settings} title="Réglages de la session">
      <div class="settings">{@render controls(true)}</div>
    </Sheet>
  {:else}
    <header class="head">
      <div class="r1">
        <button class="title" onclick={rename} use:tooltip={'Renommer'}><h1 class="ell">{session?.title ?? '…'}</h1></button>
        {#if session?.copy_branch}
          <span use:tooltip={'Copie séparée'}><Badge><Icon icon={GitBranch} size={14} />{session.copy_branch}</Badge></span>
          <Button size="sm" variant="ghost" icon={GitMerge} busy={bringing} disabled={bringing || !!live} onclick={bring}>
            {bringing ? 'Fusion…' : 'Ramener dans le topic'}
          </Button>
        {/if}
        {@render sandboxBadge()}
        <IconButton icon={PanelRight} label={sideOpen ? 'Fermer le panneau' : 'Fichiers, terminal, Git, aperçu, navigateur'} pressed={sideOpen} tip="left" onclick={togglePanel} />
      </div>
      <div class="r2">{@render controls(false)}</div>
    </header>
  {/if}

  {#snippet strip(icon: any, tone: string, body: any)}
    <div class="strip {tone}" role="status"><Icon {icon} size={16} /><div class="grow">{@render body()}</div></div>
  {/snippet}

  <div class="strips">
    {#if !connected}
      {#snippet lost()}Connexion au serveur perdue, reconnexion…{/snippet}
      {@render strip(WifiOff, 'warn', lost)}
    {/if}
    {#if sandbox.detail && (sandbox.state === 'error' || sandbox.state === 'stopped' || sandbox.state === 'ready')}
      {#snippet detail()}<span class="pre">{sandbox.detail}</span>{/snippet}
      {@render strip(sandbox.state === 'error' ? CircleX : Info, sandbox.state === 'error' ? 'danger' : '', detail)}
    {/if}
    {#if wait}
      {#snippet waiting()}
        <span>
          {#if wait!.swapping}Chargement du modèle « {presetName(wait!.swapping)} »…
          {:else if wait!.running.length}En file d’attente{wait!.position > 1 ? ` (${wait!.position}e)` : ''} : le modèle sert
            {#each wait!.running as r, i (i)}{i ? ', ' : ''}« {r.label} » {ago(r.since)}{/each}{wait!.needs_swap ? ', puis changement de modèle' : ''}.
          {:else}En file d’attente{wait!.position > 1 ? ` (${wait!.position}e)` : ''}…{/if}
        </span>
        {#each wait!.running.filter((r) => r.session_id && r.priority > 0) as r (r.session_id)}
          <Button size="sm" variant="ghost" onclick={() => interruptOther(r.session_id!)}>Interrompre « {r.label} »</Button>
        {/each}
      {/snippet}
      {@render strip(Hourglass, '', waiting)}
    {/if}
    {#if session?.undo}
      {#snippet undo()}
        <span>{redone ? 'Fichiers rétablis.' : 'Fichiers remis dans un état antérieur.'}</span>
        <Button size="sm" variant="ghost" onclick={redo}>{redone ? 'Annuler le rétablissement' : 'Rétablir'}</Button>
      {/snippet}
      {@render strip(Undo2, '', undo)}
    {/if}
    {#if session?.pending_note}
      {#snippet note()}Une note sur ce retour arrière sera jointe à votre prochain message pour Codex.{/snippet}
      {@render strip(Info, '', note)}
    {/if}
    {#if session?.pending_revert}
      {#snippet rev()}Retour de conversation en attente : il sera appliqué à Codex au prochain message.{/snippet}
      {@render strip(Info, '', rev)}
    {/if}
  </div>

  <div class="scroll" bind:this={scroller}>
    <div class="in">
      {#if more}<div class="older"><Button size="sm" variant="ghost" onclick={older}>Messages plus anciens</Button></div>{/if}
      {#if loading && !history.length && !live}
        <div class="sk"><Skeleton height="56px" width="60%" /><Skeleton lines={3} /><Skeleton height="40px" /><Skeleton lines={2} /></div>
      {:else if !history.length && !live}
        <div class="empty">
          <EmptyState icon={MessageSquare} title="Écrivez votre première demande"
            text="La sandbox démarre au premier message ; elle s’arrête seule après un moment sans activité." />
        </div>
      {/if}
      {#each rows as { it, end } (it.item_id)}
        {#if it.kind === 'userMessage' && AUTO_LABEL[autoOrigin(it) ?? '']}
          <details class="auto">
            <summary><Icon icon={ChevronRight} size={16} class="chev" />{AUTO_LABEL[autoOrigin(it) ?? '']}</summary>
            <div class="pre">{it.data.text}</div>
          </details>
        {:else}
          <Item item={it.data} sessionId={id} />
        {/if}
        {#if it.kind === 'userMessage' && it.turn_id && (autoOrigin(it) ?? 'user') === 'user' && !live}
          {@const t = turns[it.turn_id]}
          <div class="msg-actions">
            <span use:tooltip={t?.has_snapshot ? null : 'Instantané absent : retour arrière indisponible pour ce message'}>
              <Button size="sm" variant="ghost" icon={Undo2} disabled={!t?.has_snapshot} onclick={() => (reverting = it.turn_id)}>Revenir avant ce message</Button>
            </span>
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
          {#if end.status === 'failed'}
            <div class="failed" role="alert">
              <div class="fh"><Icon icon={CircleX} size={18} />Tour en échec</div>
              <p class="pre">{end.error ?? 'erreur inconnue'}</p>
              {#if end.error?.includes('max_output_tokens')}
                <p class="muted">Le modèle a écrit jusqu’à la limite (souvent une réflexion qui tourne en rond).
                  Réglages du preset : « Réflexion max par réponse » et « Sortie max par réponse » (<a href="#models/presets">Modèles → Presets</a>).</p>
              {/if}
            </div>
          {/if}
          {#if end.status === 'interrupted' || tokens(end.usage)}
            <p class="foot num">
              {#if end.status === 'interrupted'}<span>Tour interrompu.</span>{/if}
              {#if tokens(end.usage)}<span>{tokens(end.usage)}</span>{/if}
            </p>
          {/if}
        {/if}
      {/each}
      {#each liveItems as it (it.id)}
        <Item item={it} sessionId={id} />
      {/each}
      {#if live}
        <p class="working num"><Icon icon={LoaderCircle} size={16} class="spin" />Codex travaille…{tokens(live.usage) ? ` ${tokens(live.usage)}` : ''}</p>
      {/if}
      {#each requests as r (r.rid)}
        <Approval req={r} sessionId={id} />
      {/each}
      {#if notice}<div class="notice-l" role="status"><Icon icon={Info} size={16} /><span class="pre">{notice}</span></div>{/if}
    </div>
  </div>

  {#if error}<p class="err" role="alert"><Icon icon={CircleX} size={16} />{error}</p>{/if}
  <Composer sessionId={id} running={!!live} disabled={session?.archived ?? false} {send} {interrupt} bind:text={draft} />
  {#if reverting}<Revert close={() => (reverting = null)} choose={revert} />{/if}
</section>

<style>
  .thread { display: flex; flex-direction: column; min-height: 0; flex: 1; background: var(--bg); }

  /* En-tête bureau */
  .head { padding: var(--s2) var(--s3) var(--s2) var(--s5); border-bottom: 1px solid var(--line); flex: none; display: flex; flex-direction: column; gap: var(--s2); }
  .r1 { display: flex; align-items: center; gap: var(--s3); min-height: 36px; }
  .r2 { display: flex; align-items: center; gap: var(--s2) var(--s5); flex-wrap: wrap; }
  .title {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    justify-content: flex-start;
    height: 32px;
    min-height: 0;
    align-items: center;
    padding: 0;
    border: 0;
    border-radius: var(--r);
    background: none;
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: text;
    filter: none;
  }
  .title:hover h1 { text-decoration: underline dotted var(--faint); text-underline-offset: 4px; }
  h1 { font-size: var(--fs-md); font-weight: 600; margin: 0; }
  .ctl { display: flex; align-items: center; gap: var(--s2); font-size: var(--fs-sm); color: var(--muted); }
  .ctl :global(select) { max-width: 240px; }
  .lbl { white-space: nowrap; }
  .sb { display: inline-flex; }
  .sb.busy :global(.badge-ui) { animation: breathe 1.6s ease-in-out infinite; }
  @keyframes breathe { 50% { opacity: 0.6; } }

  /* Bandeaux sous l'en-tête */
  .strips { flex: none; }
  .strip {
    display: flex;
    align-items: center;
    gap: var(--s2);
    padding: var(--s1) var(--s5);
    min-height: 36px;
    border-bottom: 1px solid var(--line);
    background: var(--panel);
    font-size: var(--fs-sm);
    color: var(--text);
  }
  .strip > :global(.ui-icon) { color: var(--faint); flex: none; }
  .strip .grow { display: flex; align-items: center; gap: var(--s2); flex-wrap: wrap; justify-content: space-between; }
  .strip.warn { background: var(--warn-soft); }
  .strip.warn > :global(.ui-icon) { color: var(--warn); }
  .strip.danger { background: var(--danger-soft); }
  .strip.danger > :global(.ui-icon) { color: var(--danger); }
  .pre { white-space: pre-wrap; overflow-wrap: anywhere; }

  /* Fil */
  .scroll { flex: 1; min-height: 0; overflow: auto; padding: var(--s6) var(--s6) var(--s4); overscroll-behavior: contain; }
  .in { max-width: 760px; margin: 0 auto; display: flex; flex-direction: column; gap: var(--s4); }
  /* Appels d'outils consécutifs : une seule pile à filets. */
  .in :global(.toolrow + .toolrow) { margin-top: calc(-1 * var(--s4) - 1px); border-top-left-radius: 0; border-top-right-radius: 0; }
  .in :global(.toolrow:has(+ .toolrow)) { border-bottom-left-radius: 0; border-bottom-right-radius: 0; }
  .older { display: flex; justify-content: center; }
  .sk { display: flex; flex-direction: column; gap: var(--s4); }
  .sk > :global(:first-child) { align-self: flex-end; }
  .empty { display: grid; place-items: center; padding: var(--s8) 0; }
  .msg-actions { align-self: flex-end; margin-top: calc(-1 * var(--s3)); opacity: 0; transition: opacity 0.12s; }
  :global(.user:hover) + .msg-actions, .msg-actions:hover, .msg-actions:focus-within { opacity: 1; }
  .auto { border-left: 2px solid var(--warn); padding-left: var(--s3); font-size: var(--fs-sm); color: var(--muted); }
  .auto summary { list-style: none; display: flex; align-items: center; gap: var(--s2); min-height: 30px; cursor: pointer; }
  .auto summary::-webkit-details-marker { display: none; }
  .auto summary:hover { color: var(--text); }
  .auto :global(.chev) { color: var(--faint); transition: transform 0.12s; flex: none; }
  .auto[open] :global(.chev) { transform: rotate(90deg); }
  .auto .pre { max-height: 320px; overflow: auto; padding: var(--s1) 0 var(--s2) var(--s6); }
  .failed { border: 1px solid var(--danger); background: var(--danger-soft); border-radius: var(--r); padding: var(--s3) var(--s4); display: flex; flex-direction: column; gap: var(--s2); font-size: var(--fs-sm); }
  .failed p { margin: 0; }
  .fh { display: flex; align-items: center; gap: var(--s2); font-weight: 600; font-size: var(--fs-md); }
  .fh :global(.ui-icon) { color: var(--danger); }
  .foot { display: flex; gap: var(--s3); flex-wrap: wrap; margin: calc(-1 * var(--s2)) 0 0; font-size: var(--fs-xs); color: var(--faint); }
  .working { display: flex; align-items: center; gap: var(--s2); margin: 0; font-size: var(--fs-sm); color: var(--muted); }
  .working :global(.spin) { color: var(--accent); animation: spin 1s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .notice-l { display: flex; gap: var(--s2); align-items: flex-start; border: 1px solid var(--line-strong); border-radius: var(--r); padding: var(--s2) var(--s3); font-size: var(--fs-sm); background: var(--panel); }
  .notice-l :global(.ui-icon) { color: var(--faint); flex: none; margin-top: 2px; }
  .err { display: flex; align-items: center; gap: var(--s2); margin: 0 auto; padding: var(--s2) var(--s6) 0; width: 100%; max-width: calc(760px + 2 * var(--s6)); color: var(--danger); font-size: var(--fs-sm); }
  .err :global(.ui-icon) { flex: none; }

  /* Téléphone */
  .ph-head {
    display: flex;
    align-items: center;
    gap: var(--s1);
    min-height: 56px;
    padding: var(--safe-top) calc(var(--s1) + var(--safe-right)) 0 calc(var(--s1) + var(--safe-left));
    border-bottom: 1px solid var(--line);
    background: var(--panel);
    flex: none;
  }
  .ph-head h1 { flex: 1; min-width: 0; font-size: var(--fs-md); }
  .anchor { display: inline-flex; }
  .chips { display: flex; gap: var(--s2); overflow-x: auto; scrollbar-width: none; padding: var(--s2) var(--s4); border-bottom: 1px solid var(--line); flex: none; align-items: center; }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 32px;
    min-height: 0;
    padding: 0 var(--s3);
    border: 0;
    border-radius: var(--r);
    background: var(--raised);
    color: var(--muted);
    font: inherit;
    font-size: var(--fs-sm);
    font-weight: 400;
    white-space: nowrap;
    flex: none;
    filter: none;
    cursor: pointer;
  }
  .chip.static { cursor: default; }
  .chips .sb :global(.badge-ui) { height: 32px; font-size: var(--fs-sm); }
  .settings { display: flex; flex-direction: column; gap: var(--s5); padding: var(--s4) calc(var(--s4) + var(--safe-right)) var(--s6) calc(var(--s4) + var(--safe-left)); }
  .ctl.big { flex-direction: column; align-items: stretch; gap: var(--s2); color: var(--muted); }
  .ctl.big :global(select) { max-width: none; width: 100%; }
  .ctl.big :global(.seg) { display: flex; }
  .ctl.big :global(.seg button) { flex: 1; }
  .ctl.big .hint { margin: 0; color: var(--faint); font-size: var(--fs-xs); }
  .phone .scroll { padding: var(--s4) calc(var(--s4) + var(--safe-right)) var(--s4) calc(var(--s4) + var(--safe-left)); }
  .phone .msg-actions { opacity: 1; }
  .phone .err { padding: var(--s2) var(--s4) 0; }

  @media (prefers-reduced-motion: reduce) {
    .working :global(.spin), .sb.busy :global(.badge-ui) { animation: none; }
  }
</style>
