<script lang="ts">
  import { changePassword, logout, message } from '../lib/api';
  import { onDestroy } from 'svelte';
  import type { Component } from 'svelte';
  import Dashboard from './Dashboard.svelte';
  import { openEvents } from '../lib/sse';
  import Sessions from './Sessions.svelte';
  import ActivityBar from './shell/ActivityBar.svelte';
  import StatusBar from './shell/StatusBar.svelte';
  import TabBar from './shell/TabBar.svelte';
  import Dialog from '../lib/ui/Dialog.svelte';
  import DialogHost from '../lib/ui/DialogHost.svelte';
  import Toaster from '../lib/ui/Toaster.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Input from '../lib/ui/Input.svelte';
  import { toast } from '../lib/ui/toast.svelte';
  import { page as pageDef } from './shell/pages';

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

  $effect(() => {
    const p = pageDef(page);
    document.title = p ? `${p.label} · EngineX` : 'EngineX';
  });

  // Non lues du fil, en direct.
  let unread = $state(0);
  const closeFeed = openEvents('/api/feed/events', { feed: (d) => typeof d.unread === 'number' && (unread = d.unread) });
  onDestroy(closeFeed);

  // Palette (Ctrl+K / Cmd+K), « Plus » du téléphone, mot de passe.
  // Les deux sont chargés au premier usage (bundle principal plus léger).
  let palette = $state(false);
  let more = $state(false);
  let PaletteView = $state<Component<any> | null>(null);
  let MoreView = $state<Component<any> | null>(null);
  async function openPalette() {
    PaletteView ??= (await import('./shell/Palette.svelte')).default;
    palette = true;
  }
  async function openMore() {
    MoreView ??= (await import('./shell/MoreSheet.svelte')).default;
    more = true;
  }
  const onKey = (e: KeyboardEvent) => {
    if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === 'k') {
      e.preventDefault();
      if (palette) palette = false;
      else openPalette();
    }
  };
  window.addEventListener('keydown', onKey);
  onDestroy(() => window.removeEventListener('keydown', onKey));

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

  function openPassword() {
    pwMessage = '';
    showPassword = true;
  }

  async function submitPassword(e: SubmitEvent) {
    e.preventDefault();
    pwMessage = '';
    try {
      await changePassword(current, next);
      current = '';
      next = '';
      showPassword = false;
      toast('Mot de passe changé. Les autres sessions sont fermées.', 'ok');
    } catch (err) {
      pwMessage = message(err);
    }
  }
</script>

<a class="skip" href="#main" onclick={(e) => { e.preventDefault(); document.getElementById('main')?.focus(); }}>Aller au contenu</a>
<div class="shell">
  <div class="act-slot"><ActivityBar {page} onPalette={openPalette} onPassword={openPassword} onLogout={doLogout} /></div>
  <main class="work" id="main" tabindex="-1">
    <div class="view">
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
    </div>
  </main>
  <div class="status-slot"><StatusBar {unread} /></div>
  <div class="tab-slot"><TabBar {page} {unread} onMore={openMore} /></div>
</div>

{#if MoreView}<MoreView bind:open={more} {page} onPalette={openPalette} onPassword={openPassword} onLogout={doLogout} />{/if}
{#if PaletteView}<PaletteView bind:open={palette} />{/if}

<Dialog bind:open={showPassword} title="Changer le mot de passe" width="420px">
  <form id="pw-form" class="pw" onsubmit={submitPassword}>
    <Input label="Mot de passe actuel" type="password" bind:value={current} autocomplete="current-password" required />
    <Input label="Nouveau mot de passe" hint="12 caractères au moins" type="password" bind:value={next} autocomplete="new-password" minlength={12} required />
    {#if pwMessage}<p class="error small" role="alert">{pwMessage}</p>{/if}
  </form>
  {#snippet actions()}
    <Button onclick={() => (showPassword = false)}>Annuler</Button>
    <Button variant="primary" type="submit" form="pw-form">Changer le mot de passe</Button>
  {/snippet}
</Dialog>

<DialogHost />
<Toaster />

<style>
  .shell {
    height: 100dvh;
    display: grid;
    grid-template-columns: var(--act-w) minmax(0, 1fr);
    grid-template-rows: minmax(0, 1fr) var(--status-h);
    grid-template-areas: 'act work' 'status status';
    background: var(--bg);
  }
  .act-slot { grid-area: act; display: flex; min-height: 0; }
  .work { grid-area: work; min-width: 0; min-height: 0; overflow: auto; outline: none; }
  .status-slot { grid-area: status; min-width: 0; }
  .tab-slot { display: none; }
  .view { padding: var(--s6); display: flex; flex-direction: column; gap: var(--s4); min-width: 0; }
  .pw { display: flex; flex-direction: column; gap: var(--s4); }
  .pw p { margin: 0; }
  .skip { position: absolute; left: -9999px; top: 8px; z-index: 100; padding: var(--s2) var(--s3); background: var(--accent-fill); color: var(--accent-ink); border-radius: var(--r); }
  .skip:focus { left: 8px; }

  @media (max-width: 767px) {
    .shell {
      grid-template-columns: minmax(0, 1fr);
      grid-template-rows: auto minmax(0, 1fr) auto;
      grid-template-areas: 'status' 'work' 'tabs';
    }
    .act-slot { display: none; }
    .tab-slot { display: block; grid-area: tabs; }
    .view { padding: var(--s4) calc(var(--s4) + var(--safe-right)) var(--s4) calc(var(--s4) + var(--safe-left)); }
  }
</style>
