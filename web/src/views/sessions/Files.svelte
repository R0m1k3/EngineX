<script lang="ts">
  import { octets } from '../../lib/ui/format';
  import { tree, fileText, fileUrl, downloadUrl, message, type TreeEntry } from '../../lib/api';
  import Markdown from './Markdown.svelte';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import FolderIcon from '@lucide/svelte/icons/folder';
  import FileIcon from '@lucide/svelte/icons/file';
  import Link2 from '@lucide/svelte/icons/link-2';
  import RefreshCw from '@lucide/svelte/icons/refresh-cw';
  import Download from '@lucide/svelte/icons/download';
  import FileText from '@lucide/svelte/icons/file-text';
  import Icon from '../../lib/ui/Icon.svelte';
  import IconButton from '../../lib/ui/IconButton.svelte';
  import Skeleton from '../../lib/ui/Skeleton.svelte';
  import EmptyState from '../../lib/ui/EmptyState.svelte';

  let { sessionId }: { sessionId: string } = $props();

  interface Dir {
    entries: TreeEntry[];
    truncated: boolean;
  }
  let dirs = $state<Record<string, Dir>>({});
  let opened = $state<Record<string, boolean>>({ '': true });
  let selected = $state('');
  let preview = $state<{ kind: string; text?: string; truncated?: boolean } | null>(null);
  let error = $state('');

  const join = (dir: string, name: string) => (dir ? `${dir}/${name}` : name);

  async function load(path: string) {
    try {
      const r = await tree(sessionId, path);
      dirs[path] = { entries: r.entries, truncated: r.truncated };
      error = '';
    } catch (e) {
      error = message(e);
    }
  }

  async function toggle(path: string) {
    opened[path] = !opened[path];
    if (opened[path] && !dirs[path]) await load(path);
  }

  async function refresh() {
    const open = Object.keys(opened).filter((p) => opened[p]);
    dirs = {};
    await Promise.all(open.map(load));
  }

  const ext = (p: string) => p.split('.').pop()?.toLowerCase() ?? '';
  const KIND: Record<string, string> = {
    png: 'image', jpg: 'image', jpeg: 'image', gif: 'image', webp: 'image', avif: 'image', svg: 'image',
    pdf: 'pdf', mp4: 'video', webm: 'video', mp3: 'audio', ogg: 'audio', wav: 'audio', md: 'markdown',
  };

  async function select(path: string) {
    selected = path;
    const kind = KIND[ext(path)] ?? 'text';
    if (kind !== 'text' && kind !== 'markdown') {
      preview = { kind };
      return;
    }
    preview = { kind: 'loading' };
    try {
      const r = await fileText(sessionId, path);
      preview = r.binary ? { kind: 'binary' } : { kind, text: r.text, truncated: r.truncated };
    } catch (e) {
      preview = { kind: 'error', text: message(e) };
    }
  }

  const size = octets;

  load('');
</script>

