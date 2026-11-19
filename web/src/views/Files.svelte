<script lang="ts">
  import { dec1, octets } from '../lib/ui/format';
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
  import { Button, EmptyState, Icon, IconButton, ProgressBar, Segmented, confirmDialog, promptDialog, toast } from '../lib/ui';
  import ArrowLeft from '@lucide/svelte/icons/arrow-left';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import Download from '@lucide/svelte/icons/download';
  import FileArchive from '@lucide/svelte/icons/file-archive';
  import FilePlus from '@lucide/svelte/icons/file-plus';
  import Files from '@lucide/svelte/icons/files';
  import Folder from '@lucide/svelte/icons/folder';
  import FolderInput from '@lucide/svelte/icons/folder-input';
  import FolderOpen from '@lucide/svelte/icons/folder-open';
  import FolderPlus from '@lucide/svelte/icons/folder-plus';
  import HardDrive from '@lucide/svelte/icons/hard-drive';
  import Link2 from '@lucide/svelte/icons/link-2';
  import Pencil from '@lucide/svelte/icons/pencil';
  import RefreshCw from '@lucide/svelte/icons/refresh-cw';
  import Save from '@lucide/svelte/icons/save';
  import Trash from '@lucide/svelte/icons/trash-2';
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
  import Upload from '@lucide/svelte/icons/upload';
  import X from '@lucide/svelte/icons/x';
  import Markdown from './sessions/Markdown.svelte';
  import Editor from './files/Editor.svelte';
  import { fileIcon } from './files/icons';

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
  let disk = $state<DiskUsage | null>(null);
  /** Téléphone : l'arbre ou le fichier ouvert, en plein écran. */
  let pane = $state<'tree' | 'doc'>('tree');

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
  const size = octets;

  let picked = $derived(Object.keys(checked).filter((p) => checked[p]));
  let crumbs = $derived(selected ? selected.split('/') : []);
  let editable = $derived(!!doc && (doc.kind === 'text' || (doc.kind === 'markdown' && mdSource)) && !doc.readonly);

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

  async function leave(): Promise<boolean> {
    return (
      !dirty ||
      confirmDialog(`Les modifications de ${base(selected)} ne sont pas enregistrées.`, {
        title: 'Abandonner les modifications ?',
        confirm: 'Abandonner',
        cancel: 'Continuer à modifier',
        danger: true,
      })
    );
  }

  async function open(path: string) {
    if (path !== selected && !(await leave())) return;
    selected = path;
    pane = 'doc';
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
      toast(`${base(selected)} enregistré.`, 'ok', 2000);
    } catch (e) {
      if (e instanceof ApiError && e.code === 'file_changed') conflict = true;
      else toast(message(e), 'error');
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
    try {
      await f();
      if (done) toast(done, 'ok', 2500);
    } catch (e) {
      toast(message(e), 'error');
    }
    await refresh();
  }

  const where = (d: string) => `/${d}`;

  async function newFolder() {
    const name = await promptDialog(`Nom du dossier, dans ${where(folder)}`, { title: 'Nouveau dossier', confirm: 'Créer le dossier', mono: true });
    if (!name?.trim()) return;
    act(() => wsMkdir(join(folder, name.trim())));
  }

  async function newFile() {
    const name = await promptDialog(`Nom du fichier, dans ${where(folder)}`, { title: 'Nouveau fichier', confirm: 'Créer le fichier', mono: true });
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
          if (
            e instanceof ApiError &&
            e.code === 'name_taken' &&
            (await confirmDialog(`${p} existe déjà.`, { title: 'Remplacer le fichier ?', confirm: 'Remplacer', danger: true }))
          ) {
            await wsUpload(p, f, true);
          } else throw e;
        }
      }
      opened[folder] = true;
    }, files.length === 1 ? `${files[0].name} envoyé.` : `${files.length} fichiers envoyés.`);
  }

  function targets(): string[] {
    return picked.length ? picked : selected ? [selected] : [];
  }

  async function rename() {
    const t = targets();
    if (t.length !== 1) return toast('Cochez un seul élément à renommer.', 'warn');
    const name = await promptDialog('Nouveau nom', { title: 'Renommer', value: base(t[0]), confirm: 'Renommer', mono: true, selectStem: true });
    if (!name?.trim() || name === base(t[0])) return;
    const to = join(parent(t[0]), name.trim());
    act(async () => {
      await wsMove(t[0], to);
      if (selected === t[0]) selected = to;
      checked = {};
    });
  }

  async function move() {
    const t = targets();
    if (!t.length) return;
    const dest = await promptDialog('Vers le dossier', {
      title: t.length === 1 ? `Déplacer ${base(t[0])}` : `Déplacer ${t.length} éléments`,
      allowEmpty: true,
      selectStem: true,
      hint: 'Chemin depuis la racine du workspace, vide pour la racine.',
      value: folder,
      confirm: 'Déplacer',
      mono: true,
    });
    if (dest == null) return;
    const d = dest.trim().replace(/^\/+|\/+$/g, '');
    act(async () => {
      for (const p of t) await wsMove(p, join(d, base(p)));
      checked = {};
      if (t.includes(selected)) selected = '';
    }, 'Déplacé.');
  }

  async function remove() {
    const t = targets();
    if (!t.length) return;
    const ok = await confirmDialog(`${t.length === 1 ? t[0] : `${t.length} éléments`} : la suppression est définitive.`, {
      title: 'Supprimer ?',
      confirm: 'Supprimer',
      danger: true,
    });
    if (!ok) return;
    act(async () => {
      await wsDelete(t);
      checked = {};
      if (t.some((p) => selected === p || selected.startsWith(`${p}/`))) {
        selected = '';
        doc = null;
        dirty = false;
        pane = 'tree';
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
    {@const dir = e.kind === 'dir'}
    <div
      class="node"
      class:sel={!dir && selected === p}
      class:here={dir && folder === p}
      class:picked={checked[p]}
      style:--d={depth}
    >
      {#if dir}
        <button class="hit" onclick={() => toggle(p)} aria-expanded={!!opened[p]}>
          <span class="chev" class:open={opened[p]}><Icon icon={ChevronRight} size={16} /></span>
          <Icon icon={opened[p] ? FolderOpen : Folder} size={16} class="ficon" />
          <span class="nm">{e.name}</span>
          {#if e.managed}<span class="tag" title="Se crée et se supprime depuis Sessions, Mémoire ou Skills">géré</span>{/if}
        </button>
      {:else}
        <button
          class="hit"
          onclick={() => e.kind === 'file' && open(p)}
          disabled={e.kind !== 'file'}
          title={e.kind === 'link' ? 'Lien symbolique (jamais suivi)' : size(e.size)}
        >
          <span class="chev"></span>
          <Icon icon={e.kind === 'link' ? Link2 : fileIcon(e.name)} size={16} class="ficon" />
          <span class="nm">{e.name}</span>
          {#if selected === p && dirty}<span class="dirty" aria-label="modifié"></span>{/if}
        </button>
      {/if}
      {#if !e.managed}
        <label class="ck">
          <input type="checkbox" bind:checked={checked[p]} aria-label="Choisir {e.name}" />
        </label>
      {/if}
    </div>
    {#if dir && opened[p]}{@render level(p, depth + 1)}{/if}
  {/each}
  {#if dirs[path]?.truncated}<p class="more" style:--d={depth}>1000 entrées au plus.</p>{/if}
{/snippet}

<div class="fx" class:doc={pane === 'doc'} class:picking={picked.length > 0}>
  <aside class="side" aria-label="Arbre du workspace">
    <header class="side-h">
      <h1>Fichiers</h1>
      <span class="tools">
        <IconButton icon={FilePlus} label="Nouveau fichier dans {where(folder)}" onclick={newFile} tip="bottom" />
        <IconButton icon={FolderPlus} label="Nouveau dossier dans {where(folder)}" onclick={newFolder} tip="bottom" />
        <IconButton icon={Upload} label="Envoyer des fichiers dans {where(folder)}" onclick={() => uploadInput.click()} tip="bottom" />
        <IconButton icon={RefreshCw} label="Actualiser" onclick={refresh} tip="bottom" />
      </span>
      <input type="file" multiple hidden bind:this={uploadInput} onchange={onUpload} />
    </header>

    {#if error}
      <p class="alert err" role="alert"><Icon icon={TriangleAlert} size={16} /><span>{error}</span></p>
    {/if}

    <div class="tree">
      <div class="node root" class:here={folder === ''} style:--d={0}>
        <button class="hit" onclick={() => (folder = '')}>
          <Icon icon={HardDrive} size={16} class="ficon" />
          <span class="nm">workspace</span>
        </button>
      </div>
      {@render level('', 0)}
    </div>

    {#if picked.length}
      <div class="selbar" role="toolbar" aria-label="Éléments cochés">
        <span class="count">{picked.length} coché{picked.length > 1 ? 's' : ''}</span>
        <IconButton icon={Pencil} label="Renommer" onclick={rename} disabled={picked.length !== 1} />
        <IconButton icon={FolderInput} label="Déplacer" onclick={move} />
        <IconButton icon={FileArchive} label="Télécharger en zip" onclick={zip} />
        <IconButton icon={Trash} label="Supprimer" onclick={remove} class="del" />
        <IconButton icon={X} label="Tout décocher" onclick={() => (checked = {})} />
      </div>
    {/if}

    {#if disk}
      <footer class="disk" class:alarm={disk.alert}>
        <div class="disk-l">
          <span>Disque {dec1(disk.used_pct)} %</span>
          <span class="faint">{size(disk.free_bytes)} libres</span>
        </div>
        <ProgressBar value={disk.used_pct} size="sm" tone={disk.alert ? 'warn' : 'accent'} label="Disque de /data utilisé" />
        <div class="faint">Workspace {size(disk.workspace_bytes)}, sessions {size(disk.sessions_bytes)}</div>
        {#if disk.alert}
          <p class="alarm-t">Alerte à {dec1(disk.alert_pct)} % (réglable dans Paramètres, Sessions) : supprimez des sessions ou des fichiers.</p>
        {/if}
      </footer>
    {/if}
  </aside>

  <section class="main" aria-label="Fichier ouvert">
    {#if selected && doc}
      <header class="fbar">
        <IconButton icon={ArrowLeft} label="Retour à l’arbre" size="lg" class="back" onclick={() => (pane = 'tree')} />
        <div class="crumbs" title={selected}>
          <Icon icon={fileIcon(base(selected))} size={16} class="ficon" />
          {#each crumbs as c, i}
            {#if i < crumbs.length - 1}<span class="cr">{c}</span><span class="cr-sep" aria-hidden="true">/</span>
            {:else}<span class="cr last">{c}</span>{/if}
          {/each}
          {#if dirty}<span class="mod"><span class="dirty"></span>Modifié</span>{/if}
          {#if doc.readonly && (doc.kind === 'text' || doc.kind === 'markdown')}<span class="ro">lecture seule</span>{/if}
        </div>
        <div class="ftools">
          {#if doc.kind === 'markdown'}
            <Segmented
              label="Affichage du Markdown"
              value={mdSource ? 'src' : 'view'}
              options={[{ value: 'view', label: 'Aperçu' }, { value: 'src', label: doc.readonly ? 'Source' : 'Modifier' }]}
              onchange={(v) => (mdSource = v === 'src')}
            />
          {/if}
          {#if !picked.length}
            <span class="fileacts">
              <IconButton icon={Pencil} label="Renommer" onclick={rename} tip="bottom" />
              <IconButton icon={FolderInput} label="Déplacer" onclick={move} tip="bottom" />
              <IconButton icon={FileArchive} label="Télécharger en zip" onclick={zip} tip="bottom" />
              <IconButton icon={Trash} label="Supprimer" onclick={remove} tip="bottom" class="del" />
            </span>
          {/if}
          <IconButton icon={Download} label="Télécharger" href={wsDownloadUrl(selected)} tip="bottom" />
          {#if editable}
            <Button variant="primary" size="sm" icon={Save} onclick={() => save()} disabled={!dirty} busy={saving}>
              {saving ? 'Enregistrement…' : 'Enregistrer'}
            </Button>
          {/if}
        </div>
      </header>
      {#if conflict}
        <div class="alert warn" role="alert">
          <Icon icon={TriangleAlert} size={16} />
          <span class="grow">Le fichier a changé sur le disque depuis son ouverture (une session ?).</span>
          <Button size="sm" onclick={reload}>Recharger, perdre mes changements</Button>
          <Button size="sm" variant="danger" onclick={() => save(true)}>Écraser avec ma version</Button>
        </div>
      {/if}
      {#if doc.readonly && doc.truncated && (doc.kind === 'text' || doc.kind === 'markdown')}
        <p class="alert note">Trop gros pour l’éditeur (2 Mio au plus) : lecture seule, téléchargez-le.</p>
      {:else if doc.readonly && (doc.kind === 'text' || doc.kind === 'markdown')}
        <p class="alert note">Lecture seule : la mémoire se modifie depuis la page Mémoire.</p>
      {/if}
      <div class="body" class:padded={doc.kind !== 'text' && !(doc.kind === 'markdown' && mdSource)}>
        {#if doc.kind === 'image'}<div class="media"><img src={wsRawUrl(selected)} alt={selected} /></div>
        {:else if doc.kind === 'pdf'}<iframe src={wsRawUrl(selected)} title={selected}></iframe>
        {:else if doc.kind === 'video'}<div class="media"><!-- svelte-ignore a11y_media_has_caption --><video src={wsRawUrl(selected)} controls></video></div>
        {:else if doc.kind === 'audio'}<div class="media"><audio src={wsRawUrl(selected)} controls></audio></div>
        {:else if doc.kind === 'markdown' && !mdSource}<article class="md-doc"><Markdown text={doc.text ?? ''} /></article>
        {:else if doc.kind === 'text' || doc.kind === 'markdown'}
          {#key selected}
            <Editor bind:this={editor} path={selected} text={doc.text ?? ''} readonly={doc.readonly}
              onsave={() => save()} ondirty={() => (dirty = true)} />
          {/key}
        {:else if doc.kind === 'binary'}
          <EmptyState icon={fileIcon(base(selected))} title="Fichier binaire">
            {#snippet action()}<Button icon={Download} href={wsDownloadUrl(selected)}>Télécharger pour l’ouvrir</Button>{/snippet}
          </EmptyState>
        {:else if doc.kind === 'error'}<p class="alert err" role="alert"><Icon icon={TriangleAlert} size={16} /><span>{doc.text}</span></p>
        {:else}<p class="loading">Chargement…</p>{/if}
      </div>
    {:else}
      <div class="body padded">
        <EmptyState
          icon={Files}
          title="Ouvrez un fichier dans l’arbre"
          text="Tout le workspace : projets, topics (le code des sessions), skills, tâches. Cochez pour agir sur plusieurs éléments. Les dossiers marqués « géré » se créent et se suppriment depuis Sessions, Mémoire ou Skills."
        />
      </div>
    {/if}
  </section>
</div>


<style>
  /* Page bord à bord : la vue de la coquille perd sa marge tant que Fichiers est affiché. */

  .fx {
    flex: 1;
    min-height: 0;
    height: 100%;
    display: grid;
    grid-template-columns: 300px minmax(0, 1fr);
    background: var(--bg);
  }

  /* ---------- Arbre ---------- */
  .side { display: flex; flex-direction: column; min-height: 0; background: var(--panel); border-right: 1px solid var(--line); }
  .side-h { display: flex; align-items: center; gap: var(--s2); height: 48px; flex: none; padding: 0 var(--s2) 0 var(--s4); border-bottom: 1px solid var(--line); }
  h1 { flex: 1; margin: 0; font-size: var(--fs-lg); font-weight: 600; }
  .tools { display: flex; gap: 2px; }
  .tree { flex: 1; min-height: 0; overflow: auto; padding: var(--s2) 0; font-size: var(--fs-sm); }

  .node { position: relative; display: flex; align-items: center; height: 32px; padding-right: var(--s2); }
  .node:hover { background: var(--raised); }
  .node.sel { background: var(--accent-soft); box-shadow: inset 2px 0 0 var(--accent); }
  .node.picked { background: var(--raised); }
  .hit {
    flex: 1;
    min-width: 0;
    height: 100%;
    min-height: 0;
    display: flex;
    align-items: center;
    justify-content: flex-start;
    gap: 6px;
    padding: 0 0 0 calc(var(--s2) + var(--d) * 16px);
    border: 0;
    border-radius: 0;
    background: none;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-sm);
    font-weight: 400;
    text-align: left;
    filter: none;
  }
  .hit:disabled { color: var(--muted); opacity: 1; }
  .hit:focus-visible { outline-offset: -2px; }
  .root .hit { padding-left: var(--s4); color: var(--muted); }
  .chev { width: 16px; flex: none; display: grid; place-items: center; color: var(--faint); transition: transform 0.12s; }
  .chev.open { transform: rotate(90deg); }
  .node :global(.ficon) { color: var(--faint); }
  .node.here > .hit :global(.ficon) { color: var(--accent); }
  .node.here > .hit .nm { color: var(--text); font-weight: 600; }
  .node.sel :global(.ficon) { color: var(--accent); }
  .nm { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .tag { flex: none; font-size: var(--fs-xs); line-height: 20px; padding: 0 6px; border-radius: var(--r-sm); color: var(--faint); background: var(--raised); }
  .node:hover .tag, .node.picked .tag { background: var(--bg); }
  .dirty { flex: none; width: 8px; height: 8px; border-radius: 50%; background: var(--warn); }
  .ck { flex: none; display: grid; place-items: center; width: 28px; height: 28px; cursor: pointer; opacity: 0; }
  .node:hover .ck, .ck:focus-within, .node.picked .ck, .picking .ck { opacity: 1; }
  .more { margin: var(--s1) 0; padding-left: calc(var(--s4) + 22px + var(--d) * 16px); color: var(--faint); font-size: var(--fs-sm); }

  .selbar { display: flex; align-items: center; gap: 2px; flex: none; padding: var(--s1) var(--s2) var(--s1) var(--s4); border-top: 1px solid var(--line); background: var(--bg); }
  .count { flex: 1; font-size: var(--fs-sm); color: var(--text); font-variant-numeric: tabular-nums; }
  .fx :global(.del:hover:not(:disabled)) { color: var(--danger); background: var(--danger-soft); }

  .disk { flex: none; display: flex; flex-direction: column; gap: 6px; padding: var(--s3) var(--s4); border-top: 1px solid var(--line); font-size: var(--fs-sm); }
  .disk-l { display: flex; justify-content: space-between; gap: var(--s2); font-variant-numeric: tabular-nums; }
  .disk.alarm { background: var(--warn-soft); }
  .alarm-t { margin: 0; color: var(--warn); }

  /* ---------- Fichier ouvert ---------- */
  .main { display: flex; flex-direction: column; min-width: 0; min-height: 0; }
  .fbar { display: flex; align-items: center; gap: var(--s3); height: 48px; flex: none; padding: 0 var(--s3) 0 var(--s4); background: var(--panel); border-bottom: 1px solid var(--line); }
  .fbar :global(.back) { display: none; }
  .crumbs { flex: 1; min-width: 0; display: flex; align-items: center; gap: 6px; overflow: hidden; white-space: nowrap; font-size: var(--fs-sm); }
  .crumbs :global(.ficon) { color: var(--accent); }
  .cr { color: var(--muted); flex: 0 1 auto; min-width: 2ch; overflow: hidden; text-overflow: ellipsis; }
  .cr-sep { color: var(--faint); flex: none; }
  .cr.last { color: var(--text); font-weight: 600; flex: 0 0 auto; min-width: 0; max-width: 100%; }
  .mod { display: inline-flex; align-items: center; gap: 6px; flex: none; margin-left: var(--s2); color: var(--warn); }
  .ro { flex: none; margin-left: var(--s2); color: var(--faint); }
  .ftools { display: flex; align-items: center; gap: var(--s2); flex: none; }
  .fileacts { display: flex; gap: 2px; padding-right: var(--s2); border-right: 1px solid var(--line); }

  .alert { display: flex; align-items: center; gap: var(--s2); flex: none; margin: 0; padding: var(--s2) var(--s4); font-size: var(--fs-sm); border-bottom: 1px solid var(--line); flex-wrap: wrap; }
  .alert.warn { background: var(--warn-soft); color: var(--text); }
  .alert.warn > :global(.ui-icon) { color: var(--warn); }
  .alert.err { color: var(--danger); background: var(--danger-soft); white-space: pre-wrap; }
  .alert.note { color: var(--muted); background: var(--panel); }
  .grow { flex: 1; min-width: 20ch; }

  .body { flex: 1; min-height: 0; overflow: auto; display: flex; flex-direction: column; }
  .body.padded { padding: var(--s6); }
  .body > :global(.cm) { flex: 1; }
  .body > .alert.err { border: 1px solid var(--danger); border-radius: var(--r); }
  .md-doc { max-width: 760px; width: 100%; line-height: 1.6; }
  .md-doc :global(h1) { font-size: var(--fs-xl); }
  .md-doc :global(h2) { font-size: var(--fs-lg); margin-top: var(--s6); }
  .media { flex: 1; display: grid; place-items: center; min-height: 0; }
  .media img, .media video { max-width: 100%; max-height: 100%; object-fit: contain; border: 1px solid var(--line); background: var(--panel); }
  .body.padded:has(> iframe) { padding: 0; }
  iframe { flex: 1; width: 100%; border: 0; background: white; }
  .loading { color: var(--faint); font-size: var(--fs-sm); margin: 0; }

  @media (hover: none) {
    .ck { opacity: 1; }
  }

  /* ---------- Téléphone : arbre plein écran, puis fichier plein écran avec retour ---------- */
  @media (max-width: 767px) {
    .fx { grid-template-columns: minmax(0, 1fr); padding: 0 var(--safe-right) 0 var(--safe-left); }
    .fx:not(.doc) .main, .fx.doc .side { display: none; }
    .side { border-right: 0; }
    .side-h { height: 56px; }
    .side-h :global(.ib) { width: var(--touch); height: var(--touch); }
    .node { height: 44px; }
    .hit { font-size: var(--fs-md); padding-left: calc(var(--s3) + var(--d) * 16px); }
    .ck { width: var(--touch); height: var(--touch); opacity: 1; }
    .ck input { width: 20px; height: 20px; }
    .selbar :global(.ib) { width: var(--touch); height: var(--touch); }

    .fbar { height: auto; min-height: 56px; flex-wrap: wrap; padding: 0 var(--s2) 0 0; gap: var(--s1) var(--s2); }
    .fbar :global(.back) { display: inline-grid; }
    .crumbs { font-size: var(--fs-md); }
    .cr:not(.last), .cr-sep { display: none; }
    .ro { display: none; }
    .ftools { width: 100%; justify-content: flex-end; padding: 0 0 var(--s2) var(--s3); }
    .ftools :global(.ib) { width: var(--touch); height: var(--touch); }
    .fileacts { margin-right: auto; }
    .ftools :global(.btn) { height: var(--touch); }
    .body.padded { padding: var(--s4); }
  }
</style>
