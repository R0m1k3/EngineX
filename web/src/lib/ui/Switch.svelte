<script lang="ts">
  // Interrupteur (role="switch") avec son libellé à droite.
  import type { Snippet } from 'svelte';
  let {
    checked = $bindable(false),
    disabled = false,
    label,
    onchange,
    children,
  }: {
    checked?: boolean;
    disabled?: boolean;
    label?: string;
    onchange?: (v: boolean) => void;
    children?: Snippet;
  } = $props();
  function toggle() {
    if (disabled) return;
    checked = !checked;
    onchange?.(checked);
  }
</script>

<button type="button" class="sw" role="switch" aria-checked={checked} {disabled} onclick={toggle}>
  <i aria-hidden="true"></i>
  {#if children}<span>{@render children()}</span>{:else if label}<span>{label}</span>{/if}
</button>

<style>
  .sw {
    display: inline-flex;
    align-items: center;
    gap: var(--s2);
    min-height: 32px;
    padding: 0;
    text-align: left;
    white-space: normal;
    border: 0;
    background: none;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-sm);
    font-weight: 400;
    cursor: pointer;
    filter: none;
  }
  .sw:disabled { opacity: 0.5; cursor: default; }
  i { position: relative; flex: none; width: 34px; height: 20px; border-radius: 10px; background: var(--line-strong); transition: background 0.12s; }
  i::after { content: ''; position: absolute; top: 3px; left: 3px; width: 14px; height: 14px; border-radius: 50%; background: var(--bg); transition: left 0.12s; }
  .sw[aria-checked='true'] i { background: var(--accent-fill); }
  .sw[aria-checked='true'] i::after { left: 17px; background: var(--accent-ink); }
</style>
