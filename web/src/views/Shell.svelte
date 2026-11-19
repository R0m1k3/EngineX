<script lang="ts">
  import { changePassword, logout, message } from '../lib/api';
  import { onDestroy } from 'svelte';
  import type { Component } from 'svelte';
  import Dashboard from './Dashboard.svelte';
  import { openEvents } from '../lib/sse';
  import Sessions from './Sessions.svelte';

  // Pages moins courantes chargées à la demande (premier affichage plus léger).
  type Loader = () => Promise<{ default: Component<any> }>;
  const LAZY: Record<string, Loader> = {
    feed: () => import('./Feed.svelte'),
    system: () => import('./System.svelte'),
    settings: () => import('./Settings.svelte'),
    tasks: () => import('./Tasks.svelte'),
    crons: () => import('./Crons.svelte'),
    agents: () => import('./Agents.svelte'),
    files: () => import('./Files.svelte'),
    git: () => import('./Git.svelte'),
    memory: () => import('./Memory.svelte'),
    models: () => import('./Models.svelte'),
  };
  const loaded: Record<string, Promise<Component<any>>> = {};
  const view = (id: string) => (loaded[id] ??= LAZY[id]().then((m) => m.default));

  let { loggedOut }: { loggedOut: () => void } = $props();

  // Routage par l'ancre : #sessions/<id>, #models/presets, #settings/engine...
  const read = () => location.hash.replace(/^#/, '') || 'dashboard';
  let route = $state(read());
  const onHash = () => (route = read());
  window.addEventListener('hashchange', onHash);
  onDestroy(() => window.removeEventListener('hashchange', onHash));
  let page = $derived(route.split('/')[0]);
  let sub = $derived(route.split('/')[1] ?? '');
  const pages = [
    { id: 'dashboard', label: 'Tableau de bord' },
    { id: 'sessions', label: 'Sessions' },
    { id: 'tasks', label: 'Tâches' },
    { id: 'crons', label: 'Crons' },
    { id: 'agents', label: 'Agents' },
    { id: 'files', label: 'Fichiers' },
    { id: 'git', label: 'Git' },
    { id: 'memory', label: 'Mémoire' },
    { id: 'models', label: 'Modèles' },
    { id: 'system', label: 'Système' },
    { id: 'settings', label: 'Paramètres' },
  ];
  // Non lues du fil, en direct.
  let unread = $state(0);
  const closeFeed = openEvents('/api/feed/events', { feed: (d) => typeof d.unread === 'number' && (unread = d.unread) });
  onDestroy(closeFeed);
  let showPassword = $state(false);
  let current = $state('');
  let next = $state('');
  let pwMessage = $state('');

  async function doLogout() {
    try {
      await logout();
    } finally {
      loggedOut();
    }
  }

  async function submitPassword(e: SubmitEvent) {
    e.preventDefault();
    pwMessage = '';
    try {
      await changePassword(current, next);
      pwMessage = 'Mot de passe changé. Les autres sessions sont fermées.';
      current = '';
      next = '';
    } catch (err) {
      pwMessage = message(err);
    }
  }
</script>

<div class="layout">
  <nav>
    <div class="brand">EngineX</div>
    {#each pages as p}
      <a class:active={page === p.id} href="#{p.id}" aria-current={page === p.id ? 'page' : undefined}>{p.label}</a>
    {/each}
    <a class:active={page === 'feed'} href="#feed" class="bell" title="Notifications">
      Notifications{#if unread}<span class="count">{unread > 99 ? '99+' : unread}</span>{/if}
    </a>
    <div class="spacer"></div>
    <button class="link" onclick={() => (showPassword = !showPassword)}>Mot de passe</button>
    <button class="link" onclick={doLogout}>Déconnexion</button>
  </nav>
  <main>
    {#if showPassword}
      <form class="card narrow" onsubmit={submitPassword}>
        <h2>Changer le mot de passe</h2>
        <label>Mot de passe actuel <input type="password" bind:value={current} autocomplete="current-password" required /></label>
        <label>Nouveau (12 caractères minimum) <input type="password" bind:value={next} autocomplete="new-password" minlength="12" required /></label>
        {#if pwMessage}<p class="muted">{pwMessage}</p>{/if}
        <button type="submit">Changer</button>
      </form>
    {/if}
    {#if page === 'dashboard'}
      <Dashboard />
    {:else if LAZY[page]}
      {#await view(page) then View}
        <View sub={page === 'git' ? route.split('/').slice(1).join('/') : sub} />
      {:catch e}
        <p class="error">Page impossible à charger : {message(e)}</p>
      {/await}
    {:else}
      <Sessions {sub} />
    {/if}
  </main>
</div>

<style>
  .bell { margin-top: var(--s2); display: flex; align-items: center; gap: var(--s2); }
  .count { background: var(--accent); color: var(--bg); border-radius: 9px; padding: 0 6px; font-size: 12px; font-weight: 600; line-height: 18px; }
</style>
