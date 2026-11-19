<script lang="ts">
  import {
    gitBranch,
    gitCheckout,
    gitCommit,
    gitFetch,
    gitMerge,
    gitPull,
    gitPush,
    gitRepo,
    gitRepos,
    message,
    type GitCommitDetail,
    type GitRepo,
    type GitRepoDetail,
  } from '../lib/api';
  import Graph from './git/Graph.svelte';
  import Diff from './git/Diff.svelte';
  import Pulls from './git/Pulls.svelte';

  // #git/<topic>/pulls
  let { sub }: { sub: string } = $props();

  let repos = $state<GitRepo[] | null>(null);
  let repo = $state<GitRepoDetail | null>(null);
  let commit = $state<GitCommitDetail | null>(null);
  let selected = $state('');
  let error = $state('');
  let notice = $state('');
  let busy = $state('');
  let limit = $state(300);

  let parts = $derived(sub.split('/'));
  let id = $derived(Number(parts[0]) || repos?.[0]?.topic_id || 0);
  let tab = $derived(parts[1] === 'pulls' ? 'pulls' : 'history');
  let current = $derived(repos?.find((r) => r.topic_id === id));

  async function loadRepos() {
    try {
      repos = await gitRepos();
    } catch (e) {
      repos = [];
      error = message(e);
    }
  }

  async function loadRepo() {
    if (!id) return;
    try {
      repo = await gitRepo(id, limit);
      error = '';
    } catch (e) {
      repo = null;
      error = message(e);
    }
  }

  async function show(sha: string) {
    selected = sha;
    commit = null;
    try {
      commit = await gitCommit(id, sha);
    } catch (e) {
      error = message(e);
    }
  }

  async function act(label: string, f: () => Promise<unknown>, done: string) {
    busy = label;
    error = '';
    notice = '';
    try {
      await f();
      notice = done;
    } catch (e) {
      error = message(e);
    } finally {
      busy = '';
      await loadRepo();
    }
  }

  function newBranch() {
    const name = prompt(selected ? `Nouvelle branche sur ${selected.slice(0, 7)}` : 'Nouvelle branche sur HEAD');
    if (!name?.trim()) return;
    act('branch', () => gitBranch(id, name.trim(), selected || undefined), `Branche ${name.trim()} créée.`);
  }

  function merge(b: string) {
    if (!repo?.head || !confirm(`Fusionner ${b} dans ${repo.head} ?`)) return;
    act('merge', () => gitMerge(id, b), `${b} fusionnée dans ${repo!.head}.`);
  }

  function go(topic: number, t: 'history' | 'pulls') {
    location.hash = t === 'pulls' ? `git/${topic}/pulls` : `git/${topic}`;
  }

  const when = (t: number) => new Date(t * 1000).toLocaleString('fr-FR', { dateStyle: 'medium', timeStyle: 'short' });

  loadRepos();
  $effect(() => {
    void id;
    void limit;
    commit = null;
    selected = '';
    loadRepo();
  });
</script>

