<script lang="ts" generics="T extends string | number">
  // Liste déroulante native restylée. `options` ou des <option> en enfants.
  import type { Snippet } from 'svelte';
  import type { HTMLSelectAttributes } from 'svelte/elements';
  import Field from './Field.svelte';

  type Props = Omit<HTMLSelectAttributes, 'value' | 'size'> & {
    value?: T;
    options?: { value: T; label: string; disabled?: boolean }[];
    label?: string;
    hint?: string;
    error?: string;
    size?: 'md' | 'lg';
    children?: Snippet;
  };
  let {
    value = $bindable(),
    options,
    label,
    hint,
    error,
    size = 'md',
    id: idProp,
    class: cls = '',
    children,
    ...rest
  }: Props = $props();
  const auto = `se-${Math.random().toString(36).slice(2, 9)}`;
  const id = $derived(idProp ?? auto);
</script>

<Field {label} {hint} {error} {id}>
  <select bind:value {id} class="sel {size} {cls}" class:bad={!!error} {...rest}>
    {#if options}
      {#each options as o (o.value)}<option value={o.value} disabled={o.disabled}>{o.label}</option>{/each}
    {:else if children}
      {@render children()}
    {/if}
  </select>
</Field>

<style>
  .sel {
    height: 32px;
    min-height: 0;
    padding: 0 28px 0 10px;
    border: 1px solid var(--line-strong);
    border-radius: var(--r);
    background-color: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-sm);
    appearance: none;
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 24 24' fill='none' stroke='%23889099' stroke-width='2.5'%3E%3Cpath d='m6 9 6 6 6-6'/%3E%3C/svg%3E");
    background-repeat: no-repeat;
    background-position: right 9px center;
    cursor: pointer;
  }
  .sel:focus-visible { outline: 1px solid var(--accent); border-color: var(--accent); outline-offset: 0; }
  .lg { height: var(--touch); font-size: var(--fs-md); }
  .bad { border-color: var(--danger); }
</style>
