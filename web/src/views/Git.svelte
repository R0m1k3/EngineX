<script lang="ts">
  import { stamp } from '../lib/ui/format';
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
  import { Button, EmptyState, Page, Icon, IconButton, Select, Tabs, confirmDialog, promptDialog, toast } from '../lib/ui';
  import ArrowDown from '@lucide/svelte/icons/arrow-down';
  import ArrowLeft from '@lucide/svelte/icons/arrow-left';
  import ArrowUp from '@lucide/svelte/icons/arrow-up';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import CloudDownload from '@lucide/svelte/icons/cloud-download';
  import CloudUpload from '@lucide/svelte/icons/cloud-upload';
  import FolderGit from '@lucide/svelte/icons/folder-git-2';
  import GitBranchIcon from '@lucide/svelte/icons/git-branch';
  import GitMerge from '@lucide/svelte/icons/git-merge';
  import GitPullRequest from '@lucide/svelte/icons/git-pull-request';
  import History from '@lucide/svelte/icons/history';
  import Plus from '@lucide/svelte/icons/plus';
  import RefreshCw from '@lucide/svelte/icons/refresh-cw';
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
  import X from '@lucide/svelte/icons/x';
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
  let busy = $state('');
  let limit = $state(300);
  /** Branche dont les actions sont dépliées. */
  let unfolded = $state('');

  let parts = $derived(sub.split('/'));
  let id = $derived(Number(parts[0]) || repos?.[0]?.topic_id || 0);
  let tab = $derived(parts[1] === 'pulls' ? 'pulls' : 'history');
  let current = $derived(repos?.find((r) => r.topic_id === id));
  let tabs = $derived([
    { id: 'history', label: 'Historique', icon: History, href: `#git/${id}` },
    ...(repo?.forge ? [{ id: 'pulls', label: 'Pull requests', icon: GitPullRequest, href: `#git/${id}/pulls` }] : []),
  ]);

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
    try {
      await f();
      toast(done, 'ok');
    } catch (e) {
      toast(message(e), 'error');
    } finally {
      busy = '';
      await loadRepo();
    }
  }

  async function newBranch() {
    const name = await promptDialog(selected ? `Nom de la branche, sur ${selected.slice(0, 7)}` : 'Nom de la branche, sur HEAD', {
      title: 'Nouvelle branche',
      confirm: 'Créer la branche',
      mono: true,
    });
    if (!name?.trim()) return;
    act('branch', () => gitBranch(id, name.trim(), selected || undefined), `Branche ${name.trim()} créée.`);
  }

  async function merge(b: string) {
    if (!repo?.head) return;
    const head = repo.head;
    if (!(await confirmDialog(`Fusionner ${b} dans ${head} ?`, { title: 'Fusionner la branche', confirm: 'Fusionner' }))) return;
    act('merge', () => gitMerge(id, b), `${b} fusionnée dans ${head}.`);
  }

  function go(topic: number, t: 'history' | 'pulls') {
    location.hash = t === 'pulls' ? `git/${topic}/pulls` : `git/${topic}`;
  }

  const when = stamp;
  const STATUS: Record<string, string> = { added: 'A', modified: 'M', deleted: 'D', renamed: 'R', copied: 'C', typechange: 'T' };
  const letter = (s: string) => STATUS[s] ?? s.slice(0, 1).toUpperCase();
  let msgSubject = $derived(commit ? commit.message.split('\n')[0] : '');
  let msgBody = $derived(commit ? commit.message.split('\n').slice(1).join('\n').trim() : '');

  loadRepos();
  $effect(() => {
    void id;
    void limit;
    commit = null;
    selected = '';
    loadRepo();
  });
</script>

