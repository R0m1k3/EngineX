<script lang="ts">
  // Diff unifié lu ligne à ligne : en-tête par fichier (collant), blocs @@, numéros de ligne,
  // ajouts et retraits sur fond add/del atténué.
  let { diff, truncated = false }: { diff: string; truncated?: boolean } = $props();

  type Line =
    | { t: 'file'; path: string; key: number }
    | { t: 'meta' | 'hunk'; text: string; key: number }
    | { t: 'add' | 'del' | 'ctx'; n: number | ''; text: string; key: number };

  let lines = $derived.by(() => {
    const out: Line[] = [];
    let o = 0;
    let n = 0;
    diff.split('\n').forEach((l, key) => {
      if (l.startsWith('diff --git')) {
        const m = / b\/(.+)$/.exec(l);
        out.push({ t: 'file', path: m ? m[1] : l.slice(11), key });
      } else if (l.startsWith('@@')) {
        const m = /^@@ -(\d+)(?:,\d+)? \+(\d+)/.exec(l);
        if (m) {
          o = Number(m[1]);
          n = Number(m[2]);
        }
        out.push({ t: 'hunk', text: l, key });
      } else if (l.startsWith('+++') || l.startsWith('---') || l.startsWith('index ')) {
        // Redondant avec l'en-tête du fichier.
      } else if (l[0] === '+') out.push({ t: 'add', n: n++, text: l.slice(1), key });
      else if (l[0] === '-') out.push({ t: 'del', n: o++, text: l.slice(1), key });
      else if (l[0] === ' ') {
        out.push({ t: 'ctx', n, text: l.slice(1), key });
        o++;
        n++;
      } else if (l.trim()) out.push({ t: 'meta', text: l, key });
    });
    return out;
  });
</script>

{#if diff.trim()}
  <div class="diffv">
    {#each lines as l (l.key)}
      {#if l.t === 'file'}
        <div class="fh"><code class="ell">{l.path}</code></div>
      {:else if l.t === 'hunk'}
        <div class="hunk">{l.text}</div>
      {:else if l.t === 'meta'}
        <div class="meta">{l.text}</div>
      {:else if l.t === 'add' || l.t === 'del' || l.t === 'ctx'}
        <div class="ln {l.t}"><span class="no">{l.n}</span><span class="sg">{l.t === 'add' ? '+' : l.t === 'del' ? '−' : ''}</span><span class="tx">{l.text}</span></div>
      {/if}
    {/each}
  </div>
  {#if truncated}<p class="trunc">Diff tronqué (512 Kio).</p>{/if}
{:else}
  <p class="trunc">Pas de diff texte.</p>
{/if}

<style>
  .diffv { font-family: var(--mono); font-size: var(--fs-code); line-height: 1.6; min-width: 0; }
  .fh {
    position: sticky;
    top: 0;
    left: 0;
    z-index: 1;
    display: flex;
    align-items: center;
    height: 34px;
    padding: 0 var(--s4);
    background: var(--panel);
    border-top: 1px solid var(--line);
    border-bottom: 1px solid var(--line);
    font-weight: 600;
    color: var(--text);
  }
  .fh:first-child { border-top: 0; }
  .hunk { padding: 2px var(--s4); color: var(--faint); white-space: pre; }
  .meta { padding: 2px var(--s4); color: var(--faint); font-family: var(--font); font-size: var(--fs-sm); }
  .ln { display: grid; grid-template-columns: 44px 16px max-content; white-space: pre; min-width: 100%; width: max-content; }
  .no { color: var(--faint); text-align: right; padding-right: var(--s2); user-select: none; font-variant-numeric: tabular-nums; }
  .sg { user-select: none; }
  .tx { padding-right: var(--s4); }
  .add { background: var(--add-bg); color: var(--add-fg); }
  .del { background: var(--del-bg); color: var(--del-fg); }
  .add .no, .del .no { color: inherit; }
  .trunc { margin: 0; padding: var(--s3) var(--s4); color: var(--faint); font-size: var(--fs-sm); }
</style>
