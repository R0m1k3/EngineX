<script lang="ts">
  import {
    ApiError,
    diskUsage,
    message,
    wsDelete,
    wsDownloadUrl,
    wsMkdir,
    wsMove,
    wsRawUrl,
    wsSave,
    wsText,
    wsTree,
    wsUpload,
    wsZipUrl,
    type DiskUsage,
    type WsEntry,
  } from '../lib/api';
  import Markdown from './sessions/Markdown.svelte';
  import Editor from './files/Editor.svelte';

  interface Dir {
    entries: WsEntry[];
    truncated: boolean;
  }
  let dirs = $state<Record<string, Dir>>({});
  let opened = $state<Record<string, boolean>>({ '': true });
  let checked = $state<Record<string, boolean>>({});
  /** Fichier ouvert à droite. */
  let selected = $state('');
  /** Dossier cible (nouveau, envoi). */
  let folder = $state('');
  let error = $state('');
  let notice = $state('');
  let disk = $state<DiskUsage | null>(null);

  type Doc = { kind: string; text?: string; version?: number; readonly?: boolean; truncated?: boolean };
  let doc = $state<Doc | null>(null);
  let dirty = $state(false);
  let saving = $state(false);
  let conflict = $state(false);
  let mdSource = $state(false);
  let editor = $state<ReturnType<typeof Editor> | null>(null);
  let uploadInput: HTMLInputElement;

  const join = (dir: string, name: string) => (dir ? `${dir}/${name}` : name);
  const parent = (p: string) => (p.includes('/') ? p.slice(0, p.lastIndexOf('/')) : '');
  const base = (p: string) => p.split('/').pop() ?? p;
  const size = (n: number) =>
    n < 1024 ? `${n} o` : n < 1 << 20 ? `${(n / 1024).toFixed(0)} Kio` : n < 1 << 30 ? `${(n / (1 << 20)).toFixed(1)} Mio` : `${(n / (1 << 30)).toFixed(1)} Gio`;

  let picked = $derived(Object.keys(checked).filter((p) => checked[p]));

  function entryOf(path: string): WsEntry | undefined {
    return dirs[parent(path)]?.entries.find((e) => e.name === base(path));
  }

  async function load(path: string) {
    try {
      const r = await wsTree(path);
      dirs[path] = { entries: r.entries, truncated: r.truncated };
      error = '';
    } catch (e) {
      error = message(e);
    }
  }

  async function toggle(path: string) {
    opened[path] = !opened[path];
    folder = path;
    if (opened[path] && !dirs[path]) await load(path);
  }

  async function refresh() {
    const open = Object.keys(opened).filter((p) => opened[p]);
    dirs = {};
    await Promise.all(open.map(load));
    diskUsage().then((d) => (disk = d)).catch(() => {});
  }

  const ext = (p: string) => p.split('.').pop()?.toLowerCase() ?? '';
  const KIND: Record<string, string> = {
    png: 'image', jpg: 'image', jpeg: 'image', gif: 'image', webp: 'image', avif: 'image', svg: 'image',
    pdf: 'pdf', mp4: 'video', webm: 'video', mp3: 'audio', ogg: 'audio', wav: 'audio', md: 'markdown',
    zip: 'binary', gz: 'binary', tar: 'binary', gguf: 'binary', exe: 'binary', bin: 'binary',
  };

  function leave(): boolean {
    return !dirty || confirm('Modifications non enregistrées : les abandonner ?');
  }

  async function open(path: string) {
    if (path !== selected && !leave()) return;
    selected = path;
    folder = parent(path);
    dirty = false;
    conflict = false;
    mdSource = false;
    const kind = KIND[ext(path)] ?? 'text';
    if (kind !== 'text' && kind !== 'markdown') {
      doc = { kind };
      return;
    }
    doc = { kind: 'loading' };
    try {
      const r = await wsText(path);
      doc = r.binary ? { kind: 'binary' } : { kind, text: r.text, version: r.version, readonly: r.readonly, truncated: r.truncated };
    } catch (e) {
      doc = { kind: 'error', text: message(e) };
    }
  }

  async function save(force = false) {
    if (!doc || doc.readonly || !editor || saving) return;
    const text = editor.current();
    saving = true;
    try {
      let version = doc.version;
      if (force) version = (await wsText(selected)).version;
      const r = await wsSave(selected, text, version);
      doc.version = r.version;
      doc.text = text;
      dirty = false;
      conflict = false;
      notice = 'Enregistré.';
      setTimeout(() => (notice = ''), 2000);
    } catch (e) {
      if (e instanceof ApiError && e.code === 'file_changed') conflict = true;
      error = message(e);
    } finally {
      saving = false;
    }
  }

  async function reload() {
    dirty = false;
    const p = selected;
    selected = '';
    await open(p);
  }

  async function act(f: () => Promise<unknown>, done?: string) {
    error = '';
    try {
      await f();
      if (done) notice = done;
      setTimeout(() => (notice = ''), 2500);
    } catch (e) {
      error = message(e);
    }
    await refresh();
  }

  function newFolder() {
    const name = prompt(`Nouveau dossier dans /${folder || ''}`);
    if (!name?.trim()) return;
    act(() => wsMkdir(join(folder, name.trim())));
  }

  function newFile() {
    const name = prompt(`Nouveau fichier dans /${folder || ''}`);
    if (!name?.trim()) return;
    const p = join(folder, name.trim());
    act(async () => {
      await wsSave(p, '');
      opened[folder] = true;
    }).then(() => open(p));
  }

  async function onUpload() {
    const files = Array.from(uploadInput.files ?? []);
    uploadInput.value = '';
    if (!files.length) return;
    await act(async () => {
      for (const f of files) {
        const p = join(folder, f.name);
        try {
          await wsUpload(p, f);
        } catch (e) {
          if (e instanceof ApiError && e.code === 'name_taken' && confirm(`${p} existe : le remplacer ?`)) {
            await wsUpload(p, f, true);
          } else throw e;
        }
      }
      opened[folder] = true;
    }, `${files.length} fichier(s) envoyé(s).`);
  }

  function targets(): string[] {
    return picked.length ? picked : selected ? [selected] : [];
  }

  function rename() {
    const t = targets();
    if (t.length !== 1) return alert('Cochez un seul élément à renommer.');
    const name = prompt('Nouveau nom', base(t[0]));
    if (!name?.trim() || name === base(t[0])) return;
    const to = join(parent(t[0]), name.trim());
    act(async () => {
      await wsMove(t[0], to);
      if (selected === t[0]) selected = to;
      checked = {};
    });
  }

  function move() {
    const t = targets();
    if (!t.length) return;
    const dest = prompt('Déplacer vers le dossier (chemin depuis la racine)', folder);
    if (dest === null) return;
    const d = dest.trim().replace(/^\/+|\/+$/g, '');
    act(async () => {
      for (const p of t) await wsMove(p, join(d, base(p)));
      checked = {};
      if (t.includes(selected)) selected = '';
    }, 'Déplacé.');
  }

  function remove() {
    const t = targets();
    if (!t.length || !confirm(`Supprimer définitivement ${t.length === 1 ? t[0] : `${t.length} éléments`} ?`)) return;
    act(async () => {
      await wsDelete(t);
      checked = {};
      if (t.some((p) => selected === p || selected.startsWith(`${p}/`))) {
        selected = '';
        doc = null;
        dirty = false;
      }
    }, 'Supprimé.');
  }

  function zip() {
    const t = targets();
    if (t.length) location.href = wsZipUrl(t);
  }

  function beforeUnload(e: BeforeUnloadEvent) {
    if (dirty) e.preventDefault();
  }

  refresh();