{#snippet repoPick()}
  <div class="repo">
        <Select
          aria-label="Dépôt"
          value={id}
          options={(repos ?? []).map((r) => ({ value: r.topic_id, label: `${r.project} / ${r.topic}` }))}
          onchange={(e) => go(Number((e.currentTarget as HTMLSelectElement).value), tab as 'history' | 'pulls')}
    />
  </div>
{/snippet}

{#snippet remote()}
  {#if repo}
      <div class="remote-acts">
        <Button icon={RefreshCw} onclick={() => act('fetch', () => gitFetch(id), 'Fetch fait.')} disabled={!!busy || !repo.origin} busy={busy === 'fetch'}>
          {busy === 'fetch' ? 'Fetch…' : 'Fetch'}
        </Button>
        <Button icon={CloudDownload} onclick={() => act('pull', () => gitPull(id), 'À jour.')} disabled={!!busy || !repo.origin} busy={busy === 'pull'}>
          {busy === 'pull' ? 'Pull…' : 'Pull'}
        </Button>
        <Button icon={CloudUpload} onclick={() => act('push', () => gitPush(id), 'Poussé.')} disabled={!!busy || !repo.origin || repo.detached} busy={busy === 'push'}>
          {busy === 'push' ? 'Push…' : 'Push'}
        </Button>
      </div>
  {/if}
{/snippet}

{#snippet subbar()}
  {#if repo}
      <Tabs items={tabs} active={tab} label="Vues du dépôt" />
      <p class="where">
        <code class="ell">{current?.path}</code>
        {#if repo.origin}<span class="origin ell" title="origin">{repo.origin.replace(/^https?:\/\//, '')}</span>
        {:else}<span class="faint">pas de remote origin</span>{/if}
        {#if repo.origin && !repo.forge}<span class="faint">forge inconnue (Gitea : GITEA_URL dans Clés)</span>{/if}
      </p>
  {/if}
{/snippet}

<Page
  title="Git"
  meta={repos?.length ? repoPick : undefined}
  actions={repo && tab === 'history' ? remote : undefined}
  tabs={repos?.length && repo ? subbar : undefined}
>
<div class="gx" class:has-detail={!!selected && tab === 'history'}>

  {#if error}
    <p class="alert" role="alert"><Icon icon={TriangleAlert} size={16} /><span>{error}</span></p>
  {/if}

  {#if repos && !repos.length}
    <div class="pad">
      <EmptyState
        icon={FolderGit}
        title="Aucun topic n’est un dépôt git"
        text="Clonez un dépôt dans un topic (terminal d’une session, ou demandez-le à Codex) : il apparaîtra ici."
      />
    </div>
  {:else if repo && tab === 'pulls' && repo.forge}
    <div class="fill"><Pulls repoId={id} forge={repo.forge} branches={repo.branches} /></div>
  {:else if repo}
    <div class="cols">
      <aside class="side" aria-label="Branches">
        <div class="sec-h">
          <h2>Branches</h2>
          <IconButton icon={Plus} label={selected ? `Nouvelle branche sur ${selected.slice(0, 7)}` : 'Nouvelle branche sur HEAD'} onclick={newBranch} />
        </div>
        <ul class="branches">
          {#each repo.branches as b (b.name)}
            <li class="br" class:cur={b.current} class:unf={unfolded === b.name}>
              <button class="br-hit" onclick={() => (unfolded = unfolded === b.name || b.current ? '' : b.name)} aria-expanded={b.current ? undefined : unfolded === b.name}>
                <Icon icon={GitBranchIcon} size={16} class="bicon" />
                <code class="bname ell">{b.name}</code>
                {#if b.current}<span class="here">actuelle</span>{/if}
                <span class="sync">
                  {#if b.ahead}<span class="n" title="{b.ahead} commit(s) à pousser"><Icon icon={ArrowUp} size={14} />{b.ahead}</span>{/if}
                  {#if b.behind}<span class="n" title="{b.behind} commit(s) à récupérer"><Icon icon={ArrowDown} size={14} />{b.behind}</span>{/if}
                  {#if !b.upstream}<span class="tag" title="Jamais poussée">local</span>
                  {:else if b.upstream_gone}<span class="tag gone" title="Supprimée en amont">amont disparu</span>{/if}
                </span>
                {#if !b.current}<span class="chev"><Icon icon={ChevronRight} size={16} /></span>{/if}
              </button>
              {#if !b.current && unfolded === b.name}
                <div class="br-acts">
                  <Button size="sm" variant="ghost" onclick={() => act('checkout', () => gitCheckout(id, b.name), `Sur ${b.name}.`)} disabled={!!busy}>Basculer</Button>
                  {#if repo.head}<Button size="sm" variant="ghost" icon={GitMerge} onclick={() => merge(b.name)} disabled={!!busy}>Fusionner dans {repo.head}</Button>{/if}
                  {#if repo.origin}<Button size="sm" variant="ghost" icon={CloudUpload} onclick={() => act('push', () => gitPush(id, b.name), `${b.name} poussée.`)} disabled={!!busy}>Pousser</Button>{/if}
                </div>
              {/if}
            </li>
          {/each}
        </ul>
        {#if repo.remotes.length}
          <div class="sec-h"><h2>Distantes</h2></div>
          <ul class="branches">
            {#each repo.remotes as r (r.name)}
              <li class="rm">
                <code class="ell">{r.name}</code>
                {#if !repo.branches.some((b) => `origin/${b.name}` === r.name)}
                  <Button size="sm" variant="ghost" onclick={() => act('checkout', () => gitCheckout(id, r.name.replace(/^origin\//, '')), 'Branche locale créée.')}>Suivre</Button>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
      </aside>

      <section class="hist" aria-label="Historique">
        {#if repo.commits.length}
          <Graph commits={repo.commits} {selected} select={show} />
          {#if repo.truncated}
            <div class="more"><Button variant="ghost" onclick={() => (limit = Math.min(limit * 2, 1000))}>Plus de commits</Button></div>
          {/if}
        {:else}
          <div class="pad"><EmptyState icon={History} title="Aucun commit" text="Le premier commit du topic apparaîtra ici." /></div>
        {/if}
      </section>

      {#if selected}
        <section class="detail" aria-label="Commit">
          <header class="det-h">
            <IconButton icon={ArrowLeft} label="Retour à l’historique" size="lg" class="back" onclick={() => (selected = '')} />
            <code class="sha">{(commit?.sha ?? selected).slice(0, 12)}</code>
            <span class="grow"></span>
            <IconButton icon={X} label="Fermer" class="close" onclick={() => (selected = '')} />
          </header>
          {#if commit}
            <div class="det-body">
              <div class="msg">
                <p class="subject">{msgSubject}</p>
                {#if msgBody}<pre class="body-t">{msgBody}</pre>{/if}
                <p class="who">{commit.author} <span class="faint">&lt;{commit.email}&gt;</span>, {when(commit.time)}</p>
              </div>
              {#if commit.files.length}
                <ul class="files">
                  {#each commit.files as f (f.path)}
                    <li>
                      <span class="st st-{f.status}" title={f.status}>{letter(f.status)}</span>
                      <code class="path ell" title={f.path}>{f.path}</code>
                      {#if f.binary}<span class="faint">binaire</span>
                      {:else}<span class="plus">+{f.additions}</span><span class="minus">−{f.deletions}</span>{/if}
                    </li>
                  {/each}
                </ul>
              {/if}
              <Diff diff={commit.diff} truncated={commit.diff_truncated} />
            </div>
          {:else}
            <p class="loading">Chargement…</p>
          {/if}
        </section>
      {/if}
    </div>
  {:else if !error}
    <p class="loading pad">Chargement…</p>
  {/if}
</div>
</Page>

<style>
  .gx { flex: 1; min-height: 0; display: flex; flex-direction: column; background: var(--bg); }
  .repo { min-width: 200px; }
  .grow { flex: 1; min-width: 0; }
  .remote-acts { display: flex; gap: var(--s2); }

  .where { flex: 1; min-width: 0; display: flex; justify-content: flex-end; align-items: center; gap: var(--s4); height: 40px; margin: 0; font-size: var(--fs-sm); color: var(--muted); }
  .where code { color: var(--muted); }
  .origin { color: var(--muted); }

  .alert { display: flex; align-items: center; gap: var(--s2); flex: none; margin: 0; padding: var(--s2) var(--s4); background: var(--danger-soft); color: var(--danger); font-size: var(--fs-sm); white-space: pre-wrap; border-bottom: 1px solid var(--line); }
  .pad { padding: var(--s6); }
  .fill { flex: 1; min-height: 0; }
  .loading { margin: 0; padding: var(--s4); color: var(--faint); font-size: var(--fs-sm); }

  .cols { flex: 1; min-height: 0; display: grid; grid-template-columns: 260px minmax(0, 1fr); }
  .has-detail .cols { grid-template-columns: 260px minmax(0, 1fr) minmax(420px, 40%); }

  /* ---------- Branches ---------- */
  .side { min-height: 0; overflow: auto; background: var(--panel); border-right: 1px solid var(--line); padding-bottom: var(--s3); }
  .sec-h { display: flex; align-items: center; justify-content: space-between; height: 40px; padding: 0 var(--s2) 0 var(--s4); }
  h2 { margin: 0; font-size: var(--fs-sm); font-weight: 600; color: var(--muted); }
  .branches { list-style: none; margin: 0 0 var(--s2); padding: 0; }
  .br-hit {
    width: 100%;
    height: 36px;
    min-height: 0;
    display: flex;
    align-items: center;
    justify-content: flex-start;
    gap: var(--s2);
    padding: 0 var(--s2) 0 var(--s4);
    border: 0;
    border-radius: 0;
    background: none;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-sm);
    font-weight: 400;
    filter: none;
  }
  .br-hit:hover { background: var(--raised); }
  .br-hit:focus-visible { outline-offset: -2px; }
  .br.cur .br-hit { background: var(--accent-soft); box-shadow: inset 2px 0 0 var(--accent); cursor: default; }
  .br :global(.bicon) { color: var(--faint); }
  .br.cur :global(.bicon), .br.cur .bname { color: var(--accent); }
  .bname { flex: 0 1 auto; }
  .here { color: var(--accent); font-size: var(--fs-sm); flex: none; }
  .sync { display: flex; align-items: center; gap: 6px; margin-left: auto; flex: none; color: var(--muted); font-variant-numeric: tabular-nums; }
  .n { display: inline-flex; align-items: center; gap: 1px; }
  .tag { font-size: var(--fs-xs); line-height: 20px; padding: 0 6px; border-radius: var(--r-sm); background: var(--raised); color: var(--faint); }
  .br-hit:hover .tag { background: var(--bg); }
  .tag.gone { background: var(--warn-soft); color: var(--warn); }
  .chev { color: var(--faint); display: grid; transition: transform 0.12s; }
  .br.unf .chev { transform: rotate(90deg); }
  .br-acts { display: flex; flex-wrap: wrap; gap: 2px; padding: var(--s1) var(--s2) var(--s2) 36px; }
  .rm { display: flex; align-items: center; justify-content: space-between; gap: var(--s2); height: 32px; padding: 0 var(--s2) 0 var(--s4); }
  .rm code { color: var(--muted); }

  /* ---------- Historique ---------- */
  .hist { min-width: 0; min-height: 0; overflow-y: auto; overflow-x: hidden; }
  .more { padding: var(--s2) var(--s4); border-top: 1px solid var(--line); }

  /* ---------- Commit ---------- */
  .detail { min-width: 0; min-height: 0; display: flex; flex-direction: column; border-left: 1px solid var(--line); }
  .det-h { display: flex; align-items: center; gap: var(--s2); height: 40px; flex: none; padding: 0 var(--s2) 0 var(--s4); background: var(--panel); border-bottom: 1px solid var(--line); }
  .det-h :global(.back) { display: none; }
  .sha { color: var(--text); }
  .det-body { flex: 1; min-height: 0; overflow: auto; }
  .msg { padding: var(--s4); border-bottom: 1px solid var(--line); }
  .subject { margin: 0 0 var(--s2); font-weight: 600; line-height: 1.4; }
  .body-t { margin: 0 0 var(--s3); white-space: pre-wrap; font-family: var(--font); font-size: var(--fs-sm); color: var(--muted); }
  .who { margin: 0; font-size: var(--fs-sm); color: var(--muted); }
  .files { list-style: none; margin: 0; padding: var(--s2) 0; border-bottom: 1px solid var(--line); }
  .files li { display: flex; align-items: center; gap: var(--s2); padding: 4px var(--s4); font-size: var(--fs-sm); font-variant-numeric: tabular-nums; }
  .path { flex: 1; color: var(--text); }
  .st { width: 16px; flex: none; font-family: var(--mono); font-size: var(--fs-code); font-weight: 600; color: var(--warn); }
  .st-added { color: var(--add-fg); }
  .st-deleted { color: var(--del-fg); }
  .plus { color: var(--add-fg); }
  .minus { color: var(--del-fg); }

  /* ---------- Téléphone : colonnes empilées, commit en plein écran ---------- */
  @media (max-width: 1100px) {
    .has-detail .cols { grid-template-columns: minmax(0, 1fr) minmax(360px, 50%); }
    .has-detail .side { display: none; }
  }
  @media (max-width: 767px) {
    .gx { padding: 0 var(--safe-right) 0 var(--safe-left); }
    .repo { flex: 1; min-width: 0; }
    .repo :global(select) { width: 100%; height: var(--touch); font-size: var(--fs-md); }
    .remote-acts { width: 100%; }
    .remote-acts :global(.btn) { flex: 1; height: var(--touch); }
    .where { flex-basis: 100%; margin: 0 calc(-1 * var(--s1)); justify-content: flex-start; flex-wrap: wrap; height: auto; padding: var(--s2) var(--s4); gap: var(--s1) var(--s3); border-top: 1px solid var(--line); }

    .cols, .has-detail .cols { display: flex; flex-direction: column; overflow: auto; }
    .hist { order: -1; border-bottom: 1px solid var(--line); }
    .side { overflow: visible; border-right: 0; flex: none; }
    .br-hit { height: var(--touch); }
    .br-acts :global(.btn) { height: var(--touch); }
    .rm { height: var(--touch); }
    .hist { overflow: visible; flex: none; }
    .has-detail .side, .has-detail .hist { display: none; }
    .has-detail .cols { display: flex; }
    .detail { flex: 1; border-left: 0; }
    .det-h { height: 56px; padding-left: 0; }
    .det-h :global(.back) { display: inline-grid; }
    .det-h :global(.close) { display: none; }
  }
</style>
