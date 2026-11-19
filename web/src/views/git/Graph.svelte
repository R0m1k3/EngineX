<script lang="ts">
  import type { GitCommit } from '../../lib/api';

  let { commits, selected, select }: { commits: GitCommit[]; selected: string; select: (sha: string) => void } = $props();

  const W = 14;
  const H = 26;
  const COLORS = ['#6fb3a3', '#d6a35c', '#8fa6e0', '#d97a9e', '#9cc76b', '#c08fe0', '#e0c46f', '#6fc9d6'];
  const color = (i: number) => COLORS[i % COLORS.length];

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
    return { rows: out, width: maxw * W };
  });

  const when = (t: number) => new Date(t * 1000).toLocaleString('fr-FR', { dateStyle: 'short', timeStyle: 'short' });
</script>

<div class="graph">
  {#each rows.rows as r (r.c.sha)}
    <button class="row-c" class:sel={selected === r.c.sha} onclick={() => select(r.c.sha)}>
      <svg width={rows.width} height={H} aria-hidden="true">
        {#each r.segs as s}<line x1={s[0]} y1={s[1]} x2={s[2]} y2={s[3]} stroke={s[4]} stroke-width="2" />{/each}
        <circle cx={r.lane * W + W / 2} cy={H / 2} r={r.c.parents.length > 1 ? 3 : 4} fill={color(r.lane)} />
      </svg>
      <span class="subject">
        {#each r.c.refs as ref}<span class="ref {ref.kind}">{ref.name}</span>{/each}
        {r.c.subject}
      </span>
      <span class="meta muted">{r.c.author} · {when(r.c.time)} · <code>{r.c.sha.slice(0, 7)}</code></span>
    </button>
  {/each}
</div>

<style>
  .graph { display: flex; flex-direction: column; min-width: 0; }
  .row-c { all: unset; box-sizing: border-box; width: 100%; min-width: 0; display: flex; align-items: center; gap: var(--s2); height: 26px; cursor: pointer; font-size: var(--fs-sm); padding-right: var(--s2); border-radius: 4px; }
  .row-c:hover { background: var(--bg); }
  .row-c.sel { background: color-mix(in srgb, var(--accent) 15%, transparent); }
  svg { flex: none; }
  .subject { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .meta { flex: none; font-size: var(--fs-sm); white-space: nowrap; }
  .ref { font-size: 11px; border-radius: 3px; padding: 0 5px; margin-right: 4px; border: 1px solid var(--line); }
  .ref.branch { color: var(--accent); border-color: var(--accent); }
  .ref.remote { color: #8fa6e0; border-color: #8fa6e0; }
  .ref.tag { color: #d6a35c; border-color: #d6a35c; }
  @media (max-width: 900px) { .meta { display: none; } }
</style>
