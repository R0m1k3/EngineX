<script lang="ts">
  import Layers from '@lucide/svelte/icons/layers';
  import HardDrive from '@lucide/svelte/icons/hard-drive';
  import Search from '@lucide/svelte/icons/search';
  import Download from '@lucide/svelte/icons/download';
  import Icon from '../lib/ui/Icon.svelte';
  import EngineBar from './models/EngineBar.svelte';
  import SearchView from './models/Search.svelte';
  import Downloads from './models/Downloads.svelte';
  import Local from './models/Local.svelte';
  import Presets from './models/Presets.svelte';

  let { sub }: { sub: string } = $props();
  // Mêmes routes qu'avant (#models = recherche) ; ordre d'affichage de la maquette.
  const tabs = [
    { id: 'presets', label: 'Presets', icon: Layers },
    { id: 'local', label: 'Locaux', icon: HardDrive },
    { id: '', label: 'Recherche HF', icon: Search },
    { id: 'downloads', label: 'Téléchargements', icon: Download },
  ];
</script>

<div class="models">
  <h1 class="title">Modèles</h1>
  <nav class="mtabs" aria-label="Modèles">
    {#each tabs as t (t.id)}
      <a href={t.id ? `#models/${t.id}` : '#models'} class:on={sub === t.id} aria-current={sub === t.id ? 'page' : undefined}>
        <Icon icon={t.icon} size={16} /><span>{t.label}</span>
      </a>
    {/each}
  </nav>

  <EngineBar />

  <div class="body" class:split={sub === 'presets'}>
    {#if sub === 'downloads'}
      <Downloads />
    {:else if sub === 'local'}
      <Local />
    {:else if sub === 'presets'}
      <Presets />
    {:else}
      <SearchView />
    {/if}
  </div>
</div>

<style>
  /* Bord à bord : la page annule la marge de la vue et pose ses propres filets. */
  .models {
    margin: calc(-1 * var(--s6));
    min-height: calc(100dvh - var(--status-h));
    display: flex;
    flex-direction: column;
    background: var(--bg);
  }
  .title { display: none; }
  .mtabs {
    display: flex;
    gap: var(--s1);
    padding: 0 var(--s4);
    background: var(--panel);
    border-bottom: 1px solid var(--line);
    overflow-x: auto;
    scrollbar-width: none;
    flex: none;
  }
  .mtabs a {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 40px;
    padding: 0 var(--s3);
    margin-bottom: -1px;
    border-bottom: 2px solid transparent;
    color: var(--muted);
    font-size: var(--fs-sm);
    text-decoration: none;
    white-space: nowrap;
  }
  .mtabs a:hover { color: var(--text); }
  .mtabs a.on { color: var(--text); border-bottom-color: var(--accent); }
  .body { flex: 1; min-height: 0; display: flex; flex-direction: column; }

  @media (max-width: 767px) {
    .models { margin: calc(-1 * var(--s4)) calc(-1 * (var(--s4) + var(--safe-right))) calc(-1 * var(--s4)) calc(-1 * (var(--s4) + var(--safe-left))); min-height: 0; }
    .title { display: block; margin: 0; padding: var(--s5) var(--s4) var(--s3); font-size: var(--fs-xl); font-weight: 400; }
    /* Téléphone : filtres en pilules (seul endroit où la direction les admet). */
    .mtabs { gap: var(--s2); padding: var(--s3) var(--s4); background: var(--bg); border-top: 1px solid var(--line); }
    .mtabs a {
      height: var(--touch);
      margin: 0;
      padding: 0 var(--s4);
      border: 1px solid var(--line-strong);
      border-radius: 22px;
      font-size: var(--fs-md);
    }
    .mtabs a :global(.ui-icon) { display: none; }
    .mtabs a.on { background: var(--accent-soft); border-color: var(--accent); color: var(--accent); }
  }
</style>
