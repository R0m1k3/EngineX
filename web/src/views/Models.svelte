<script lang="ts">
  import Layers from '@lucide/svelte/icons/layers';
  import HardDrive from '@lucide/svelte/icons/hard-drive';
  import Search from '@lucide/svelte/icons/search';
  import Download from '@lucide/svelte/icons/download';
  import Page from '../lib/ui/Page.svelte';
  import Tabs from '../lib/ui/Tabs.svelte';
  import type { TabItem } from '../lib/ui/types';
  import EngineBar from './models/EngineBar.svelte';
  import SearchView from './models/Search.svelte';
  import Downloads from './models/Downloads.svelte';
  import Local from './models/Local.svelte';
  import Presets from './models/Presets.svelte';

  let { sub }: { sub: string } = $props();
  // Mêmes routes qu'avant (#models = recherche) ; ordre d'affichage de la maquette.
  const items: TabItem[] = [
    { id: 'presets', label: 'Presets', icon: Layers, href: '#models/presets' },
    { id: 'local', label: 'Locaux', icon: HardDrive, href: '#models/local' },
    { id: '', label: 'Recherche HF', icon: Search, href: '#models' },
    { id: 'downloads', label: 'Téléchargements', icon: Download, href: '#models/downloads' },
  ];
</script>

<Page title="Modèles">
  {#snippet tabs()}<Tabs {items} active={sub} label="Modèles" />{/snippet}
  <EngineBar />
  <div class="body">
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
</Page>

<style>
  .body { flex: 1; min-height: 0; display: flex; flex-direction: column; }
</style>
