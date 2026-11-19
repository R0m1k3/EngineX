<script lang="ts">
  import type { Verification } from '../../lib/api';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import CircleCheck from '@lucide/svelte/icons/circle-check';
  import CircleX from '@lucide/svelte/icons/circle-x';
  import LoaderCircle from '@lucide/svelte/icons/loader-circle';
  import CircleDot from '@lucide/svelte/icons/circle-dot';
  import Check from '@lucide/svelte/icons/check';
  import X from '@lucide/svelte/icons/x';
  import RotateCw from '@lucide/svelte/icons/rotate-cw';
  import Undo2 from '@lucide/svelte/icons/undo-2';
  import Icon from '../../lib/ui/Icon.svelte';
  import Badge from '../../lib/ui/Badge.svelte';
  import Button from '../../lib/ui/Button.svelte';

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
  const secs = (ms?: number) => (ms == null ? '' : `${(ms / 1000).toLocaleString('fr-FR', { maximumFractionDigits: 1 })} s`);
  const ICON: Record<string, any> = { running: LoaderCircle, passed: CircleCheck, failed: CircleX, error: CircleX };
</script>

{#if last}
  <details class="verify v-{last.status}">
    <summary>
      <Icon icon={ChevronRight} size={16} class="chev" />
      <Icon icon={ICON[last.status] ?? CircleDot} size={16} class="st" />
      <span>{LABEL[last.status] ?? last.status}</span>
      {#if checks.length > 1}<span class="faint num">essai {last.attempt}</span>{/if}
      {#if green}<Badge tone="ok">dernier état vert</Badge>{/if}
    </summary>
    <div class="body">
      {#each last.steps ?? [] as s, i (i)}
        {#if s.error}<p class="error step">{s.error}</p>
        {:else}
          <div class="step">
            <span class={s.exit_code === 0 ? 'good' : 'bad'}><Icon icon={s.exit_code === 0 ? Check : X} size={16} label={s.exit_code === 0 ? 'Réussi' : 'Échec'} /></span>
            <span>{s.name}</span>
            <code class="faint ell">{s.run}</code>
            <span class="faint num meta">{s.timed_out ? 'délai dépassé' : `code ${s.exit_code ?? '?'}`} {secs(s.duration_ms)}</span>
          </div>
          {#if s.output && s.exit_code !== 0}<pre class="out">{s.output}</pre>{/if}
        {/if}
      {/each}
      {#if (rerun && last.status !== 'running') || (toGreen && last.status !== 'passed')}
        <div class="acts">
          {#if rerun && last.status !== 'running'}<Button size="sm" icon={RotateCw} onclick={rerun}>Relancer</Button>{/if}
          {#if toGreen && last.status !== 'passed'}<Button size="sm" variant="ghost" icon={Undo2} onclick={toGreen}>Revenir au dernier état vert</Button>{/if}
        </div>
      {/if}
    </div>
  </details>
{/if}

<style>
  .verify { border-left: 2px solid var(--line-strong); padding-left: var(--s3); font-size: var(--fs-sm); }
  .verify.v-passed { border-left-color: var(--accent); }
  .verify.v-failed, .verify.v-error { border-left-color: var(--danger); }
  .verify.v-running { border-left-color: var(--warn); }
  summary { list-style: none; display: flex; gap: var(--s2); align-items: center; min-height: 30px; cursor: pointer; color: var(--text); flex-wrap: wrap; }
  summary::-webkit-details-marker { display: none; }
  summary :global(.chev) { color: var(--faint); transition: transform 0.12s; flex: none; }
  .verify[open] > summary :global(.chev) { transform: rotate(90deg); }
  .v-passed :global(.st) { color: var(--accent); }
  .v-failed :global(.st), .v-error :global(.st) { color: var(--danger); }
  .v-running :global(.st) { color: var(--warn); animation: spin 1s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .body { display: flex; flex-direction: column; gap: var(--s1); padding: var(--s1) 0 var(--s2) var(--s6); }
  .step { display: flex; gap: var(--s2); align-items: center; margin: 0; min-height: 28px; min-width: 0; }
  .step code { flex: 1; min-width: 0; }
  .meta { white-space: nowrap; }
  .good { color: var(--accent); display: inline-flex; }
  .bad { color: var(--danger); display: inline-flex; }
  .out { margin: 0; max-height: 240px; overflow: auto; background: var(--bg); border: 1px solid var(--line); border-radius: var(--r); padding: var(--s2) var(--s3); white-space: pre-wrap; color: var(--muted); }
  .acts { display: flex; gap: var(--s2); flex-wrap: wrap; margin-top: var(--s2); }
  @media (prefers-reduced-motion: reduce) { .v-running :global(.st) { animation: none; } }
</style>
