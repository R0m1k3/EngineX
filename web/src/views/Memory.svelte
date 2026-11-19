<script lang="ts">
  import {
    memoryTree, memoryFile, saveMemoryFile, memoryRevisions, memoryRevision, message,
    type MemTree, type MemScope, type MemFile, type MemRevision,
  } from '../lib/api';

  let tree = $state<MemTree | null>(null);
  let path = $state('');
  let file = $state<MemFile>('memory');
  let content = $state('');
  let base = $state('');
  let loaded = $state('');
  let error = $state('');
  let info = $state('');
  let revisions = $state<MemRevision[]>([]);
  let viewing = $state<{ id: number; content: string } | null>(null);
  let showHistory = $state(false);

  const kio = (n: number) =>
    n < 1024 ? `${n} o` : `${(n / 1024).toFixed(n < 10240 ? 1 : 0).replace('.', ',')} Kio`;
  const bytes = (s: string) => new TextEncoder().encode(s).length;
  const date = (t: number | null) => (t ? new Date(t * 1000).toLocaleString('fr-FR') : '');
  const who = (a: string | null) => (!a ? '' : a === 'michael' ? 'Michael' : a.startsWith('session:') ? `session ${a.slice(8, 16)}` : a);
  const sessionOf = (a: string | null) => (a?.startsWith('session:') ? a.slice(8) : null);

  function scope(t: MemTree, p: string): MemScope | undefined {
    if (!p) return t.global;
    const [ps, ts] = p.split('/');
    const proj = t.projects.find((x) => x.slug === ps);
    return ts ? proj?.topics.find((x) => x.slug === ts) : proj;
  }
  let current = $derived(tree ? scope(tree, path) : undefined);
  let limit = $derived(current?.[file].limit ?? (file === 'memory' ? 16384 : 4096));
  let size = $derived(bytes(content));
  let dirty = $derived(content !== loaded);
  let label = $derived(
    !path ? 'Global' : path.includes('/') ? `Topic ${path.split('/')[1]} (projet ${path.split('/')[0]})` : `Projet ${path}`,
  );

  async function loadTree() {
    try {
      tree = await memoryTree();
    } catch (e) {
      error = message(e);
    }
  }

  async function open(p: string, f: MemFile) {
    if (dirty && !confirm('Modifications non enregistrées : les abandonner ?')) return;
    path = p;
    file = f;
    error = '';
    info = '';
    viewing = null;
    try {
      const r = await memoryFile(p, f);
      content = r.content;
      loaded = r.content;
      base = r.sha;
      revisions = showHistory ? await memoryRevisions(p, f) : [];
    } catch (e) {
      error = message(e);
    }
  }

  async function save(text = content) {
    error = '';
    info = '';
    try {
      const r = await saveMemoryFile(path, file, text, base);
      base = r.sha;
      content = text;
      loaded = text;
      info = 'Enregistré.';
      viewing = null;
      await loadTree();
      if (showHistory) revisions = await memoryRevisions(path, file);
    } catch (e) {
      error = message(e);
    }
  }

  async function reload() {
    loaded = content;
    await open(path, file);
  }

  async function toggleHistory() {
    showHistory = !showHistory;
    viewing = null;
    if (showHistory) {
      try {
        revisions = await memoryRevisions(path, file);
      } catch (e) {
        error = message(e);
      }
    }
  }

  async function view(id: number) {
    try {
      const r = await memoryRevision(id);
      viewing = { id, content: r.content };
    } catch (e) {
      error = message(e);
    }
  }

  async function restore() {
    if (!viewing) return;
    if (!confirm('Remplacer le contenu actuel par cette version ? La version actuelle reste dans l’historique.')) return;
    await save(viewing.content);
  }

  loadTree().then(() => open('', 'memory'));
</script>

<header class="row">
  <h2>Mémoire</h2>
</header>
<p class="small muted">
  Notes données à Codex au début de chaque session : globales, du projet, puis du topic. Les fichiers
  <strong>instructions</strong> sont à vous (Codex les lit sans pouvoir les modifier) ; la <strong>mémoire</strong>
  est tenue par Codex pendant les sessions, et vous pouvez la corriger ici.
</p>

