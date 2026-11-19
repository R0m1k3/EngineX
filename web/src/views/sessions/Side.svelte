<script lang="ts">
  import type { SideTab } from '../../lib/api';
  import Files from './Files.svelte';
  import SessionGit from './SessionGit.svelte';
  import Preview from './Preview.svelte';
  import Browser from './Browser.svelte';
  // xterm.js n'est chargé qu'à l'ouverture du terminal.
  const terminal = () => import('./Terminal.svelte');

  let { sessionId, tab = $bindable(), close }: { sessionId: string; tab: SideTab; close: () => void } = $props();
  // Le terminal reste monté (connexion gardée) une fois ouvert.
  let termOpened = $state(false);
  $effect(() => {
    if (tab === 'terminal') termOpened = true;
  });
</script>

<aside class="side card">
  <div class="row">
    <div class="tabs" role="tablist">
      <a href={'#'} class:active={tab === 'files'} onclick={(e) => { e.preventDefault(); tab = 'files'; }}>Fichiers</a>
      <a href={'#'} class:active={tab === 'terminal'} onclick={(e) => { e.preventDefault(); tab = 'terminal'; }}>Terminal</a>
      <a href={'#'} class:active={tab === 'git'} onclick={(e) => { e.preventDefault(); tab = 'git'; }}>Git</a>
      <a href={'#'} class:active={tab === 'preview'} onclick={(e) => { e.preventDefault(); tab = 'preview'; }}>Aperçu</a>
      <a href={'#'} class:active={tab === 'browser'} onclick={(e) => { e.preventDefault(); tab = 'browser'; }}>Navigateur</a>
    </div>
    <button class="link" onclick={close} title="Fermer le panneau">✕</button>
  </div>
  <div class="body" class:hidden={tab !== 'files'}><Files {sessionId} /></div>
  {#if tab === 'git'}<div class="body"><SessionGit {sessionId} /></div>{/if}
  {#if tab === 'preview'}<div class="body"><Preview {sessionId} /></div>{/if}
  {#if tab === 'browser'}<div class="body"><Browser {sessionId} /></div>{/if}
  {#if termOpened}
    <div class="body" class:hidden={tab !== 'terminal'}>
      {#await terminal()}<p class="muted small">Chargement du terminal…</p>
      {:then m}<m.default {sessionId} />
      {:catch}<p class="error small">Terminal introuvable : rechargez la page.</p>{/await}
    </div>
  {/if}
</aside>

<style>
  .side { display: flex; flex-direction: column; gap: var(--s3); min-height: 0; height: 100%; padding: var(--s3); }
  .body { flex: 1; min-height: 0; }
  .hidden { display: none; }
</style>
