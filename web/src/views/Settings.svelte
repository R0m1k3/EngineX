<script lang="ts">
  import Engine from './settings/Engine.svelte';

  let { sub }: { sub: string } = $props();
</script>

<header class="row">
  <h2>Paramètres</h2>
</header>

<div class="tabs" role="tablist">
  <a role="tab" href="#settings/engine" class:active={sub === '' || sub === 'engine'} aria-selected="true">Moteur</a>
</div>

<Engine />
