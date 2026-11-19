<script lang="ts">
  import type { Verification } from '../../lib/api';

  let { checks, green, rerun, toGreen }: {
    checks: Verification[];
    /** Ce tour est le dernier état vert. */
    green: boolean;
    rerun: (() => void) | null;
    toGreen: (() => void) | null;
  } = $props();

  let last = $derived(checks[checks.length - 1]);
  const LABEL: Record<string, string> = {
    running: 'Vérification en cours…',
    passed: 'Vérification réussie',
    failed: 'Vérification en échec',
    error: 'Vérification impossible',
    cancelled: 'Vérification annulée',
  };
  const secs = (ms?: number) => (ms == null ? '' : `${(ms / 1000).toFixed(1)} s`);
</script>

{#if last}
  <details class="verify v-{last.status}">
    <summary class="small">
      <span class="dot"></span>{LABEL[last.status] ?? last.status}
      {#if checks.length > 1}<span class="muted">(essai {last.attempt})</span>{/if}
      {#if green}<span class="badge">dernier état vert</span>{/if}
    </summary>
    {#each last.steps ?? [] as s, i (i)}
      {#if s.error}<p class="error small">{s.error}</p>
      {:else}
        <p class="small step">
          <span class:ok={s.exit_code === 0} class:error={s.exit_code !== 0}>{s.exit_code === 0 ? '✓' : '✗'}</span>
          {s.name} <code class="muted">{s.run}</code>
          <span class="muted">{s.timed_out ? 'délai dépassé' : `code ${s.exit_code ?? '?'}`} {secs(s.duration_ms)}</span>
        </p>
        {#if s.output && s.exit_code !== 0}<pre class="out">{s.output}</pre>{/if}
      {/if}
    {/each}
    <div class="actions small">
      {#if rerun && last.status !== 'running'}<button class="link" onclick={rerun}>Relancer</button>{/if}
      {#if toGreen && last.status !== 'passed'}<button class="link" onclick={toGreen}>Revenir au dernier état vert</button>{/if}
    </div>
  </details>
{/if}

<style>
  .verify { border-left: 2px solid var(--line); padding-left: var(--s3); }
  .verify.v-passed { border-left-color: var(--accent); }
  .verify.v-failed, .verify.v-error { border-left-color: var(--error); }
  summary { cursor: pointer; display: flex; gap: var(--s2); align-items: center; }
  .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--muted); display: inline-block; }
  .v-passed .dot { background: var(--accent); }
  .v-failed .dot, .v-error .dot { background: var(--error); }
  .v-running .dot { background: var(--warn); }
  .step { margin: var(--s1) 0 0; display: flex; gap: var(--s2); align-items: baseline; flex-wrap: wrap; }
  .out {
    max-height: 240px; overflow: auto; background: var(--bg); border: 1px solid var(--line);
    border-radius: var(--radius); padding: var(--s2); font-size: var(--fs-sm); white-space: pre-wrap; margin: var(--s1) 0;
  }
  .badge { margin-left: 0; }
</style>
