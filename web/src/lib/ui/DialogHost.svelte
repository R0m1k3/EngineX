<script lang="ts">
  // Modale des confirmDialog() / promptDialog() en attente (une à la fois). À monter une fois dans la coquille.
  import Dialog from './Dialog.svelte';
  import Button from './Button.svelte';
  import Input from './Input.svelte';
  import { dialogs, settle } from './dialog.svelte';

  let open = $state(false);
  let value = $state('');
  let ref = $state<HTMLInputElement>();
  const formId = `prompt-${Math.random().toString(36).slice(2, 9)}`;

  $effect(() => {
    const d = dialogs.current;
    open = !!d;
    if (d?.kind !== 'prompt') return;
    value = d.value ?? '';
    const stem = d.selectStem;
    setTimeout(() => {
      if (!ref) return;
      ref.focus();
      const dot = stem ? value.lastIndexOf('.') : -1;
      ref.setSelectionRange(0, dot > 0 ? dot : value.length);
    }, 30);
  });

  function submit(e: SubmitEvent) {
    e.preventDefault();
    const d = dialogs.current;
    if (d?.kind === 'prompt' && (d.allowEmpty || value.trim())) settle(value);
  }
</script>

{#if dialogs.current}
  {@const d = dialogs.current}
  <Dialog bind:open title={d.title} width={d.kind === 'prompt' ? '440px' : undefined} onclose={() => settle(d.kind === 'prompt' ? null : false)}>
    {#if d.kind === 'prompt'}
      <form id={formId} onsubmit={submit}>
        <Input
          label={d.message}
          hint={d.hint}
          placeholder={d.placeholder}
          maxlength={d.maxlength}
          class={d.mono ? 'mono-in' : ''}
          bind:value
          bind:ref
          autocomplete="off"
          spellcheck="false"
        />
      </form>
    {:else}
      <p class="msg">{d.message}</p>
    {/if}
    {#snippet actions()}
      {#if d.kind === 'prompt'}
        <Button onclick={() => settle(null)}>{d.cancel}</Button>
        <Button variant="primary" type="submit" form={formId} disabled={!d.allowEmpty && !value.trim()}>{d.confirm}</Button>
      {:else}
        <Button onclick={() => settle(false)}>{d.cancel}</Button>
        <Button variant={d.danger ? 'danger' : 'primary'} onclick={() => settle(true)}>{d.confirm}</Button>
      {/if}
    {/snippet}
  </Dialog>
{/if}

<style>
  .msg { margin: 0; white-space: pre-line; }
  form :global(.mono-in input) { font-family: var(--mono); }
</style>
