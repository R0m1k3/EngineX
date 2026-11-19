<script lang="ts">
  // Jauge de placement VRAM par carte (élément signature) : segments poids / cache KV / calcul,
  // une ligne par GPU et le CPU. Trois tailles : mini (barre d'état), md (tableau de bord), lg (éditeur de preset).
  // Valeurs en Mio. Sans détail (nvidia-smi), `used_mib` donne un seul segment.
  import type { GaugeRow } from './types';
  import { gb } from './format';

  let {
    rows,
    size = 'md',
    legend = false,
    compact = false,
  }: { rows: GaugeRow[]; size?: 'mini' | 'md' | 'lg'; legend?: boolean; compact?: boolean } = $props();

  const pct = (v: number | undefined, t: number) => (v && t > 0 ? Math.min(100, (v / t) * 100) : 0);
  const used = (r: GaugeRow) => r.used_mib ?? (r.weights_mib ?? 0) + (r.kv_mib ?? 0) + (r.compute_mib ?? 0);
  const totals = $derived({
    w: rows.reduce((a, r) => a + (r.weights_mib ?? 0), 0),
    kv: rows.reduce((a, r) => a + (r.kv_mib ?? 0), 0),
    c: rows.reduce((a, r) => a + (r.compute_mib ?? 0), 0),
    free: rows.filter((r) => r.total_mib > 0).reduce((a, r) => a + Math.max(0, r.total_mib - used(r)), 0),
  });
  const detailed = $derived(rows.some((r) => r.weights_mib != null || r.kv_mib != null));
  const summary = $derived(rows.map((r) => `${r.label} ${gb(used(r))} / ${gb(r.total_mib)} Go`).join(', '));
</script>

{#if size === 'mini'}
  <span class="mini" role="img" aria-label="VRAM : {summary}" title={summary}>
    {#each rows.filter((r) => r.total_mib > 0) as r (r.label)}
      <span class="mtrack" style:width="{Math.max(16, Math.min(48, Math.round(r.total_mib / 512)))}px"><i style:width="{pct(used(r), r.total_mib)}%"></i></span>
    {/each}
  </span>
{:else}
  <div class="gauge {size}" class:compact>
    {#each rows as r (r.label)}
      <div class="grow-row">
        <div class="lbl">{r.label}{#if r.sub && !compact}<small>{r.sub}</small>{/if}</div>
        <div class="track" role="img" aria-label="{r.label} : {r.note ?? `${gb(used(r))} sur ${gb(r.total_mib)} Go`}">
          {#if r.used_mib != null}
            <i class="g-w" style:width="{pct(r.used_mib, r.total_mib)}%"></i>
          {:else}
            {#if r.weights_mib}<i class="g-w" style:width="{pct(r.weights_mib, r.total_mib)}%"></i>{/if}
            {#if r.kv_mib}<i class="g-kv" style:width="{pct(r.kv_mib, r.total_mib)}%"></i>{/if}
            {#if r.compute_mib}<i class="g-c" style:width="{pct(r.compute_mib, r.total_mib)}%"></i>{/if}
          {/if}
        </div>
        <div class="val" class:faint={!!r.note}>{r.note ?? `${gb(used(r))} / ${gb(r.total_mib)}${compact ? '' : ' Go'}`}</div>
      </div>
    {/each}
    {#if legend && detailed}
      <div class="legend">
        <span><i class="g-w"></i>Poids {gb(totals.w)} Go</span>
        <span><i class="g-kv"></i>Cache KV {gb(totals.kv)} Go</span>
        {#if totals.c}<span><i class="g-c"></i>Calcul {gb(totals.c)} Go</span>{/if}
        <span class="rest">Reste {gb(totals.free)} Go</span>
      </div>
    {/if}
  </div>
{/if}

<style>
  .mini { display: inline-flex; gap: 2px; align-items: center; }
  .mtrack { position: relative; height: 8px; background: var(--raised); border: 1px solid var(--line-strong); border-radius: var(--r-sm); overflow: hidden; }
  .mtrack i { position: absolute; inset: 0 auto 0 0; background: var(--accent-fill); }

  .gauge { display: grid; gap: var(--s3); font-size: var(--fs-sm); }
  .grow-row { display: grid; grid-template-columns: 148px 1fr 108px; align-items: center; gap: var(--s3); }
  .compact .grow-row { grid-template-columns: 56px 1fr 84px; }
  .lbl { display: flex; flex-direction: column; line-height: 1.3; min-width: 0; }
  .lbl small { font-size: var(--fs-xs); color: var(--muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .track { display: flex; height: 14px; background: var(--raised); border: 1px solid var(--line-strong); border-radius: var(--r-sm); overflow: hidden; }
  .lg .track { height: 18px; }
  .track i { display: block; height: 100%; }
  .track i + i { border-left: 1px solid var(--bg); }
  .val { text-align: right; font-variant-numeric: tabular-nums; white-space: nowrap; }
  .faint { color: var(--faint); }
  .g-w { background: var(--accent-fill); }
  .g-kv { background: var(--accent-2); }
  .g-c { background: repeating-linear-gradient(135deg, var(--faint) 0 2px, transparent 2px 5px); }
  .legend { display: flex; flex-wrap: wrap; gap: var(--s2) var(--s4); font-size: var(--fs-xs); color: var(--muted); }
  .legend span { display: inline-flex; align-items: center; gap: 6px; }
  .legend i { width: 12px; height: 10px; border-radius: var(--r-sm); display: inline-block; }
  .legend .rest { margin-left: auto; }
  @media (max-width: 767px) {
    /* « 14,2 / 15,9 Go » tient en entier. */
    .grow-row { grid-template-columns: 64px 1fr minmax(116px, auto); }
    .compact .grow-row { grid-template-columns: 56px 1fr minmax(84px, auto); }
    .lbl small { display: none; }
  }
</style>
