<script lang="ts">
  // Encadré bref : avertissement (warn), problème (danger) ou information (info). Icône + texte, action à droite.
  import type { Snippet } from 'svelte';
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
  import CircleAlert from '@lucide/svelte/icons/circle-alert';
  import Info from '@lucide/svelte/icons/info';
  import Icon from '../../lib/ui/Icon.svelte';

  let {
    tone = 'warn',
    children,
    action,
  }: { tone?: 'warn' | 'danger' | 'info'; children: Snippet; action?: Snippet } = $props();
  const glyph = $derived(tone === 'danger' ? CircleAlert : tone === 'warn' ? TriangleAlert : Info);
</script>

<div class="note {tone}" role={tone === 'danger' ? 'alert' : undefined}>
  <Icon icon={glyph} size={16} class="ic" />
  <div class="txt">{@render children()}</div>
  {#if action}<div class="act">{@render action()}</div>{/if}
</div>

<style>
  .note {
    display: flex;
    align-items: flex-start;
    gap: var(--s2);
    padding: var(--s2) var(--s3);
    border: 1px solid var(--line-strong);
    border-radius: var(--r);
    background: var(--panel);
    color: var(--text);
    font-size: var(--fs-sm);
    line-height: 1.5;
  }
  .note :global(.ic) { margin-top: 3px; color: var(--muted); }
  .warn { background: var(--warn-soft); border-color: var(--warn); }
  .warn :global(.ic) { color: var(--warn); }
  .danger { background: var(--danger-soft); border-color: var(--danger); }
  .danger :global(.ic) { color: var(--danger); }
  .txt { flex: 1; min-width: 0; overflow-wrap: anywhere; }
  .txt :global(p) { margin: 0; }
  .act { flex: none; display: flex; gap: var(--s2); align-items: center; }
</style>