{#snippet level(path: string, depth: number)}
  {#each dirs[path]?.entries ?? [] as e (e.name)}
    {@const p = join(path, e.name)}
    {#if e.kind === 'dir'}
      <button class="entry" style:padding-left="{depth * 16 + 8}px" aria-expanded={!!opened[p]} onclick={() => toggle(p)}>
        <Icon icon={ChevronRight} size={14} class="chev" /><Icon icon={FolderIcon} size={16} class="fi" /><span class="ell">{e.name}</span>
      </button>
      {#if opened[p]}{@render level(p, depth + 1)}{/if}
    {:else}
      <button class="entry" class:sel={selected === p} style:padding-left="{depth * 16 + 22}px"
        onclick={() => e.kind === 'file' && select(p)} disabled={e.kind !== 'file'} title={e.kind === 'link' ? 'Lien symbolique' : size(e.size)}>
        <Icon icon={e.kind === 'link' ? Link2 : FileIcon} size={16} class="fi" /><span class="ell">{e.name}</span>
      </button>
    {/if}
  {/each}
  {#if dirs[path]?.truncated}<p class="more" style:padding-left="{depth * 16 + 22}px">1000 entrées au plus.</p>{/if}
{/snippet}

<div class="files">
  <div class="list">
    <div class="tb">
      <span class="grow">Dossier de travail</span>
      <IconButton icon={RefreshCw} label="Actualiser" size="sm" onclick={refresh} />
    </div>
    <div class="entries">
      {#if error}<p class="error small pad">{error}</p>{/if}
      {#if !dirs[''] && !error}<div class="pad"><Skeleton lines={5} /></div>{/if}
      {#if dirs['']?.entries.length === 0}<p class="faint small pad">Dossier vide : il se remplit quand Codex ou vous y créez des fichiers.</p>{/if}
      {@render level('', 0)}
    </div>
  </div>
  <div class="preview">
    {#if selected}
      <div class="tb">
        <code class="grow name" title={selected}><span>{selected}</span></code>
        <IconButton icon={Download} label="Télécharger" size="sm" href={downloadUrl(sessionId, selected)} />
      </div>
      <div class="pv">
        {#if preview?.kind === 'image'}<img src={fileUrl(sessionId, selected)} alt={selected} />
        {:else if preview?.kind === 'pdf'}<iframe src={fileUrl(sessionId, selected)} title={selected}></iframe>
        {:else if preview?.kind === 'video'}<!-- svelte-ignore a11y_media_has_caption --><video src={fileUrl(sessionId, selected)} controls></video>
        {:else if preview?.kind === 'audio'}<audio src={fileUrl(sessionId, selected)} controls></audio>
        {:else if preview?.kind === 'markdown'}<div class="md"><Markdown text={preview.text ?? ''} /></div>
        {:else if preview?.kind === 'text'}<pre>{preview.text}</pre>
        {:else if preview?.kind === 'binary'}<p class="muted small">Fichier binaire : téléchargez-le pour l’ouvrir.</p>
        {:else if preview?.kind === 'error'}<p class="error small">{preview.text}</p>
        {:else}<Skeleton lines={6} />{/if}
        {#if preview?.truncated}<p class="faint small">Aperçu limité à 512 Kio.</p>{/if}
      </div>
    {:else}
      <div class="pv">
        <EmptyState icon={FileText} title="Aucun fichier choisi" text="Choisissez un fichier pour le lire ici. Lecture seule ; les secrets du coffre sont masqués dans l’aperçu texte." />
      </div>
    {/if}
  </div>
</div>

<style>
  .files { display: grid; grid-template-rows: minmax(140px, 42%) minmax(0, 1fr); height: 100%; min-height: 0; }
  .list, .preview { display: flex; flex-direction: column; min-height: 0; }
  .preview { border-top: 1px solid var(--line); }
  .tb { display: flex; align-items: center; gap: var(--s2); min-height: 36px; padding: 0 var(--s1) 0 var(--s3); border-bottom: 1px solid var(--line); font-size: var(--fs-sm); color: var(--muted); flex: none; }
  .entries { flex: 1; min-height: 0; overflow: auto; padding: var(--s1) 0; }
  .pad { margin: 0; padding: var(--s2) var(--s3); }
  .entry {
    display: flex;
    align-items: center;
    justify-content: flex-start;
    gap: 6px;
    width: 100%;
    height: 28px;
    min-height: 0;
    padding-right: var(--s3);
    border: 0;
    border-left: 2px solid transparent;
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
  .entry:hover:not(:disabled) { background: var(--raised); }
  .entry:disabled { color: var(--muted); opacity: 1; cursor: default; }
  .entry.sel { background: var(--accent-soft); border-left-color: var(--accent); }
  .entry :global(.chev) { color: var(--faint); transition: transform 0.12s; flex: none; }
  .entry[aria-expanded='true'] :global(.chev) { transform: rotate(90deg); }
  .entry :global(.fi) { color: var(--faint); flex: none; }
  .more { margin: 0; font-size: var(--fs-xs); color: var(--faint); }
  .name { overflow: hidden; min-width: 0; color: var(--text); }
  .name span { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; direction: rtl; text-align: left; }
  .pv { flex: 1; min-height: 0; overflow: auto; padding: var(--s3); display: flex; flex-direction: column; gap: var(--s2); }
  .pv img, .pv video { max-width: 100%; align-self: flex-start; border: 1px solid var(--line); border-radius: var(--r); }
  iframe { width: 100%; flex: 1; min-height: 60vh; border: 1px solid var(--line); border-radius: var(--r); background: white; }
  pre { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; background: var(--bg); border: 1px solid var(--line); border-radius: var(--r); padding: var(--s3); }
  .pv p { margin: 0; }
  @media (max-width: 767px) {
    .entry { height: var(--touch); font-size: var(--fs-md); }
  }
</style>
