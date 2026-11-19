<script lang="ts">
  // Modale des confirmDialog() en attente (une à la fois). À monter une fois dans la coquille.
  import Dialog from './Dialog.svelte';
  import Button from './Button.svelte';
  import { dialogs, settle } from './dialog.svelte';

  let open = $state(false);
  $effect(() => {
    open = !!dialogs.current;
  });
</script>

{#if dialogs.current}
  {@const d = dialogs.current}
  <Dialog bind:open title={d.title} onclose={() => settle(false)}>
    <p class="msg">{d.message}</p>
    {#snippet actions()}
      <Button onclick={() => settle(false)}>{d.cancel}</Button>
      <Button variant={d.danger ? 'danger' : 'primary'} onclick={() => settle(true)}>{d.confirm}</Button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .msg { margin: 0; white-space: pre-line; }
</style>
