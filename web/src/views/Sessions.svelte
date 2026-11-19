<script lang="ts">
  import type { Project, SideTab } from '../lib/api';
  import { sandboxStatus, type SandboxStatus } from '../lib/api';
  import MessageSquare from '@lucide/svelte/icons/message-square';
  import TriangleAlert from '@lucide/svelte/icons/triangle-exclamation-point';
  import Plus from '@lucide/svelte/icons/plus';
  import Button from '../lib/ui/Button.svelte';
  import EmptyState from '../lib/ui/EmptyState.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import Sheet from '../lib/ui/Sheet.svelte';
  import Tree from './sessions/Tree.svelte';
  import Thread from './sessions/Thread.svelte';
  import Side from './sessions/Side.svelte';
  import NewSession from './sessions/NewSession.svelte';
  import { screen, stored, store } from './sessions/phone.svelte';

  let { sub }: { sub: string } = $props();
  // Nombre de rafraîchissements de l'arbre demandés (titres, sessions en cours).
  let version = $state(0);
  let status = $state<SandboxStatus | null>(null);
  let projects = $state<Project[]>([]);

  sandboxStatus()
    .then((s) => (status = s))
    .catch(() => {});

  const open = (id: string) => (location.hash = `sessions/${id}`);

  // Panneau latéral (Fichiers, Terminal…) : fermé par défaut, état gardé d'une visite à l'autre.
  const TABS: SideTab[] = ['files', 'terminal', 'git', 'preview', 'browser'];
  const remembered = (() => {
    try {
      return localStorage.getItem('enginex.sessions.panel') as SideTab | null;
    } catch {
      return null;
    }
  })();
  const initial = remembered && TABS.includes(remembered) ? remembered : null;
  let side = $state<SideTab | null>(initial);
  let tab = $state<SideTab>(initial ?? 'files');
  // Téléphone : le panneau s'ouvre dans une feuille qui monte du bas.
  let sheet = $state(false);
  $effect(() => store('enginex.sessions.panel', side ?? ''));

  const panel = (t: SideTab) => {
    if (screen.phone) {
      tab = t;
      sheet = true;
    } else if (side === t) side = null;
    else side = tab = t;
  };
  const togglePanel = () => {
    if (screen.phone) sheet = !sheet;
    else side = side ? null : tab;
  };

  // Nouvelle session (bouton en tête de liste, état vide).
  let creating = $state(false);
  let suggested = $state<number | null>(null);
  const onNew = (topic?: number | null) => {
    suggested = topic ?? null;
    creating = true;
  };

  // Volets redimensionnables : largeurs gardées dans localStorage.
  let treeW = $state(stored('enginex.sessions.tree-w', 280, 200, 440));
  let sideW = $state(stored('enginex.sessions.side-w', 420, 320, 960));
  let root: HTMLDivElement | undefined = $state();

  function drag(which: 'tree' | 'side') {
    return (e: PointerEvent) => {
      const el = e.currentTarget as HTMLElement;
      el.setPointerCapture(e.pointerId);
      const x0 = e.clientX;
      const w0 = which === 'tree' ? treeW : sideW;
      const max = which === 'tree' ? 440 : Math.max(320, (root?.clientWidth ?? 1200) - treeW - 420);
      const min = which === 'tree' ? 200 : 320;
      const move = (ev: PointerEvent) => {
        const d = ev.clientX - x0;
        const w = Math.round(Math.min(max, Math.max(min, which === 'tree' ? w0 + d : w0 - d)));
        if (which === 'tree') treeW = w;
        else sideW = w;
      };
      const up = () => {
        el.removeEventListener('pointermove', move);
        el.removeEventListener('pointerup', up);
        el.removeEventListener('pointercancel', up);
        store(`enginex.sessions.${which}-w`, which === 'tree' ? treeW : sideW);
      };
      el.addEventListener('pointermove', move);
      el.addEventListener('pointerup', up);
      el.addEventListener('pointercancel', up);
    };
  }
  function nudge(which: 'tree' | 'side') {
    return (e: KeyboardEvent) => {
      const step = e.shiftKey ? 48 : 16;
      let d = e.key === 'ArrowRight' ? step : e.key === 'ArrowLeft' ? -step : 0;
      if (!d) return;
      e.preventDefault();
      if (which === 'tree') treeW = Math.min(440, Math.max(200, treeW + d));
      else sideW = Math.min(960, Math.max(320, sideW - d));
      store(`enginex.sessions.${which}-w`, which === 'tree' ? treeW : sideW);
    };
  }

  // Fil ouvert : la coquille masque sa barre d'onglets du bas au téléphone
  // (règle `html[data-thread-open] .tab-slot` côté coquille).
  $effect(() => {
    const h = document.documentElement;
    if (sub) h.dataset.threadOpen = '';
    else delete h.dataset.threadOpen;
    return () => delete h.dataset.threadOpen;
  });

  // Téléphone : le fil plein écran suit la zone visible (clavier ouvert).
  let vv = $state<{ h: number; top: number } | null>(null);
  $effect(() => {
    const v = window.visualViewport;
    if (!screen.phone || !sub || !v) {
      vv = null;
      return;
    }
    const update = () => (vv = { h: v.height, top: v.offsetTop });
    update();
    v.addEventListener('resize', update);
    v.addEventListener('scroll', update);
    return () => {
      v.removeEventListener('resize', update);
      v.removeEventListener('scroll', update);
    };
  });
