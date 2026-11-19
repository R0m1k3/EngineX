<script lang="ts">
  let { diff, truncated = false }: { diff: string; truncated?: boolean } = $props();
  const cls = (l: string) =>
    l.startsWith('diff --git') ? 'file' : l.startsWith('+++') || l.startsWith('---') || l.startsWith('@@') ? 'h' : l[0] === '+' ? 'add' : l[0] === '-' ? 'del' : '';
  let lines = $derived(diff.split('\n'));
</script>

{#if diff.trim()}
  <pre class="diff">{#each lines as l}<span class={cls(l)}>{l}</span>{'\n'}{/each}</pre>
  {#if truncated}<p class="muted small">Diff tronqué (512 Kio).</p>{/if}
{:else}
  <p class="muted small">Pas de diff texte.</p>
{/if}

<style>
  .diff { overflow: auto; background: var(--bg); border: 1px solid var(--line); border-radius: var(--radius); padding: var(--s2); font-size: var(--fs-sm); margin: 0; }
  .add { color: var(--accent); }
  .del { color: var(--error); }
  .h { color: var(--muted); }
  .file { color: var(--text); font-weight: 600; }
</style>
