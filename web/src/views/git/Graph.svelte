<script lang="ts">
  import { stamp } from '../../lib/ui/format';
  import type { GitCommit } from '../../lib/api';
  import Icon from '../../lib/ui/Icon.svelte';
  import Tag from '@lucide/svelte/icons/tag';
  import Cloud from '@lucide/svelte/icons/cloud';

  let { commits, selected, select }: { commits: GitCommit[]; selected: string; select: (sha: string) => void } = $props();

  const W = 16;
  // Hauteur d'une ligne : 36 au bureau, 48 au téléphone (cible tactile).
  const phone = typeof matchMedia === 'function' ? matchMedia('(max-width: 767px)') : null;
  let small = $state(phone?.matches ?? false);
  $effect(() => {
    if (!phone) return;
    const on = () => (small = phone.matches);
    phone.addEventListener('change', on);
    return () => phone.removeEventListener('change', on);
  });
  let H = $derived(small ? 48 : 36);
  // Couleurs des couloirs tirées des jetons (clair et sombre suivent le thème).
  const COLORS = ['--accent', '--code-fn', '--warn', '--code-type', '--code-tag', '--term-cyan', '--code-number', '--accent-2'];
  const color = (i: number) => `var(${COLORS[i % COLORS.length]})`;

  interface Row {
    c: GitCommit;
    lane: number;
    /** Segments [x1, y1, x2, y2, couleur] dans la ligne. */
    segs: [number, number, number, number, string][];
    width: number;
  }

  // Couloirs : chaque couloir attend un sha ; un commit prend le premier
  // couloir qui l'attend, ses parents prennent la suite.
  let rows = $derived.by(() => {
    const lanes: (string | null)[] = [];
    const out: Row[] = [];
    let maxw = 1;
    for (const c of commits) {
      let lane = lanes.indexOf(c.sha);
      if (lane < 0) {
        lane = lanes.indexOf(null);
        if (lane < 0) lane = lanes.push(null) - 1;
      }
      const before = lanes.slice();
      // Autres couloirs qui attendaient ce commit : ils s'y rejoignent.
      for (let j = 0; j < lanes.length; j++) if (j !== lane && lanes[j] === c.sha) lanes[j] = null;
      lanes[lane] = c.parents[0] ?? null;
      const extra: number[] = [];
      for (const p of c.parents.slice(1)) {
        let k = lanes.indexOf(p);
        if (k < 0) {
          k = lanes.indexOf(null);
          if (k < 0) k = lanes.push(null) - 1;
          lanes[k] = p;
        }
        extra.push(k);
      }
      while (lanes.length && lanes[lanes.length - 1] === null) lanes.pop();
      const x = (i: number) => i * W + W / 2;
      const segs: Row['segs'] = [];
      before.forEach((sha, j) => {
        if (!sha) return;
        if (sha === c.sha) segs.push([x(j), 0, x(lane), H / 2, color(j)]);
        else {
          const k = lanes.indexOf(sha);
          if (k >= 0) segs.push([x(j), 0, x(k), H, color(k)]);
        }
      });
      if (c.parents[0] && lanes[lane] === c.parents[0]) segs.push([x(lane), H / 2, x(lane), H, color(lane)]);
      for (const k of extra) segs.push([x(lane), H / 2, x(k), H, color(k)]);
      const width = Math.max(before.length, lanes.length, lane + 1);
      maxw = Math.max(maxw, width);
      out.push({ c, lane, segs, width });
    }
    return { rows: out, width: maxw * W + 4 };
  });

  // Courbe douce quand un trait change de couloir, droite sinon.
  const path = (s: Row['segs'][number]) =>
    s[0] === s[2] ? `M${s[0]} ${s[1]}V${s[3]}` : `M${s[0]} ${s[1]}C${s[0]} ${(s[1] + s[3]) / 2} ${s[2]} ${(s[1] + s[3]) / 2} ${s[2]} ${s[3]}`;

  const when = stamp;
</script>

<div class="graph-rows">
  {#each rows.rows as r (r.c.sha)}
    <button class="crow" class:sel={selected === r.c.sha} onclick={() => select(r.c.sha)} aria-current={selected === r.c.sha || undefined}>
      <svg width={rows.width} height={H} aria-hidden="true">
        {#each r.segs as s}<path d={path(s)} style:stroke={s[4]} />{/each}
        {#if r.c.parents.length > 1}
          <circle class="merge" cx={r.lane * W + W / 2} cy={H / 2} r="4" style:stroke={color(r.lane)} />
        {:else}
          <circle class="dot" cx={r.lane * W + W / 2} cy={H / 2} r="4.5" style:fill={color(r.lane)} />
        {/if}
      </svg>
      <span class="subject">
        {#each r.c.refs as ref}
          <span class="ref {ref.kind}">
            {#if ref.kind === 'tag'}<Icon icon={Tag} size={12} />{:else if ref.kind === 'remote'}<Icon icon={Cloud} size={12} />{/if}{ref.name}
          </span>
        {/each}
        <span class="txt">{r.c.subject}</span>
      </span>
      <span class="author">{r.c.author}</span>
      <span class="time">{when(r.c.time)}</span>
      <code class="sha">{r.c.sha.slice(0, 7)}</code>
    </button>
  {/each}
</div>

<style>
  .graph-rows { display: flex; flex-direction: column; min-width: 0; padding: var(--s1) 0; }
  .crow {
    box-sizing: border-box;
    width: 100%;
    min-width: 0;
    height: 36px;
    min-height: 0;
    display: flex;
    align-items: center;
    justify-content: flex-start;
    gap: var(--s3);
    padding: 0 var(--s4) 0 var(--s3);
    border: 0;
    border-radius: 0;
    background: none;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-sm);
    font-weight: 400;
    text-align: left;
    cursor: pointer;
    filter: none;
  }
  .crow:hover { background: var(--raised); }
  .crow.sel { background: var(--accent-soft); box-shadow: inset 2px 0 0 var(--accent); }
  .crow:focus-visible { outline-offset: -2px; }
  svg { flex: none; overflow: visible; }
  path { fill: none; stroke-width: 2; stroke-linecap: round; }
  .dot { stroke: var(--bg); stroke-width: 2; }
  .crow:hover .dot { stroke: var(--raised); }
  .crow.sel .dot { stroke: var(--accent-soft); }
  .merge { fill: var(--bg); stroke-width: 2; }
  .subject { flex: 1; min-width: 0; display: flex; align-items: center; gap: 6px; overflow: hidden; white-space: nowrap; }
  .txt { overflow: hidden; text-overflow: ellipsis; min-width: 0; }
  .ref {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 3px;
    height: 20px;
    padding: 0 6px;
    border-radius: var(--r-sm);
    font-family: var(--mono);
    font-size: var(--fs-count);
    background: var(--raised);
    color: var(--muted);
  }
  .ref.branch { background: var(--accent-soft); color: var(--accent); }
  .ref.tag { background: var(--warn-soft); color: var(--warn); }
  .crow:hover .ref.remote { background: var(--bg); }
  .author { flex: none; width: 120px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--muted); }
  .time { flex: none; width: 116px; color: var(--muted); font-variant-numeric: tabular-nums; white-space: nowrap; }
  .sha { flex: none; width: 7ch; color: var(--faint); }

  .graph-rows { container-type: inline-size; }
  @container (max-width: 760px) { .author { display: none; } }
  @container (max-width: 540px) { .time { display: none; } }
  @container (max-width: 400px) { .sha { display: none; } }
  @media (max-width: 767px) {
    .crow { height: 48px; padding-right: var(--s3); }
  }
</style>
