<script lang="ts">
  // Comparaison des presets : mêmes chiffres que le résumé d'un run, cumulés.
  import type { BenchCaseDef, BenchCompare } from '../../lib/api';
  import { EmptyState } from '../../lib/ui';
  import FlaskConical from '@lucide/svelte/icons/flask-conical';
  import { FAMILY, dur, pct, visual } from './shared';

  let { rows, defs }: { rows: BenchCompare[]; defs: BenchCaseDef[] } = $props();
  const avgDur = (p: BenchCompare) => (p.summary.audited ? Math.round(p.summary.duration_s / p.summary.audited) : 0);
</script>

{#if rows.length === 0}
  <div class="pad"><EmptyState icon={FlaskConical} title="Rien à comparer" text="La comparaison se remplit à chaque test contrôlé." /></div>
{:else}
  <p class="faint pad">Tous les tests contrôlés de chaque preset, tous runs confondus. Résultat = points gagnés sur points possibles ; visuel = moyenne de vos notes.</p>
  <div class="scroll">
    <table class="grid">
      <thead>
        <tr><th>Preset</th><th class="num">Runs</th><th class="num">Réussis</th><th class="num">Résultat</th><th class="num">Visuel</th><th class="num">Durée moy.</th><th class="num">t/s</th></tr>
      </thead>
      <tbody>
        {#each rows as p (p.preset_id)}
          <tr>
            <td>{p.preset}</td>
            <td class="num">{p.runs}</td>
            <td class="num">{p.summary.passed} / {p.summary.audited}</td>
            <td class="num"><strong>{pct(p.summary.result)}</strong></td>
            <td class="num">{visual(p.summary.visual)}</td>
            <td class="num">{dur(avgDur(p))}</td>
            <td class="num">{p.summary.tps ? p.summary.tps.toFixed(1) : '—'}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>

  <h3 class="pad">Résultat par test</h3>
  <div class="scroll">
    <table class="grid">
      <thead>
        <tr><th>Test</th>{#each rows as p (p.preset_id)}<th class="num">{p.preset}</th>{/each}</tr>
      </thead>
      <tbody>
        {#each (['visual', 'code', 'tools'] as const) as fam (fam)}
          <tr class="fam"><td colspan={rows.length + 1}>{FAMILY[fam].label}</td></tr>
          {#each defs.filter((d) => d.family === fam) as d (d.key)}
            <tr>
              <td>{d.title}</td>
              {#each rows as p (p.preset_id)}
                {@const x = p.cases.find((k) => k.key === d.key)}
                <td class="num" title={x ? `${x.passed} réussi(s) sur ${x.samples}` : ''}>
                  {x ? pct(x.result) : '—'}{#if x && fam === 'visual' && x.visual != null}<span class="faint">&nbsp;· {visual(x.visual)}</span>{/if}
                </td>
              {/each}
            </tr>
          {/each}
        {/each}
      </tbody>
    </table>
  </div>
{/if}

<style>
  .pad { padding: var(--sp-4, 16px); margin: 0; }
  h3.pad { padding-bottom: 0; font-size: var(--fs-md, 1rem); }
  .scroll { overflow-x: auto; padding: 0 var(--sp-4, 16px); }
  .grid { width: 100%; border-collapse: collapse; font-size: var(--fs-sm); }
  .grid th, .grid td { text-align: left; padding: 8px; border-bottom: 1px solid var(--line); }
  .grid .num { text-align: right; font-variant-numeric: tabular-nums; white-space: nowrap; }
  .fam td { font-weight: 600; color: var(--muted); padding-top: 14px; }
  .faint { color: var(--faint, var(--muted)); }
</style>
