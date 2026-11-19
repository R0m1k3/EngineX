<script lang="ts">
  import {
    gitComment,
    gitCreatePull,
    gitMergePull,
    gitPullDetail,
    gitPulls,
    message,
    type GitBranch,
    type PullDetail,
    type PullRequest,
  } from '../../lib/api';
  import Markdown from '../sessions/Markdown.svelte';
  import Diff from './Diff.svelte';

  let { repoId, forge, branches }: { repoId: number; forge: 'github' | 'gitea'; branches: GitBranch[] } = $props();

  let filter = $state<'open' | 'closed' | 'all'>('open');
  let list = $state<PullRequest[] | null>(null);
  let detail = $state<PullDetail | null>(null);
  let error = $state('');
  let busy = $state(false);
  let reply = $state('');
  let method = $state<'merge' | 'squash' | 'rebase'>('merge');
  let creating = $state(false);
  let form = $state({ title: '', body: '', head: '', base: '' });

  async function load() {
    error = '';
    try {
      list = await gitPulls(repoId, filter);
    } catch (e) {
      list = [];
      error = message(e);
    }
  }

  async function open(n: number) {
    error = '';
    detail = null;
    try {
      detail = await gitPullDetail(repoId, n);
    } catch (e) {
      error = message(e);
    }
  }

  async function act(f: () => Promise<unknown>) {
    busy = true;
    error = '';
    try {
      await f();
    } catch (e) {
      error = message(e);
    } finally {
      busy = false;
    }
  }

  const comment = () =>
    act(async () => {
      await gitComment(repoId, detail!.number, reply);
      reply = '';
      await open(detail!.number);
    });

  const merge = () => {
    if (!detail || !confirm(`Fusionner #${detail.number} (${method}) dans ${detail.base} ?`)) return;
    act(async () => {
      await gitMergePull(repoId, detail!.number, method);
      await open(detail!.number);
      await load();
    });
  };

  const create = () =>
    act(async () => {
      const r = await gitCreatePull(repoId, form.title, form.body, form.head, form.base.trim() || undefined);
      creating = false;
      form = { title: '', body: '', head: '', base: '' };
      await load();
      await open(r.number);
    });

  const when = (s: string) => (s ? new Date(s).toLocaleString('fr-FR', { dateStyle: 'short', timeStyle: 'short' }) : '');
  const STATE: Record<string, string> = { open: 'ouverte', closed: 'fermée', merged: 'fusionnée' };

  $effect(() => {
    void filter;
    void repoId;
    load();
  });
</script>

<div class="pulls">
  <div class="list">
    <div class="row small">
      <select bind:value={filter} aria-label="Filtre">
        <option value="open">Ouvertes</option>
        <option value="closed">Fermées</option>
        <option value="all">Toutes</option>
      </select>
      <button class="secondary" onclick={() => (creating = !creating)}>Nouvelle</button>
    </div>
    {#if creating}
      <form class="new" onsubmit={(e) => { e.preventDefault(); create(); }}>
        <label>Branche (déjà poussée)
          <select bind:value={form.head} required>
            <option value="" disabled>choisir</option>
            {#each branches.filter((b) => b.upstream) as b}<option value={b.name}>{b.name}</option>{/each}
          </select>
        </label>
        <label>Vers (vide : branche par défaut) <input bind:value={form.base} placeholder="main" /></label>
        <label>Titre <input bind:value={form.title} required /></label>
        <label>Description <textarea rows="3" bind:value={form.body}></textarea></label>
        <button type="submit" disabled={busy || !form.head || !form.title.trim()}>Créer</button>
      </form>
    {/if}
    {#if list === null}<p class="muted small">Chargement…</p>
    {:else if !list.length}<p class="muted small">Aucune pull request.</p>{/if}
    {#each list ?? [] as p (p.number)}
      <button class="pr" class:sel={detail?.number === p.number} onclick={() => open(p.number)}>
        <span>#{p.number} {p.title}</span>
        <span class="muted tiny">{STATE[p.state] ?? p.state}{p.draft ? ' · brouillon' : ''} · {p.head} → {p.base} · {p.author}</span>
      </button>
    {/each}
  </div>
  <div class="detail">
    {#if error}<p class="error small pre">{error}</p>{/if}
    {#if detail}
      <div class="row">
        <h3>#{detail.number} {detail.title}</h3>
        <a href={detail.url} target="_blank" rel="noopener noreferrer">Ouvrir sur {forge === 'github' ? 'GitHub' : 'Gitea'}</a>
      </div>
      <p class="muted small">
        {STATE[detail.state] ?? detail.state} · <code>{detail.head}</code> → <code>{detail.base}</code> · {detail.author} · {when(detail.created_at)}
      </p>
      {#if detail.body}<div class="md card-in"><Markdown text={detail.body} /></div>{/if}
      {#if detail.state === 'open'}
        <div class="merge">
          <select bind:value={method} aria-label="Méthode de fusion">
            <option value="merge">Commit de fusion</option>
            <option value="squash">Squash</option>
            <option value="rebase">Rebase</option>
          </select>
          <button onclick={merge} disabled={busy || detail.mergeable === false}>Fusionner</button>
          {#if detail.mergeable === false}<span class="error small">Conflits : à résoudre d’abord.</span>{/if}
        </div>
      {/if}
      <h3>Modifications</h3>
      <Diff diff={detail.diff} truncated={detail.diff_truncated} />
      <h3>Commentaires ({detail.comments.length})</h3>
      {#each detail.comments as c}
        <div class="comment card-in">
          <p class="muted tiny">{c.author} · {when(c.created_at)}</p>
          <Markdown text={c.body ?? ''} />
        </div>
      {/each}
      <textarea rows="3" bind:value={reply} placeholder="Votre commentaire"></textarea>
      <button onclick={comment} disabled={busy || !reply.trim()}>Commenter</button>
    {:else}
      <p class="muted small">Choisissez une pull request.</p>
    {/if}
  </div>
</div>

<style>
  .pulls { display: grid; grid-template-columns: minmax(240px, 340px) 1fr; gap: var(--s3); min-height: 0; height: 100%; }
  .list, .detail { overflow: auto; min-height: 0; display: flex; flex-direction: column; gap: var(--s2); }
  .pr { all: unset; display: flex; flex-direction: column; cursor: pointer; padding: var(--s2); border: 1px solid var(--line); border-radius: var(--radius); font-size: var(--fs-sm); }
  .pr.sel, .pr:hover { border-color: var(--accent); }
  .new { display: flex; flex-direction: column; gap: var(--s2); border: 1px solid var(--line); border-radius: var(--radius); padding: var(--s2); }
  .merge { display: flex; gap: var(--s2); align-items: center; }
  .card-in { border: 1px solid var(--line); border-radius: var(--radius); padding: var(--s2) var(--s3); }
  h3 { margin: var(--s2) 0 0; }
  textarea { width: 100%; background: var(--bg); color: var(--text); border: 1px solid var(--line); border-radius: var(--radius); padding: var(--s2); font: inherit; }
  .detail > button { align-self: flex-start; }
  .tiny { font-size: var(--fs-sm); margin: 0; }
  .pre { white-space: pre-wrap; }
  a { color: var(--accent); }
</style>
