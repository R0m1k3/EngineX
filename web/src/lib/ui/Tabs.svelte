<script lang="ts">
  // Onglets plats soulignés d'accent. Avec `href` : liens (routage #…), sinon boutons (bind:active).
  // `compact` : onglets inactifs réduits à l'icône (nom en infobulle), comme le panneau de session.
  import Icon from './Icon.svelte';
  import { tooltip } from './tooltip';
  import type { TabItem } from './types';

  let {
    items,
    active = $bindable(),
    label,
    compact = false,
    onchange,
  }: { items: TabItem[]; active: string; label: string; compact?: boolean; onchange?: (id: string) => void } = $props();

  function pick(id: string) {
    active = id;
    onchange?.(id);
  }
</script>

<div class="tabs-ui" role="tablist" aria-label={label}>
  {#each items as t (t.id)}
    {@const on = t.id === active}
    {@const iconOnly = compact && !on && !!t.icon}
    {#if t.href}
      <a role="tab" href={t.href} class:on aria-selected={on} aria-label={iconOnly ? t.label : undefined} use:tooltip={iconOnly ? t.label : null} onclick={() => pick(t.id)}>
        {#if t.icon}<Icon icon={t.icon} size={16} />{/if}{#if !iconOnly}<span>{t.label}</span>{/if}{#if t.count}<span class="n">{t.count}</span>{/if}
      </a>
    {:else}
      <button type="button" role="tab" class:on aria-selected={on} aria-label={iconOnly ? t.label : undefined} use:tooltip={iconOnly ? t.label : null} onclick={() => pick(t.id)}>
        {#if t.icon}<Icon icon={t.icon} size={16} />{/if}{#if !iconOnly}<span>{t.label}</span>{/if}{#if t.count}<span class="n">{t.count}</span>{/if}
      </button>
    {/if}
  {/each}
</div>

<style>
  .tabs-ui { display: flex; border-bottom: 1px solid var(--line); overflow-x: auto; scrollbar-width: none; flex: none; }
  a, button {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 40px;
    min-height: 0;
    padding: 0 var(--s3);
    margin-bottom: -1px;
    border: 0;
    border-bottom: 2px solid transparent;
    border-radius: 0;
    background: none;
    color: var(--muted);
    font: inherit;
    font-size: var(--fs-sm);
    font-weight: 400;
    white-space: nowrap;
    text-decoration: none;
    cursor: pointer;
    filter: none;
  }
  a:hover, button:hover { color: var(--text); }
  .on { color: var(--text); border-bottom-color: var(--accent); }
  .n { font-size: 12px; color: var(--muted); background: var(--raised); border-radius: 8px; padding: 0 6px; line-height: 18px; font-variant-numeric: tabular-nums; }
</style>
