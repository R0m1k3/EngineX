<script lang="ts">
  import History from '@lucide/svelte/icons/history';
  import Save from '@lucide/svelte/icons/save';
  import Globe from '@lucide/svelte/icons/globe';
  import Folder from '@lucide/svelte/icons/folder';
  import Hash from '@lucide/svelte/icons/hash';
  import Eye from '@lucide/svelte/icons/eye';
  import Button from '../lib/ui/Button.svelte';
  import Select from '../lib/ui/Select.svelte';
  import Icon from '../lib/ui/Icon.svelte';
  import { confirmDialog } from '../lib/ui/dialog.svelte';
  import { toast } from '../lib/ui/toast.svelte';
  import Page from './ops/Page.svelte';
  import Note from './settings/Note.svelte';
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

  /** Renvoie false si l'utilisateur garde ses modifications. */
  async function open(p: string, f: MemFile): Promise<boolean> {
    if (dirty && !(await confirmDialog('Modifications non enregistrées : les abandonner ?', { title: 'Abandonner les modifications', confirm: 'Abandonner', danger: true }))) return false;
    path = p;
    file = f;
    error = '';
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
    return true;
  }

  async function save(text = content) {
    error = '';
    try {
      const r = await saveMemoryFile(path, file, text, base);
      base = r.sha;
      content = text;
      loaded = text;
      toast('Enregistré.', 'ok');
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
    const ok = await confirmDialog('Remplacer le contenu actuel par cette version ? La version actuelle reste dans l’historique.', {
      title: 'Restaurer cette version',
      confirm: 'Restaurer',
    });
    if (!ok) return;
    await save(viewing.content);
  }

  // Téléphone : niveaux dans une liste déroulante.
  async function pickLevel(e: Event & { currentTarget: HTMLSelectElement }) {
    const el = e.currentTarget;
    if (!(await open(el.value, file))) el.value = path;
  }

  loadTree().then(() => open('', 'memory'));
</script>

{#snippet lvl(p: string, name: string, sz: number, icon: typeof Globe, sub: boolean, warn: boolean)}
  <button type="button" class="lvl" class:sub class:on={path === p} class:warn aria-current={path === p ? 'true' : undefined} onclick={() => open(p, file)}>
    <Icon {icon} size={16} />
    <span class="ell">{name}</span>
    <span class="sz num">{kio(sz)}</span>
  </button>
{/snippet}

<Page title="Mémoire">
  {#snippet meta()}<span class="pmeta">Notes données à Codex au début de chaque session : globales, du projet, puis du topic.</span>{/snippet}

  <div class="mem">
    <nav class="levels" aria-label="Niveaux de mémoire">
      {#if tree}
        {@render lvl('', 'Global', tree.global.memory.size, Globe, false, false)}
        {#each tree.projects as p (p.slug)}
          {@render lvl(p.slug ?? "", p.name ?? p.slug ?? "", p.memory.size, Folder, false, false)}
          {#each p.topics as t (t.slug)}
            {@render lvl(`${p.slug}/${t.slug}`, t.name ?? t.slug ?? "", t.memory.size, Hash, true, t.memory.size > tree.warn)}
          {/each}
        {/each}
      {/if}
    </nav>

    <section class="ed" aria-label="Éditeur de mémoire">
      {#if tree}
        <div class="lvl-select">
          <Select label="Niveau" size="lg" value={path} onchange={pickLevel}>
            <option value="">Global ({kio(tree.global.memory.size)})</option>
            {#each tree.projects as p (p.slug)}
              <option value={p.slug}>{p.name} ({kio(p.memory.size)})</option>
              {#each p.topics as t (t.slug)}
                <option value="{p.slug}/{t.slug}">{p.name} / {t.name} ({kio(t.memory.size)})</option>
              {/each}
            {/each}
          </Select>
        </div>
      {/if}

      <div class="head">
        <h2 class="ell">{label}</h2>
        <div class="ftabs" role="tablist" aria-label="Fichier">
          <button type="button" role="tab" class:on={file === 'instructions'} aria-selected={file === 'instructions'} onclick={() => open(path, 'instructions')}>Instructions</button>
          <button type="button" role="tab" class:on={file === 'memory'} aria-selected={file === 'memory'} onclick={() => open(path, 'memory')}>Mémoire</button>
        </div>
      </div>
      <p class="explain">
        {#if file === 'instructions'}Vos consignes : Codex les lit sans pouvoir les modifier.
        {:else}Tenue par Codex pendant les sessions ; vous pouvez la corriger ici.{/if}
      </p>

      {#if error}
        <Note tone="danger">
          {error}
          {#snippet action()}{#if error.startsWith('Modifié entre-temps')}<Button size="sm" onclick={reload}>Recharger</Button>{/if}{/snippet}
        </Note>
      {/if}

      <textarea bind:value={content} spellcheck="false" aria-label="{file === 'memory' ? 'Mémoire' : 'Instructions'} : {label}"
        placeholder={file === 'memory' ? '(vide) Une ligne par fait : - AAAA-MM-JJ : …' : '(vide) Vos consignes pour Codex à ce niveau.'}></textarea>

      <div class="foot">
        <span class="count num" class:warn={size > limit * 0.75} class:over={size > limit}>
          <span class="meter" aria-hidden="true"><i style:width="{Math.min(100, (size / limit) * 100)}%"></i></span>
          {kio(size)} / {limit / 1024} Kio
        </span>
        {#if current?.[file].updated}
          <span class="by">
            Dernière écriture : {date(current[file].updated)}, par
            {#if sessionOf(current[file].author)}<a href="#sessions/{sessionOf(current[file].author)}">{who(current[file].author)}</a>
            {:else}{who(current[file].author)}{/if}
          </span>
        {/if}
        <div class="spacer"></div>
        <Button variant="ghost" icon={History} aria-pressed={showHistory} onclick={toggleHistory}>{showHistory ? 'Masquer l’historique' : 'Historique'}</Button>
        <Button variant="primary" icon={Save} onclick={() => save()} disabled={!dirty || size > limit}>Enregistrer</Button>
      </div>

      {#if showHistory}
        <section class="hist" aria-label="Historique">
          <h3>Historique <span class="faint">{revisions.length} version{revisions.length > 1 ? 's' : ''}, 20 gardées</span></h3>
          {#if !revisions.length}<p class="faint">Aucune version enregistrée.</p>{/if}
          <ul>
            {#each revisions as r (r.id)}
              <li class:on={viewing?.id === r.id}>
                <span class="num">{date(r.created_at)}</span>
                <span class="who">{#if sessionOf(r.author)}<a href="#sessions/{sessionOf(r.author)}">{who(r.author)}</a>{:else}{who(r.author)}{/if}</span>
                <span class="num faint">{kio(r.size)}</span>
                <Button size="sm" variant="ghost" icon={Eye} onclick={() => view(r.id)}>Voir</Button>
              </li>
            {/each}
          </ul>
          {#if viewing}
            <pre class="out">{viewing.content || '(vide)'}</pre>
            <div class="vacts">
              <Button variant="primary" onclick={restore}>Restaurer cette version</Button>
              <Button variant="ghost" onclick={() => (viewing = null)}>Fermer</Button>
            </div>
          {/if}
        </section>
      {/if}
    </section>
  </div>
</Page>

<style>
  .pmeta { white-space: normal; }
  .mem { flex: 1; display: grid; grid-template-columns: 260px minmax(0, 1fr); min-height: calc(100dvh - var(--status-h) - 49px); }
  .levels { background: var(--panel); border-right: 1px solid var(--line); padding: var(--s2) 0; display: flex; flex-direction: column; }
  .lvl {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--s2);
    width: 100%;
    height: 32px;
    min-height: 0;
    padding: 0 var(--s3) 0 var(--s4);
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
  .lvl :global(.ui-icon) { color: var(--faint); }
  .lvl.sub { padding-left: calc(var(--s4) + 20px); }
  @media (hover: hover) { .lvl:hover { background: var(--raised); } }
  .lvl.on { background: var(--accent-soft); }
  .lvl.on::before { content: ''; position: absolute; left: 0; top: 0; bottom: 0; width: 2px; background: var(--accent); }
  .lvl.on :global(.ui-icon) { color: var(--accent); }
  .lvl .ell { flex: 1; }
  .sz { color: var(--faint); font-size: var(--fs-xs); }
  .lvl.warn .sz { color: var(--warn); }

  .ed { display: flex; flex-direction: column; gap: var(--s3); padding: var(--s5) var(--s6); min-width: 0; max-width: 1000px; }
  .lvl-select { display: none; }
  .head { display: flex; align-items: flex-end; justify-content: space-between; gap: var(--s4); border-bottom: 1px solid var(--line); }
  .head h2 { margin: 0 0 var(--s2); font-size: var(--fs-lg); font-weight: 600; }
  .ftabs { display: flex; }
  .ftabs button {
    height: 40px;
    min-height: 0;
    padding: 0 var(--s3);
    margin-bottom: -1px;
    border: 0;
    border-bottom: 2px solid transparent;
    border-radius: 0;
    background: none;
    color: var(--muted);
    font: inherit;
    font-size: var(--fs-sm);
    font-weight: 400;
    filter: none;
    cursor: pointer;
  }
  .ftabs button:hover { color: var(--text); }
  .ftabs button.on { color: var(--text); border-bottom-color: var(--accent); }
  .explain { margin: 0; color: var(--muted); font-size: var(--fs-sm); }
  textarea {
    width: 100%;
    box-sizing: border-box;
    min-height: max(320px, calc(100dvh - 360px));
    padding: var(--s3) var(--s4);
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--line-strong);
    border-radius: var(--r);
    font-family: var(--mono);
    font-size: var(--fs-code);
    line-height: 1.6;
    resize: vertical;
  }
  textarea:focus-visible { outline: 1px solid var(--accent); border-color: var(--accent); outline-offset: 0; }
  .foot { display: flex; align-items: center; flex-wrap: wrap; gap: var(--s2) var(--s4); font-size: var(--fs-sm); }
  .count { display: inline-flex; align-items: center; gap: var(--s2); color: var(--muted); }
  .meter { width: 64px; height: 6px; border-radius: 3px; background: var(--raised); overflow: hidden; }
  .meter i { display: block; height: 100%; background: var(--accent-fill); }
  .count.warn { color: var(--warn); }
  .count.warn .meter i { background: var(--warn); }
  .count.over { color: var(--danger); }
  .count.over .meter i { background: var(--danger); }
  .by { color: var(--muted); }
  .spacer { flex: 1; }
  .hist { border-top: 1px solid var(--line); padding-top: var(--s4); display: flex; flex-direction: column; gap: var(--s2); }
  .hist h3 { margin: 0; font-size: var(--fs-md); font-weight: 600; display: flex; gap: var(--s3); align-items: baseline; }
  .faint { color: var(--faint); font-size: var(--fs-sm); font-weight: 400; margin: 0; }
  .hist ul { list-style: none; margin: 0; padding: 0; }
  .hist li { display: grid; grid-template-columns: 180px minmax(0, 1fr) 80px auto; align-items: center; gap: var(--s3); padding: var(--s1) var(--s2); border-bottom: 1px solid var(--line); font-size: var(--fs-sm); }
  .hist li.on { background: var(--accent-soft); }
  .out {
    margin: 0;
    padding: var(--s3) var(--s4);
    max-height: 360px;
    overflow: auto;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: var(--r);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    line-height: 1.6;
  }
  .vacts { display: flex; gap: var(--s2); }

  @media (max-width: 767px) {
    .mem { display: block; }
    .levels { display: none; }
    .ed { padding: var(--s4); }
    .lvl-select { display: block; }
    .head { flex-direction: column; align-items: stretch; gap: var(--s2); }
    .ftabs button { flex: 1; height: var(--touch); font-size: var(--fs-md); }
    textarea { min-height: 50dvh; }
    .foot :global(.btn) { height: var(--touch); }
    .hist li { grid-template-columns: minmax(0, 1fr) auto; }
    .hist li .who, .hist li > .faint { grid-column: 1; }
    .hist li :global(.btn) { grid-column: 2; grid-row: 1 / span 3; height: var(--touch); }
  }
</style>
