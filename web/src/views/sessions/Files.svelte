<script lang="ts">
  import { tree, fileText, fileUrl, downloadUrl, message, type TreeEntry } from '../../lib/api';
  import Markdown from './Markdown.svelte';

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

  const size = (n: number) => (n < 1024 ? `${n} o` : n < 1 << 20 ? `${(n / 1024).toFixed(0)} Kio` : `${(n / (1 << 20)).toFixed(1)} Mio`);

  load('');
</script>

{#snippet level(path: string, depth: number)}
  {#each dirs[path]?.entries ?? [] as e (e.name)}
    {@const p = join(path, e.name)}
    {#if e.kind === 'dir'}
      <button class="link entry" style:padding-left="{depth * 14}px" onclick={() => toggle(p)}>
        {opened[p] ? '▾' : '▸'} {e.name}/
      </button>
      {#if opened[p]}{@render level(p, depth + 1)}{/if}
    {:else}
      <button class="link entry" class:sel={selected === p} style:padding-left="{depth * 14 + 12}px"
        onclick={() => e.kind === 'file' && select(p)} disabled={e.kind !== 'file'} title={e.kind === 'link' ? 'Lien symbolique' : size(e.size)}>
        {e.name}{e.kind === 'link' ? ' →' : ''}
      </button>
    {/if}
  {/each}
  {#if dirs[path]?.truncated}<p class="muted tiny" style:padding-left="{depth * 14}px">1000 entrées au plus.</p>{/if}
{/snippet}

<div class="files">
  <div class="list">
    <div class="row small"><span class="muted">Dossier de travail</span><button class="link" onclick={refresh}>Actualiser</button></div>
    {#if error}<p class="error small">{error}</p>{/if}
    {@render level('', 0)}
  </div>
  <div class="preview">
    {#if selected}
      <div class="row small">
        <code class="name">{selected}</code>
        <a href={downloadUrl(sessionId, selected)} download>Télécharger</a>
      </div>
      {#if preview?.kind === 'image'}<img src={fileUrl(sessionId, selected)} alt={selected} />
      {:else if preview?.kind === 'pdf'}<iframe src={fileUrl(sessionId, selected)} title={selected}></iframe>
      {:else if preview?.kind === 'video'}<!-- svelte-ignore a11y_media_has_caption --><video src={fileUrl(sessionId, selected)} controls></video>
      {:else if preview?.kind === 'audio'}<audio src={fileUrl(sessionId, selected)} controls></audio>
      {:else if preview?.kind === 'markdown'}<div class="md"><Markdown text={preview.text ?? ''} /></div>
      {:else if preview?.kind === 'text'}<pre>{preview.text}</pre>
      {:else if preview?.kind === 'binary'}<p class="muted small">Fichier binaire : téléchargez-le pour l’ouvrir.</p>
      {:else if preview?.kind === 'error'}<p class="error small">{preview.text}</p>
      {:else}<p class="muted small">Chargement…</p>{/if}
      {#if preview?.truncated}<p class="muted tiny">Aperçu limité à 512 Kio.</p>{/if}
    {:else}
      <p class="muted small">Choisissez un fichier. Lecture seule ; les secrets du coffre sont masqués dans l’aperçu texte.</p>
    {/if}
  </div>
</div>

<style>
  .files { display: grid; grid-template-rows: minmax(120px, 40%) 1fr; gap: var(--s3); height: 100%; min-height: 0; }
  .list, .preview { overflow: auto; min-height: 0; }
  .entry { display: block; width: 100%; padding-top: 1px; padding-bottom: 1px; font-size: var(--fs-sm); color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .entry:disabled { color: var(--muted); opacity: 1; }
  .entry.sel { color: var(--accent); }
  .name { overflow: hidden; text-overflow: ellipsis; }
  .preview img, .preview video { max-width: 100%; }
  iframe { width: 100%; height: 70vh; border: 1px solid var(--line); border-radius: var(--radius); background: white; }
  pre { font-size: var(--fs-sm); white-space: pre-wrap; background: var(--bg); border: 1px solid var(--line); border-radius: var(--radius); padding: var(--s2); }
  .tiny { font-size: var(--fs-sm); margin: 0; }
  a { color: var(--accent); }
</style>