</script>

<div
  class="sessions"
  class:phone={screen.phone}
  class:with-side={!!side && !!sub && !screen.phone}
  style:--tree-w="{treeW}px"
  style:--side-w="{sideW}px"
  bind:this={root}
>
  {#if !screen.phone || !sub}
    <div class="tree-slot">
      <Tree selected={sub} {version} {open} {onNew} bind:projects />
      {#if !screen.phone}
        <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
        <div class="grip" role="separator" aria-orientation="vertical" aria-label="Largeur de la liste des sessions"
          aria-valuenow={treeW} aria-valuemin={200} aria-valuemax={440} tabindex="0"
          onpointerdown={drag('tree')} onkeydown={nudge('tree')}></div>
      {/if}
    </div>
  {/if}

  {#if !screen.phone || sub}
    <div class="main" class:full={screen.phone} style:height={vv ? `${vv.h}px` : undefined} style:top={vv ? `${vv.top}px` : undefined}>
      {#if status && !status.available}
        <div class="down" role="alert">
          <Icon icon={TriangleAlert} size={18} />
          <span>
            Docker injoignable ou dossier /data introuvable : les sessions ne peuvent pas démarrer.
            {status.docker.error ?? status.host_data.error ?? ''} <a href="#settings/sessions">Détails</a>
          </span>
        </div>
      {/if}
      {#if sub}
        {#key sub}
          <Thread id={sub} changed={() => version++} {panel} {togglePanel} sideOpen={!!side} />
        {/key}
      {:else}
        <div class="pick">
          <EmptyState icon={MessageSquare} title="Aucune session ouverte"
            text="Choisissez une session dans la liste, ou ouvrez-en une : la sandbox démarre au premier message.">
            {#snippet action()}
              <Button variant="primary" icon={Plus} onclick={() => onNew()}>Nouvelle session</Button>
            {/snippet}
          </EmptyState>
        </div>
      {/if}
    </div>
  {/if}

  {#if side && sub && !screen.phone}
    {#key sub}
      <div class="side-slot">
        <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
        <div class="grip left" role="separator" aria-orientation="vertical" aria-label="Largeur du panneau"
          aria-valuenow={sideW} aria-valuemin={320} aria-valuemax={960} tabindex="0"
          onpointerdown={drag('side')} onkeydown={nudge('side')}></div>
        <Side sessionId={sub} bind:tab close={() => (side = null)} />
      </div>
    {/key}
  {/if}
</div>

{#if screen.phone && sub}
  <Sheet bind:open={sheet} title="Outils de la session">
    <div class="sheet-body"><Side sessionId={sub} bind:tab /></div>
  </Sheet>
{/if}

<NewSession bind:open={creating} {projects} topic={suggested} created={(id) => { version++; open(id); }} />

<style>
  /* Vue `fill` (shell/pages.ts) : toute la hauteur de travail, défilement dans les volets. */
  .sessions {
    position: relative;
    display: grid;
    grid-template-columns: var(--tree-w) minmax(0, 1fr);
    flex: 1;
    min-height: 0;
    background: var(--bg);
  }
  .sessions.with-side { grid-template-columns: var(--tree-w) minmax(0, 1fr) var(--side-w); }
  .tree-slot { position: relative; min-height: 0; min-width: 0; border-right: 1px solid var(--line); }
  .main { display: flex; flex-direction: column; min-height: 0; min-width: 0; }
  .side-slot { position: relative; min-height: 0; min-width: 0; border-left: 1px solid var(--line); background: var(--panel); }

  .grip {
    position: absolute;
    top: 0;
    bottom: 0;
    right: -4px;
    width: 8px;
    z-index: 5;
    cursor: col-resize;
    touch-action: none;
  }
  .grip.left { right: auto; left: -4px; }
  .grip::after { content: ''; position: absolute; top: 0; bottom: 0; left: 3px; width: 2px; background: transparent; transition: background 0.12s; }
  .grip:hover::after, .grip:focus-visible::after, .grip:active::after { background: var(--accent); }
  .grip:focus-visible { outline: none; }

  .down {
    display: flex;
    gap: var(--s2);
    align-items: flex-start;
    padding: var(--s2) var(--s5);
    background: var(--danger-soft);
    border-bottom: 1px solid var(--danger);
    color: var(--text);
    font-size: var(--fs-sm);
    flex: none;
  }
  .down :global(.ui-icon) { color: var(--danger); flex: none; margin-top: 1px; }
  .pick { flex: 1; display: grid; place-items: center; padding: var(--s6); }

  /* Fenêtre moyenne : le panneau passe par-dessus le fil. */
  @media (max-width: 1199px) {
    .sessions.with-side { grid-template-columns: var(--tree-w) minmax(0, 1fr); }
    .side-slot {
      position: absolute;
      top: 0;
      right: 0;
      bottom: 0;
      width: min(var(--side-w), calc(100% - 64px));
      z-index: 8;
      box-shadow: var(--shadow);
    }
  }

  /* Téléphone : liste plein écran, puis fil plein écran par-dessus la coquille. */
  .sessions.phone { display: block; background: none; }
  .phone .tree-slot { border-right: 0; }
  .main.full {
    position: fixed;
    left: 0;
    top: 0;
    width: 100%;
    height: 100dvh;
    z-index: 40;
    background: var(--bg);
  }
  .sheet-body { height: 76dvh; display: flex; flex-direction: column; }
</style>