<div class="mem">
  <nav class="levels">
    {#if tree}
      <button class="lvl" class:active={path === ''} onclick={() => open('', file)}>
        Global <span class="muted tiny">{kio(tree.global.memory.size)}</span>
      </button>
      {#each tree.projects as p (p.slug)}
        <button class="lvl" class:active={path === p.slug} onclick={() => open(p.slug!, file)}>
          {p.name} <span class="muted tiny">{kio(p.memory.size)}</span>
        </button>
        {#each p.topics as t (t.slug)}
          {@const tp = `${p.slug}/${t.slug}`}
          <button class="lvl sub" class:active={path === tp} class:warn={t.memory.size > tree.warn}
            onclick={() => open(tp, file)}>
            {t.name} <span class="muted tiny">{kio(t.memory.size)}</span>
          </button>
        {/each}
      {/each}
    {/if}
  </nav>

  <section class="editor">
    <div class="row head">
      <strong>{label}</strong>
      <div class="tabs" role="tablist">
        <button role="tab" class:active={file === 'instructions'} aria-selected={file === 'instructions'}
          onclick={() => open(path, 'instructions')}>Instructions</button>
        <button role="tab" class:active={file === 'memory'} aria-selected={file === 'memory'}
          onclick={() => open(path, 'memory')}>Mémoire</button>
      </div>
    </div>
    <textarea bind:value={content} spellcheck="false" rows="22"
      placeholder={file === 'memory' ? '(vide) Une ligne par fait : - AAAA-MM-JJ : …' : '(vide) Vos consignes pour Codex à ce niveau.'}></textarea>
    <div class="row foot">
      <span class="small" class:warn={size > limit * 0.75} class:error={size > limit}>
        {kio(size)} / {limit / 1024} Kio
      </span>
      {#if current?.[file].updated}
        <span class="small muted">
          Dernière écriture : {date(current[file].updated)}, par
          {#if sessionOf(current[file].author)}<a href="#sessions/{sessionOf(current[file].author)}">{who(current[file].author)}</a>
          {:else}{who(current[file].author)}{/if}
        </span>
      {/if}
      <div class="spacer"></div>
      <button class="link" onclick={toggleHistory}>{showHistory ? 'Masquer l’historique' : 'Historique'}</button>
      <button onclick={() => save()} disabled={!dirty || size > limit}>Enregistrer</button>
    </div>
    {#if info}<p class="ok small">{info}</p>{/if}
    {#if error}
      <p class="error small">{error}
        {#if error.startsWith('Modifié entre-temps')}<button class="link" onclick={reload}>Recharger</button>{/if}</p>
    {/if}

    {#if showHistory}
      <div class="card history">
        <h3>Historique ({revisions.length} version{revisions.length > 1 ? 's' : ''}, 20 gardées)</h3>
        {#if !revisions.length}<p class="small muted">Aucune version enregistrée.</p>{/if}
        <table class="small">
          <tbody>
            {#each revisions as r (r.id)}
              <tr class:active={viewing?.id === r.id}>
                <td>{date(r.created_at)}</td>
                <td>{#if sessionOf(r.author)}<a href="#sessions/{sessionOf(r.author)}">{who(r.author)}</a>{:else}{who(r.author)}{/if}</td>
                <td class="muted">{kio(r.size)}</td>
                <td><button class="link" onclick={() => view(r.id)}>Voir</button></td>
              </tr>
            {/each}
          </tbody>
        </table>
        {#if viewing}
          <pre class="out">{viewing.content || '(vide)'}</pre>
          <div class="row"><button onclick={restore}>Restaurer cette version</button>
            <button class="link" onclick={() => (viewing = null)}>Fermer</button></div>
        {/if}
      </div>
    {/if}
  </section>
</div>

<style>
  .mem { display: grid; grid-template-columns: minmax(180px, 240px) 1fr; gap: var(--s4); align-items: start; }
  .levels { display: flex; flex-direction: column; gap: 2px; border-right: 1px solid var(--line); padding: var(--s4); }
  .lvl { text-align: left; background: none; border: 1px solid transparent; color: var(--text); padding: var(--s1) var(--s2);
    border-radius: var(--radius); display: flex; justify-content: space-between; gap: var(--s2); }
  .lvl.sub { padding-left: var(--s4); }
  .lvl.active { border-color: var(--accent); }
  .lvl.warn .tiny { color: var(--warn); }
  .editor { display: flex; flex-direction: column; gap: var(--s2); min-width: 0; }
  .head { justify-content: space-between; }
  .tabs button { background: none; border: none; padding: var(--s1) var(--s2); color: var(--muted); }
  .tabs button.active { color: var(--text); border-bottom: 2px solid var(--accent); }
  textarea { width: 100%; font-family: var(--mono, monospace); font-size: var(--fs-sm); resize: vertical; }
  .foot { gap: var(--s3); align-items: center; }
  .spacer { flex: 1; }
  .warn { color: var(--warn); }
  .tiny { font-size: var(--fs-sm); }
  .history table { width: 100%; }
  .history tr.active td { font-weight: 600; }
  .out { white-space: pre-wrap; max-height: 320px; overflow: auto; }
</style>
