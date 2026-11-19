<script lang="ts">
  // Champ texte 32 px (44 avec size="lg") : libellé, icône, unité, aide, erreur.
  import type { HTMLInputAttributes } from 'svelte/elements';
  import Field from './Field.svelte';
  import Icon from './Icon.svelte';
  import type { IconType } from './types';

  type Props = Omit<HTMLInputAttributes, 'size'> & {
    value?: string | number | null;
    label?: string;
    hint?: string;
    error?: string;
    icon?: IconType;
    unit?: string;
    size?: 'md' | 'lg';
    ref?: HTMLInputElement;
  };
  let {
    value = $bindable(),
    ref = $bindable(),
    label,
    hint,
    error,
    icon,
    unit,
    size = 'md',
    id: idProp,
    class: cls = '',
    ...rest
  }: Props = $props();
  const auto = `in-${Math.random().toString(36).slice(2, 9)}`;
  const id = $derived(idProp ?? auto);
</script>

<Field {label} {hint} {error} {id}>
  <span class="box {size} {cls}" class:bad={!!error}>
    {#if icon}<Icon {icon} size={16} class="lead" />{/if}
    <input
      bind:this={ref}
      bind:value
      {id}
      aria-invalid={error ? 'true' : undefined}
      aria-describedby={error ? `${id}-err` : hint ? `${id}-hint` : undefined}
      {...rest}
    />
    {#if unit}<span class="unit">{unit}</span>{/if}
  </span>
</Field>

<style>
  .box {
    display: flex;
    align-items: center;
    gap: var(--s2);
    height: 32px;
    padding: 0 var(--s3);
    background: var(--bg);
    border: 1px solid var(--line-strong);
    border-radius: var(--r);
    color: var(--faint);
  }
  .box:focus-within { border-color: var(--accent); outline: 1px solid var(--accent); }
  .box.bad { border-color: var(--danger); }
  .lg { height: var(--touch); font-size: var(--fs-md); }
  input {
    flex: 1;
    min-width: 0;
    height: 100%;
    min-height: 0;
    padding: 0;
    border: 0;
    background: none;
    outline: none;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-sm);
  }
  .lg input { font-size: var(--fs-md); }
  input:focus-visible { outline: none; }
  .unit { color: var(--faint); white-space: nowrap; font-size: var(--fs-sm); }
</style>