<section class="page">
  <div class="row head">
    <h1>Git</h1>
    {#if repos?.length}
      <select value={id} onchange={(e) => go(Number((e.target as HTMLSelectElement).value), tab as 'history' | 'pulls')} aria-label="Dépôt">
        {#each repos as r}<option value={r.topic_id}>{r.project} / {r.topic}</option>{/each}
      </select>
      <div class="tabs">
        <a href="#git/{id}" class:active={tab === 'history'}>Historique</a>
        {#if repo?.forge}<a href="#git/{id}/pulls" class:active={tab === 'pulls'}>Pull requests</a>{/if}
      </div>
    {/if}
    <span class="grow"></span>
    {#if repo && tab === 'history'}
      <button class="secondary" onclick={() => act('fetch', () => gitFetch(id), 'Fetch fait.')} disabled={!!busy || !repo.origin}>
        {busy === 'fetch' ? 'Fetch…' : 'Fetch'}
      </button>
      <button class="secondary" onclick={() => act('pull', () => gitPull(id), 'À jour.')} disabled={!!busy || !repo.origin}>
        {busy === 'pull' ? 'Pull…' : 'Pull'}
      </button>
      <button class="secondary" onclick={() => act('push', () => gitPush(id), 'Poussé.')} disabled={!!busy || !repo.origin || repo.detached}>
        {busy === 'push' ? 'Push…' : 'Push'}
      </button>
    {/if}
  </div>
  {#if error}<p class="error small pre">{error}</p>{/if}
  {#if notice}<p class="muted small">{notice}</p>{/if}

  {#if repos && !repos.length}
    <p class="muted">
      Aucun topic n’est un dépôt git. Clonez un dépôt dans un topic (terminal d’une session, ou demandez-le à Codex) :
      il apparaîtra ici.
    </p>
  {:else if repo && tab === 'pulls' && repo.forge}
    <div class="fill"><Pulls repoId={id} forge={repo.forge} branches={repo.branches} /></div>
  {:else if repo}
    <p class="muted small">
      <code>{current?.path}</code> · {repo.origin ? `origin : ${repo.origin}` : 'pas de remote origin'}
      {#if repo.origin && !repo.forge} · forge inconnue (Gitea : GITEA_URL dans Clés){/if}
    </p>
    <div class="split">
      <div class="card side">
        <div class="row small"><strong>Branches</strong><button class="link" onclick={newBranch}>Nouvelle</button></div>
        {#each repo.branches as b (b.name)}
          <div class="branch" class:cur={b.current}>
            <div class="row small">
              <code>{b.current ? '● ' : ''}{b.name}</code>
              <span class="muted">
                {#if b.ahead}↑{b.ahead}{/if}{#if b.behind} ↓{b.behind}{/if}
                {#if !b.upstream}<span title="Jamais poussée">local</span>{:else if b.upstream_gone}<span title="Supprimée en amont">amont disparu</span>{/if}
              </span>
            </div>
            {#if !b.current}
              <div class="acts small">
                <button class="link" onclick={() => act('checkout', () => gitCheckout(id, b.name), `Sur ${b.name}.`)} disabled={!!busy}>Basculer</button>
                {#if repo.head}<button class="link" onclick={() => merge(b.name)} disabled={!!busy}>Fusionner dans {repo.head}</button>{/if}
                {#if repo.origin}<button class="link" onclick={() => act('push', () => gitPush(id, b.name), `${b.name} poussée.`)} disabled={!!busy}>Pousser</button>{/if}
              </div>
            {/if}
          </div>
        {/each}
        {#if repo.remotes.length}
          <p class="small"><strong>Distantes</strong></p>
          {#each repo.remotes as r}
            <div class="row small">
              <code class="muted">{r.name}</code>
              {#if !repo.branches.some((b) => `origin/${b.name}` === r.name)}
                <button class="link" onclick={() => act('checkout', () => gitCheckout(id, r.name.replace(/^origin\//, '')), 'Branche locale créée.')}>Suivre</button>
              {/if}
            </div>
          {/each}
        {/if}
      </div>
      <div class="card graph">
        {#if repo.commits.length}
          <Graph commits={repo.commits} {selected} select={show} />
          {#if repo.truncated}<button class="link" onclick={() => (limit = Math.min(limit * 2, 1000))}>Plus de commits…</button>{/if}
        {:else}<p class="muted small">Aucun commit.</p>{/if}
      </div>
      {#if selected}
        <div class="card detail">
          {#if commit}
            <div class="row small"><code>{commit.sha.slice(0, 12)}</code><button class="link" onclick={() => (selected = '')}>✕</button></div>
            <p class="muted small">{commit.author} &lt;{commit.email}&gt; · {when(commit.time)}</p>
            <pre class="msg">{commit.message}</pre>
            {#each commit.files as f}
              <div class="small file"><code>{f.path}</code> <span class="muted">{f.status}</span>
                {#if f.binary}<span class="muted">binaire</span>{:else}<span class="add">+{f.additions}</span> <span class="del">−{f.deletions}</span>{/if}</div>
            {/each}
            <Diff diff={commit.diff} truncated={commit.diff_truncated} />
          {:else}<p class="muted small">Chargement…</p>{/if}
        </div>
      {/if}
    </div>
  {:else if !error}
    <p class="muted small">Chargement…</p>
  {/if}
</section>

<style>
  .page { display: flex; flex-direction: column; gap: var(--s3); height: calc(100vh - 2 * var(--s6)); min-height: 0; min-width: 0; overflow: hidden; }
  .head { justify-content: flex-start; gap: var(--s3); flex-wrap: wrap; }
  .head h1 { margin: 0; }
  .head button { padding: var(--s1) var(--s3); font-size: 13px; }
  .grow { flex: 1; }
  .tabs { display: flex; gap: var(--s3); }
  .tabs a { color: var(--muted); text-decoration: none; }
  .tabs a.active { color: var(--accent); }
  .fill { flex: 1; min-height: 0; }
  .split { display: flex; gap: var(--s3); flex: 1; min-height: 0; min-width: 0; }
  .side { width: 260px; flex: none; overflow: auto; display: flex; flex-direction: column; gap: var(--s2); padding: var(--s3); }
  .graph { flex: 1; overflow-y: auto; overflow-x: hidden; padding: var(--s2); min-width: 0; }
  .detail { width: min(520px, 40%); flex: none; min-width: 0; overflow: auto; padding: var(--s3); display: flex; flex-direction: column; gap: var(--s2); }
  .branch { border-bottom: 1px solid var(--line); padding-bottom: var(--s1); }
  .branch.cur code { color: var(--accent); }
  .acts { display: flex; gap: var(--s3); flex-wrap: wrap; }
  .acts button { padding: 0; }
  .msg { white-space: pre-wrap; font-size: 13px; margin: 0; }
  .file { display: flex; gap: var(--s2); }
  .add { color: var(--accent); }
  .del { color: var(--error); }
  .pre { white-space: pre-wrap; }
  @media (max-width: 1000px) { .split { flex-direction: column; } .side, .detail { width: auto; } }
</style>
