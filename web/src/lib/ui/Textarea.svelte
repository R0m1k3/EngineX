<script lang="ts">
  import type { HTMLTextareaAttributes } from 'svelte/elements';
  import Field from './Field.svelte';

  type Props = HTMLTextareaAttributes & {
    value?: string | null;
    label?: string;
    hint?: string;
    error?: string;
    mono?: boolean;
    ref?: HTMLTextAreaElement;
  };
  let {
    value = $bindable(),
    ref = $bindable(),
    label,
    hint,
    error,
    mono = false,
    rows = 4,
    id: idProp,
    class: cls = '',
    ...rest
  }: Props = $props();
  const auto = `ta-${Math.random().toString(36).slice(2, 9)}`;
  const id = $derived(idProp ?? auto);
</script>

<Field {label} {hint} {error} {id}>
  <textarea
    bind:this={ref}
    bind:value
    {id}
    {rows}
    class="ta {cls}"
    class:mono
    class:bad={!!error}
    aria-invalid={error ? 'true' : undefined}
    {...rest}
  ></textarea>
</Field>

<style>
  .ta {
    width: 100%;
    padding: var(--s2) var(--s3);
    background: var(--bg);
    border: 1px solid var(--line-strong);
    border-radius: var(--r);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-md);
    line-height: 1.5;
    resize: vertical;
  }
  .ta:focus-visible { outline: 1px solid var(--accent); border-color: var(--accent); outline-offset: 0; }
  .ta::placeholder { color: var(--faint); }
  .mono { font-family: var(--mono); font-size: var(--fs-code); line-height: 1.55; }
  .bad { border-color: var(--danger); }
</style>
