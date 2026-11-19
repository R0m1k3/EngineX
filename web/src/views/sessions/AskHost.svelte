<script lang="ts">
  import Dialog from '../../lib/ui/Dialog.svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Input from '../../lib/ui/Input.svelte';
  import { asking, answer } from './ask.svelte';

  let open = $state(false);
  let value = $state('');
  let ref = $state<HTMLInputElement>();
  const formId = `ask-${Math.random().toString(36).slice(2, 8)}`;

  $effect(() => {
    const p = asking.current;
    if (p) {
      value = p.value ?? '';
      open = true;
      queueMicrotask(() => ref?.select());
    } else open = false;
  });

  function submit(e: SubmitEvent) {
    e.preventDefault();
    if (value.trim()) answer(value);
  }
</script>

<Dialog bind:open title={asking.current?.title ?? ''} width="440px" onclose={() => answer(null)}>
  <form id={formId} onsubmit={submit}>
    <Input label={asking.current?.label} bind:value bind:ref autocomplete="off" />
  </form>
  {#snippet actions()}
    <Button onclick={() => answer(null)}>Annuler</Button>
    <Button variant="primary" type="submit" form={formId} disabled={!value.trim()}>{asking.current?.confirm ?? 'Valider'}</Button>
  {/snippet}
</Dialog>
