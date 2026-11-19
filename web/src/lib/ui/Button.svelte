<script lang="ts">
  // Bouton : primary (un seul par zone), secondary, ghost (discret), danger.
  // Tailles sm 28 / md 32 / lg 44 (cible tactile). `href` en fait un lien.
  // Sans texte (`children` absent), `label` est obligatoire : aria-label + infobulle.
  import type { Snippet } from 'svelte';
  import type { HTMLButtonAttributes } from 'svelte/elements';
  import Icon from './Icon.svelte';
  import { tooltip } from './tooltip';
  import type { ButtonSize, ButtonVariant, IconType } from './types';

  type Props = Omit<HTMLButtonAttributes, 'children'> & {
    variant?: ButtonVariant;
    size?: ButtonSize;
    icon?: IconType;
    /** Icône après le texte (menu, lien externe). */
    iconEnd?: IconType;
    label?: string;
    href?: string;
    full?: boolean;
    busy?: boolean;
    children?: Snippet;
  };
  let {
    variant = 'secondary',
    size = 'md',
    icon,
    iconEnd,
    label,
    href,
    full = false,
    busy = false,
    type = 'button',
    disabled,
    class: cls = '',
    children,
    ...rest
  }: Props = $props();
  const only = $derived(!children);
  const isz = $derived(size === 'lg' ? 20 : 16);
</script>

{#snippet inner()}
  {#if icon}<Icon icon={icon} size={isz} />{/if}
  {#if children}<span class="lbl">{@render children()}</span>{/if}
  {#if iconEnd}<Icon icon={iconEnd} size={isz} />{/if}
{/snippet}

{#if href && !disabled}
  <a
    {href}
    class="btn {variant} {size} {cls}"
    class:only
    class:full
    aria-label={only ? label : undefined}
    use:tooltip={only ? label : null}
  >{@render inner()}</a>
{:else}
  <button
    {type}
    class="btn {variant} {size} {cls}"
    class:only
    class:full
    disabled={disabled || busy}
    aria-busy={busy || undefined}
    aria-label={only ? label : undefined}
    use:tooltip={only ? label : null}
    {...rest}
  >{@render inner()}</button>
{/if}

<style>
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--s2);
    height: 32px;
    min-height: 0;
    padding: 0 var(--s3);
    border: 1px solid var(--line-strong);
    border-radius: var(--r);
    background: var(--raised);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-sm);
    font-weight: 400;
    line-height: 1;
    white-space: nowrap;
    text-decoration: none;
    cursor: pointer;
    filter: none;
  }
  .btn:hover:not(:disabled) { border-color: var(--faint); }
  .btn:disabled { opacity: 0.5; cursor: default; }
  .primary { background: var(--accent-fill); border-color: var(--accent-fill); color: var(--accent-ink); font-weight: 600; }
  .primary:hover:not(:disabled) { border-color: var(--accent-fill); filter: brightness(1.06); }
  .ghost { background: transparent; border-color: transparent; color: var(--muted); }
  .ghost:hover:not(:disabled) { background: var(--raised); border-color: transparent; color: var(--text); }
  .danger { color: var(--danger); }
  .danger:hover:not(:disabled) { border-color: var(--danger); background: var(--danger-soft); }
  .sm { height: 28px; padding: 0 var(--s2); }
  .lg { height: var(--touch); padding: 0 var(--s4); font-size: var(--fs-md); }
  .only { padding: 0; width: 32px; }
  .only.sm { width: 28px; }
  .only.lg { width: var(--touch); }
  .full { width: 100%; }
  .lbl { overflow: hidden; text-overflow: ellipsis; }
  .btn[aria-busy='true'] { cursor: progress; }
</style>
