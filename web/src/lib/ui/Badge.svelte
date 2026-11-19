<script lang="ts">
  // Étiquette plate (24 px) : neutre, ok (accent), warn, danger. `dot` ajoute un point d'état.
  import type { Snippet } from 'svelte';
  import StatusDot from './StatusDot.svelte';
  import type { Tone } from './types';

  let {
    tone = 'neutral',
    dot = false,
    title,
    children,
  }: { tone?: Tone; dot?: boolean; title?: string; children: Snippet } = $props();
  const kind = $derived(tone === 'ok' ? 'ok' : tone === 'warn' ? 'warn' : tone === 'danger' ? 'err' : 'off');
</script>

<span class="badge-ui {tone}" {title}>{#if dot}<StatusDot {kind} />{/if}{@render children()}</span>

<style>
  .badge-ui {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 24px;
    padding: 0 var(--s2);
    border-radius: var(--r);
    background: var(--raised);
    color: var(--muted);
    font-size: var(--fs-xs);
    white-space: nowrap;
    vertical-align: middle;
  }
  .ok { background: var(--accent-soft); color: var(--accent); }
  .warn { background: var(--warn-soft); color: var(--warn); }
  .danger { background: var(--danger-soft); color: var(--danger); }
</style>
