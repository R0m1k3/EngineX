<script lang="ts">
  import type { SideTab } from '../../lib/api';
  import Folder from '@lucide/svelte/icons/folder';
  import SquareTerminal from '@lucide/svelte/icons/square-terminal';
  import GitBranch from '@lucide/svelte/icons/git-branch';
  import Eye from '@lucide/svelte/icons/eye';
  import Globe from '@lucide/svelte/icons/globe';
  import PanelRightClose from '@lucide/svelte/icons/panel-right-close';
  import Tabs from '../../lib/ui/Tabs.svelte';
  import IconButton from '../../lib/ui/IconButton.svelte';
  import Skeleton from '../../lib/ui/Skeleton.svelte';
  import Files from './Files.svelte';
  import SessionGit from './SessionGit.svelte';
  import Preview from './Preview.svelte';
  import Browser from './Browser.svelte';
  // xterm.js n'est chargé qu'à l'ouverture du terminal.
  const terminal = () => import('./Terminal.svelte');

  let { sessionId, tab = $bindable(), close }: { sessionId: string; tab: SideTab; close?: () => void } = $props();
  // Le terminal reste monté (connexion gardée) une fois ouvert.
  let termOpened = $state(false);
  $effect(() => {
    if (tab === 'terminal') termOpened = true;
  });

  const items = [
    { id: 'files', label: 'Fichiers', icon: Folder },
    { id: 'terminal', label: 'Terminal', icon: SquareTerminal },
    { id: 'git', label: 'Git', icon: GitBranch },
    { id: 'preview', label: 'Aperçu', icon: Eye },
    { id: 'browser', label: 'Navigateur', icon: Globe },
  ];
  let active = $state<string>(tab);
  $effect(() => {
    active = tab;
  });
</script>

<aside class="side" aria-label="Outils de la session">
  <div class="bar">
    <div class="tabs"><Tabs {items} bind:active label="Outils de la session" compact onchange={(id) => (tab = id as SideTab)} /></div>
    {#if close}<IconButton icon={PanelRightClose} label="Fermer le panneau" tip="left" onclick={close} />{/if}
  </div>
  <div class="body" class:hidden={tab !== 'files'}><Files {sessionId} /></div>
  {#if tab === 'git'}<div class="body"><SessionGit {sessionId} /></div>{/if}
  {#if tab === 'preview'}<div class="body"><Preview {sessionId} /></div>{/if}
  {#if tab === 'browser'}<div class="body"><Browser {sessionId} /></div>{/if}
  {#if termOpened}
    <div class="body" class:hidden={tab !== 'terminal'}>
      {#await terminal()}<div class="wait"><Skeleton height="100%" /></div>
      {:then m}<m.default {sessionId} />
      {:catch}<p class="error small wait">Terminal introuvable : rechargez la page.</p>{/await}
    </div>
  {/if}
</aside>

<style>
  .side { display: flex; flex-direction: column; min-height: 0; height: 100%; background: var(--panel); }
  .bar { display: flex; align-items: center; gap: var(--s1); padding-right: var(--s1); border-bottom: 1px solid var(--line); flex: none; }
  .tabs { flex: 1; min-width: 0; }
  /* Le filet du bas est porté par la barre (bouton fermer compris). */
  .tabs :global(.tabs-ui) { border-bottom: 0; }
  .body { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .hidden { display: none; }
  .wait { padding: var(--s3); flex: 1; }
  @media (max-width: 767px) {
    .tabs :global([role='tab']) { height: var(--touch); padding: 0 var(--s4); }
  }
</style>
