<script lang="ts">
  import Engine from './settings/Engine.svelte';
  import Api from './settings/Api.svelte';

  let { sub }: { sub: string } = $props();
  let tab = $derived(sub === 'api' ? 'api' : 'engine');
</script>

<header class="row">
  <h2>Paramètres</h2>
</header>

<div class="tabs" role="tablist">
  <a role="tab" href="#settings/engine" class:active={tab === 'engine'} aria-selected={tab === 'engine'}>Moteur</a>
  <a role="tab" href="#settings/api" class:active={tab === 'api'} aria-selected={tab === 'api'}>API</a>
</div>

{#if tab === 'api'}<Api />{:else}<Engine />{/if}
