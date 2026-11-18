<script lang="ts">
  import Engine from './settings/Engine.svelte';
  import Api from './settings/Api.svelte';
  import Vault from './settings/Vault.svelte';
  import Sessions from './settings/Sessions.svelte';

  let { sub }: { sub: string } = $props();
  const tabs = [
    { id: 'engine', label: 'Moteur' },
    { id: 'sessions', label: 'Sessions' },
    { id: 'vault', label: 'Clés' },
    { id: 'api', label: 'API' },
  ];
  let tab = $derived(tabs.some((t) => t.id === sub) ? sub : 'engine');
</script>

<header class="row">
  <h2>Paramètres</h2>
</header>

<div class="tabs" role="tablist">
  {#each tabs as t (t.id)}
    <a role="tab" href="#settings/{t.id}" class:active={tab === t.id} aria-selected={tab === t.id}>{t.label}</a>
  {/each}
</div>

{#if tab === 'api'}<Api />{:else if tab === 'vault'}<Vault />{:else if tab === 'sessions'}<Sessions />{:else}<Engine />{/if}
