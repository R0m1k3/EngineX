<script lang="ts">
  import EngineBar from './models/EngineBar.svelte';
  import Search from './models/Search.svelte';
  import Downloads from './models/Downloads.svelte';
  import Local from './models/Local.svelte';
  import Presets from './models/Presets.svelte';

  let { sub }: { sub: string } = $props();
  const tabs = [
    { id: '', label: 'Chercher' },
    { id: 'downloads', label: 'Téléchargements' },
    { id: 'local', label: 'Mes modèles' },
    { id: 'presets', label: 'Presets' },
  ];
</script>

<header class="row">
  <h2>Modèles</h2>
</header>

<EngineBar />

<div class="tabs" role="tablist">
  {#each tabs as t}
    <a role="tab" href={t.id ? `#models/${t.id}` : '#models'} class:active={sub === t.id} aria-selected={sub === t.id}>{t.label}</a>
  {/each}
</div>

{#if sub === 'downloads'}
  <Downloads />
{:else if sub === 'local'}
  <Local />
{:else if sub === 'presets'}
  <Presets />
{:else}
  <Search />
{/if}