</script>

<svelte:window onbeforeunload={beforeUnload} />

{#snippet level(path: string, depth: number)}
  {#each dirs[path]?.entries ?? [] as e (e.name)}
    {@const p = join(path, e.name)}
    <div class="line" style:padding-left="{depth * 14}px">
      <input type="checkbox" bind:checked={checked[p]} disabled={e.managed} aria-label="Choisir {e.name}" />
      {#if e.kind === 'dir'}
        <button class="link entry" class:here={folder === p} onclick={() => toggle(p)}>
          {opened[p] ? '▾' : '▸'} {e.name}/{#if e.managed}<span class="tag">géré</span>{/if}
        </button>
      {:else}
        <button class="link entry" class:sel={selected === p} onclick={() => e.kind === 'file' && open(p)}
          disabled={e.kind !== 'file'} title={e.kind === 'link' ? 'Lien symbolique (jamais suivi)' : size(e.size)}>
          {e.name}{e.kind === 'link' ? ' →' : ''}
        </button>
      {/if}
    </div>
    {#if e.kind === 'dir' && opened[p]}{@render level(p, depth + 1)}{/if}
  {/each}
  {#if dirs[path]?.truncated}<p class="muted tiny" style:padding-left="{depth * 14}px">1000 entrées au plus.</p>{/if}
{/snippet}

<section class="page">
  <div class="row head">
    <h1>Fichiers</h1>
    {#if disk}
      <span class="muted small">
        Workspace {size(disk.workspace_bytes)} · sessions {size(disk.sessions_bytes)} · disque {disk.used_pct} % utilisé, {size(disk.free_bytes)} libres
      </span>
    {/if}
  </div>
  {#if disk?.alert}
    <p class="banner">Disque de /data rempli à {disk.used_pct} % (alerte à {disk.alert_pct} %, réglable dans Paramètres → Sessions) : supprimez des sessions ou des fichiers.</p>
  {/if}
  <div class="actions">
    <span class="muted small">Dossier : <code>/{folder}</code></span>
    <button class="secondary" onclick={newFolder}>Nouveau dossier</button>
    <button class="secondary" onclick={newFile}>Nouveau fichier</button>
    <button class="secondary" onclick={() => uploadInput.click()}>Envoyer…</button>
    <input type="file" multiple hidden bind:this={uploadInput} onchange={onUpload} />
    <span class="sep"></span>
    <span class="muted small">{picked.length ? `${picked.length} coché(s)` : selected ? 'Fichier ouvert' : ''}</span>
    <button class="secondary" onclick={rename} disabled={!targets().length}>Renommer</button>
    <button class="secondary" onclick={move} disabled={!targets().length}>Déplacer</button>
    <button class="secondary" onclick={zip} disabled={!targets().length}>Zip</button>
    <button class="secondary danger" onclick={remove} disabled={!targets().length}>Supprimer</button>
    <button class="link" onclick={refresh}>Actualiser</button>
  </div>
  {#if error}<p class="error small pre">{error}</p>{/if}
  {#if notice}<p class="muted small">{notice}</p>{/if}

  <div class="split">
    <div class="tree card">
      <button class="link entry" class:here={folder === ''} onclick={() => (folder = '')}>/ (workspace)</button>
      {@render level('', 0)}
    </div>
    <div class="view card">
      {#if selected && doc}
        <div class="row small filebar">
          <code class="name">{selected}{dirty ? ' •' : ''}</code>
          <span class="tools">
            {#if doc.kind === 'markdown'}
              <button class="link" onclick={() => (mdSource = !mdSource)}>{mdSource ? 'Aperçu' : doc.readonly ? 'Source' : 'Modifier'}</button>
            {/if}
            {#if (doc.kind === 'text' || (doc.kind === 'markdown' && mdSource)) && !doc.readonly}
              <button onclick={() => save()} disabled={!dirty || saving}>{saving ? 'Enregistrement…' : 'Enregistrer'}</button>
            {/if}
            <a href={wsDownloadUrl(selected)} download>Télécharger</a>
          </span>
        </div>
        {#if conflict}
          <p class="banner small">
            Le fichier a changé sur le disque depuis son ouverture (une session ?).
            <button class="link" onclick={reload}>Recharger (perdre mes changements)</button> ·
            <button class="link" onclick={() => save(true)}>Écraser avec ma version</button>
          </p>
        {/if}
        {#if doc.readonly && (doc.kind === 'text' || doc.kind === 'markdown')}
          <p class="muted tiny">
            {doc.truncated ? 'Trop gros pour l’éditeur (2 Mio au plus) : lecture seule, téléchargez-le.' : 'Lecture seule (la mémoire se modifie depuis la page Mémoire).'}
          </p>
        {/if}
        <div class="body">
          {#if doc.kind === 'image'}<img src={wsRawUrl(selected)} alt={selected} />
          {:else if doc.kind === 'pdf'}<iframe src={wsRawUrl(selected)} title={selected}></iframe>
          {:else if doc.kind === 'video'}<!-- svelte-ignore a11y_media_has_caption --><video src={wsRawUrl(selected)} controls></video>
          {:else if doc.kind === 'audio'}<audio src={wsRawUrl(selected)} controls></audio>
          {:else if doc.kind === 'markdown' && !mdSource}<div class="md"><Markdown text={doc.text ?? ''} /></div>
          {:else if doc.kind === 'text' || doc.kind === 'markdown'}
            {#key selected}
              <Editor bind:this={editor} path={selected} text={doc.text ?? ''} readonly={doc.readonly}
                onsave={() => save()} ondirty={() => (dirty = true)} />
            {/key}
          {:else if doc.kind === 'binary'}<p class="muted small">Fichier binaire : téléchargez-le pour l’ouvrir.</p>
          {:else if doc.kind === 'error'}<p class="error small">{doc.text}</p>
          {:else}<p class="muted small">Chargement…</p>{/if}
        </div>
      {:else}
        <p class="muted small">
          Tout le workspace : projets, topics (le code des sessions), skills, tâches. Cochez pour agir sur plusieurs éléments.
          Les dossiers marqués « géré » se créent et se suppriment depuis Sessions, Mémoire ou Skills.
        </p>
      {/if}
    </div>
  </div>
</section>

<style>
  .page { display: flex; flex-direction: column; gap: var(--s3); height: calc(100vh - 2 * var(--s6)); min-height: 0; min-width: 0; }
  .head { justify-content: flex-start; gap: var(--s4); flex-wrap: wrap; }
  .head h1 { margin: 0; }
  .actions { display: flex; gap: var(--s2); align-items: center; flex-wrap: wrap; }
  .actions button { padding: var(--s1) var(--s3); font-size: var(--fs-sm); }
  .sep { flex: 1; }
  .danger { color: var(--error); }
  .split { display: grid; grid-template-columns: minmax(240px, 340px) minmax(0, 1fr); grid-template-rows: minmax(0, 1fr); gap: var(--s3); flex: 1; min-height: 0; }
  .tree { overflow: auto; min-height: 0; padding: var(--s3); }
  .view { overflow: hidden; min-height: 0; min-width: 0; padding: var(--s3); display: flex; flex-direction: column; gap: var(--s2); }
  .body { flex: 1; min-height: 0; overflow: auto; display: flex; flex-direction: column; }
  .body > :global(.cm) { flex: 1; }
  .line { display: flex; align-items: center; gap: var(--s1); }
  .line input { margin: 0; }
  .entry { display: block; flex: 1; padding-top: 1px; padding-bottom: 1px; font-size: var(--fs-sm); color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .entry:disabled { color: var(--muted); opacity: 1; }
  .entry.sel, .entry.here { color: var(--accent); }
  .tag { margin-left: var(--s2); font-size: 11px; color: var(--muted); border: 1px solid var(--line); border-radius: 3px; padding: 0 4px; }
  .filebar { gap: var(--s3); flex: none; }
  .view > p, .view > .banner { flex: none; }
  .tools { display: flex; gap: var(--s3); align-items: center; }
  .tools button:not(.link) { padding: var(--s1) var(--s3); font-size: var(--fs-sm); }
  .name { overflow: hidden; text-overflow: ellipsis; }
  .banner { background: color-mix(in srgb, var(--error) 15%, transparent); border: 1px solid var(--error); border-radius: var(--radius); padding: var(--s2) var(--s3); margin: 0; }
  .body img, .body video { max-width: 100%; }
  iframe { width: 100%; height: 100%; min-height: 70vh; border: 1px solid var(--line); border-radius: var(--radius); background: white; }
  .tiny { font-size: var(--fs-sm); margin: 0; }
  .pre { white-space: pre-wrap; }
  a { color: var(--accent); }
  @media (max-width: 800px) { .split { grid-template-columns: 1fr; } }
</style>
