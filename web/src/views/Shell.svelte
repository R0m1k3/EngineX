<script lang="ts">
  import { changePassword, logout, message } from '../lib/api';
  import { onDestroy } from 'svelte';
  import Models from './Models.svelte';
  import Sessions from './Sessions.svelte';
  import Settings from './Settings.svelte';
  import System from './System.svelte';

  let { loggedOut }: { loggedOut: () => void } = $props();
  const later = ['Mémoire'];

  // Routage par l'ancre : #sessions/<id>, #models/presets, #settings/engine...
  const read = () => location.hash.replace(/^#/, '') || 'sessions';
  let route = $state(read());
  const onHash = () => (route = read());
  window.addEventListener('hashchange', onHash);
  onDestroy(() => window.removeEventListener('hashchange', onHash));
  let page = $derived(route.split('/')[0]);
  let sub = $derived(route.split('/')[1] ?? '');
  const pages = [
    { id: 'sessions', label: 'Sessions' },
    { id: 'models', label: 'Modèles' },
    { id: 'system', label: 'Système' },
    { id: 'settings', label: 'Paramètres' },
  ];
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
    {#each later as item}
      <span class="disabled" title="Étape suivante">{item}</span>
    {/each}
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
    {#if page === 'system'}
      <System />
    {:else if page === 'settings'}
      <Settings {sub} />
    {:else if page === 'models'}
      <Models {sub} />
    {:else}
      <Sessions {sub} />
    {/if}
  </main>
</div>
