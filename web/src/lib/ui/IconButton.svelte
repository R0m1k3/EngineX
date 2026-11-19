<script lang="ts">
  // Bouton icône seule, sans fond au repos (barres d'outils, barre d'état).
  // `label` = aria-label + infobulle ; `count` = pastille (non lues…) ; `pressed` = état bascule.
  import type { HTMLButtonAttributes } from 'svelte/elements';
  import Icon from './Icon.svelte';
  import { tooltip, type TooltipSide } from './tooltip';
  import type { IconType } from './types';

  type Props = Omit<HTMLButtonAttributes, 'children'> & {
    icon: IconType;
    label: string;
    size?: 'sm' | 'md' | 'lg';
    iconSize?: number;
    count?: number;
    pressed?: boolean;
    href?: string;
    tip?: TooltipSide;
    kbd?: string;
  };
  let {
    icon,
    label,
    size = 'md',
    iconSize,
    count = 0,
    pressed,
    href,
    tip = 'top',
    kbd,
    type = 'button',
    class: cls = '',
    ...rest
  }: Props = $props();
  const isz = $derived(iconSize ?? (size === 'lg' ? 22 : size === 'sm' ? 16 : 18));
  const shown = $derived(count > 99 ? '99+' : String(count));
</script>

{#snippet inner()}
  <Icon {icon} size={isz} />
  {#if count > 0}<span class="count" aria-hidden="true">{shown}</span>{/if}
{/snippet}

{#if href}
  <a {href} class="ib {size} {cls}" aria-label={count ? `${label} (${count})` : label} use:tooltip={{ text: label, side: tip, kbd }}>{@render inner()}</a>
{:else}
  <button
    {type}
    class="ib {size} {cls}"
    aria-label={count ? `${label} (${count})` : label}
    aria-pressed={pressed}
    use:tooltip={{ text: label, side: tip, kbd }}
    {...rest}
  >{@render inner()}</button>
{/if}

<style>
  .ib {
    position: relative;
    display: inline-grid;
    place-items: center;
    width: 32px;
    height: 32px;
    min-height: 0;
    padding: 0;
    border: 0;
    border-radius: var(--r);
    background: transparent;
    color: var(--muted);
    cursor: pointer;
    text-decoration: none;
    filter: none;
  }
  .ib:hover:not(:disabled) { background: var(--raised); color: var(--text); }
  .ib:disabled { opacity: 0.5; cursor: default; }
  .ib[aria-pressed='true'] { color: var(--accent); background: var(--accent-soft); }
  .sm { width: 28px; height: 28px; }
  .lg { width: var(--touch); height: var(--touch); }
  .count {
    position: absolute;
    top: 0;
    right: -2px;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    border-radius: 8px;
    background: var(--accent-fill);
    color: var(--accent-ink);
    font-size: 11px;
    font-weight: 600;
    line-height: 16px;
    text-align: center;
    font-variant-numeric: tabular-nums;
  }
</style>
