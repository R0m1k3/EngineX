<script lang="ts">
  // État vide : repère en tirets avec l'icône, une phrase qui dit ce qui arrivera ici, une action au plus.
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';
  import type { IconType } from './types';

  let {
    icon,
    title,
    text,
    action,
  }: { icon?: IconType; title: string; text?: string; action?: Snippet } = $props();
</script>

<div class="empty">
  {#if icon}<div class="mark"><Icon {icon} /></div>{/if}
  <div class="lead">{title}</div>
  {#if text}<div class="txt">{text}</div>{/if}
  {#if action}<div class="act">{@render action()}</div>{/if}
</div>

<style>
  .empty { display: flex; flex-direction: column; align-items: flex-start; gap: var(--s2); padding: var(--s4) 0; color: var(--muted); font-size: var(--fs-sm); max-width: 44ch; }
  .mark { width: 40px; height: 40px; border: 1px dashed var(--line-strong); border-radius: var(--r); display: grid; place-items: center; color: var(--faint); margin-bottom: var(--s1); }
  .lead { color: var(--text); font-size: var(--fs-md); }
  .act { margin-top: var(--s2); }
</style>
