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
  import { Badge, Button, Dialog, EmptyState, Icon, IconButton, Input, Segmented, Select, Textarea, confirmDialog } from '../../lib/ui';
  import ArrowLeft from '@lucide/svelte/icons/arrow-left';
  import ExternalLink from '@lucide/svelte/icons/external-link';
  import GitMerge from '@lucide/svelte/icons/git-merge';
  import GitPullRequest from '@lucide/svelte/icons/git-pull-request';
  import GitPullRequestClosed from '@lucide/svelte/icons/git-pull-request-closed';
  import GitPullRequestDraft from '@lucide/svelte/icons/git-pull-request-draft';
  import Plus from '@lucide/svelte/icons/plus';
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
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

  const METHOD: Record<string, string> = { merge: 'commit de fusion', squash: 'squash', rebase: 'rebase' };
  const merge = async () => {
    if (!detail) return;
    const ok = await confirmDialog(`Fusionner #${detail.number} (${METHOD[method]}) dans ${detail.base} ?`, {
      title: 'Fusionner la pull request',
      confirm: 'Fusionner',
    });
    if (!ok) return;
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
  const icon = (p: PullRequest) =>
    p.state === 'merged' ? GitMerge : p.state === 'closed' ? GitPullRequestClosed : p.draft ? GitPullRequestDraft : GitPullRequest;
  const kind = (p: PullRequest) => (p.state === 'merged' ? 'merged' : p.state === 'closed' ? 'closed' : p.draft ? 'draft' : 'open');
  const tone = (p: PullRequest) => (p.state === 'open' && !p.draft ? 'ok' : 'neutral');
  const label = (p: PullRequest) => (STATE[p.state] ?? p.state) + (p.draft ? ', brouillon' : '');
  const forgeName = $derived(forge === 'github' ? 'GitHub' : 'Gitea');
  const pushed = $derived(branches.filter((b) => b.upstream));

  $effect(() => {
    void filter;
    void repoId;
    load();
  });
</script>

<div class="pulls" class:has-detail={!!detail}>
  <aside class="list" aria-label="Pull requests">
    <div class="list-h">
      <Segmented
        label="Filtre"
        bind:value={filter}
        options={[{ value: 'open', label: 'Ouvertes' }, { value: 'closed', label: 'Fermées' }, { value: 'all', label: 'Toutes' }]}
      />
      <Button icon={Plus} onclick={() => (creating = true)}>Nouvelle</Button>
    </div>
    {#if list === null}<p class="loading">Chargement…</p>
    {:else if !list.length && !error}
      <div class="pad"><EmptyState icon={GitPullRequest} title="Aucune pull request" text="Poussez une branche, puis ouvrez-en une avec Nouvelle." /></div>
    {/if}
    <ul class="prs">
      {#each list ?? [] as p (p.number)}
        <li>
          <button class="pr" class:sel={detail?.number === p.number} onclick={() => open(p.number)} aria-current={detail?.number === p.number || undefined}>
            <span class="st {kind(p)}" title={label(p)}><Icon icon={icon(p)} size={16} label={label(p)} /></span>
            <span class="pr-main">
              <span class="pr-title">{p.title}</span>
              <span class="pr-meta">
                <span class="num">#{p.number}</span>
                <code class="ell">{p.head} → {p.base}</code>
                <span class="ell">{p.author}</span>
              </span>
            </span>
          </button>
        </li>
      {/each}
    </ul>
  </aside>

  <section class="pdetail" aria-label="Pull request">
    {#if error}<p class="alert" role="alert"><Icon icon={TriangleAlert} size={16} /><span>{error}</span></p>{/if}
    {#if detail}
      <header class="d-h">
        <IconButton icon={ArrowLeft} label="Retour à la liste" size="lg" class="back" onclick={() => (detail = null)} />
        <div class="d-title">
          <h2><span class="num">#{detail.number}</span> {detail.title}</h2>
          <p class="d-meta">
            <Badge tone={tone(detail)}>{label(detail)}</Badge>
            <span><code>{detail.head}</code> → <code>{detail.base}</code></span>
            <span>{detail.author}, {when(detail.created_at)}</span>
          </p>
        </div>
        <a class="ext" href={detail.url} target="_blank" rel="noopener noreferrer">Ouvrir sur {forgeName}<Icon icon={ExternalLink} size={16} /></a>
      </header>

      {#if detail.body}<div class="sec md-body"><Markdown text={detail.body} /></div>{/if}

      {#if detail.state === 'open'}
        <div class="sec merge">
          <Select
            aria-label="Méthode de fusion"
            bind:value={method}
            options={[{ value: 'merge', label: 'Commit de fusion' }, { value: 'squash', label: 'Squash' }, { value: 'rebase', label: 'Rebase' }]}
          />
          <Button variant="primary" icon={GitMerge} onclick={merge} disabled={busy || detail.mergeable === false}>Fusionner</Button>
          {#if detail.mergeable === false}<span class="conflict"><Icon icon={TriangleAlert} size={16} />Conflits : à résoudre d’abord.</span>{/if}
        </div>
      {/if}

      <h3 class="sec-t">Modifications</h3>
      <div class="diffbox"><Diff diff={detail.diff} truncated={detail.diff_truncated} /></div>

      <h3 class="sec-t">Commentaires <span class="count">{detail.comments.length}</span></h3>
      <div class="comments">
        {#each detail.comments as c}
          <article class="comment">
            <p class="c-h"><strong>{c.author}</strong> <span class="faint">{when(c.created_at)}</span></p>
            <Markdown text={c.body ?? ''} />
          </article>
        {/each}
        <div class="reply">
          <Textarea rows={3} bind:value={reply} placeholder="Votre commentaire" aria-label="Votre commentaire" />
          <div><Button onclick={comment} disabled={busy || !reply.trim()}>Commenter</Button></div>
        </div>
      </div>
    {:else if list?.length}
      <div class="pad"><EmptyState icon={GitPullRequest} title="Choisissez une pull request" text="Sa description, ses modifications et ses commentaires s’affichent ici." /></div>
    {/if}
  </section>
</div>

<Dialog bind:open={creating} title="Nouvelle pull request" width="520px">
  <form id="pr-new" class="new" onsubmit={(e) => { e.preventDefault(); create(); }}>
    <Select label="Branche (déjà poussée)" bind:value={form.head} required>
      <option value="" disabled>Choisir</option>
      {#each pushed as b (b.name)}<option value={b.name}>{b.name}</option>{/each}
    </Select>
    <Input label="Vers" hint="Vide : branche par défaut du dépôt." bind:value={form.base} placeholder="main" />
    <Input label="Titre" bind:value={form.title} required />
    <Textarea label="Description" rows={4} bind:value={form.body} />
    {#if error && creating}<p class="err" role="alert">{error}</p>{/if}
  </form>
  {#snippet actions()}
    <Button onclick={() => (creating = false)}>Annuler</Button>
    <Button variant="primary" type="submit" form="pr-new" disabled={busy || !form.head || !form.title.trim()}>Créer la pull request</Button>
  {/snippet}
</Dialog>

<style>
  .pulls { display: grid; grid-template-columns: minmax(280px, 360px) minmax(0, 1fr); height: 100%; min-height: 0; }

  .list { display: flex; flex-direction: column; min-height: 0; background: var(--panel); border-right: 1px solid var(--line); }
  .list-h { display: flex; align-items: center; justify-content: space-between; gap: var(--s2); padding: var(--s3) var(--s3) var(--s3) var(--s4); border-bottom: 1px solid var(--line); flex: none; }
  .list-h :global(.seg) { flex: none; }
  .prs { list-style: none; margin: 0; padding: 0; overflow: auto; flex: 1; min-height: 0; }
  .pr {
    width: 100%;
    min-height: 60px;
    display: flex;
    align-items: flex-start;
    justify-content: flex-start;
    gap: var(--s3);
    padding: var(--s3) var(--s4);
    border: 0;
    border-bottom: 1px solid var(--line);
    border-radius: 0;
    background: none;
    color: var(--text);
    font: inherit;
    font-weight: 400;
    text-align: left;
    white-space: normal;
    filter: none;
  }
  .pr:hover { background: var(--raised); }
  .pr.sel { background: var(--accent-soft); box-shadow: inset 2px 0 0 var(--accent); }
  .pr:focus-visible { outline-offset: -2px; }
  .st { flex: none; padding-top: 2px; color: var(--faint); }
  .st.open { color: var(--accent); }
  .st.merged { color: var(--code-type); }
  .st.closed { color: var(--danger); }
  .pr-main { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .pr-title { font-size: var(--fs-sm); line-height: 1.4; }
  .pr-meta { display: flex; gap: var(--s3); min-width: 0; color: var(--muted); font-size: var(--fs-sm); }
  .pr-meta code { color: var(--muted); }
  .num { color: var(--faint); font-variant-numeric: tabular-nums; flex: none; }
  .loading { margin: 0; padding: var(--s4); color: var(--faint); font-size: var(--fs-sm); }
  .pad { padding: var(--s4) var(--s6); }

  .pdetail { min-width: 0; min-height: 0; overflow: auto; }
  .alert { display: flex; align-items: center; gap: var(--s2); margin: 0; padding: var(--s2) var(--s4); background: var(--danger-soft); color: var(--danger); font-size: var(--fs-sm); white-space: pre-wrap; border-bottom: 1px solid var(--line); }
  .d-h { display: flex; align-items: flex-start; gap: var(--s3); padding: var(--s4) var(--s4) var(--s4) var(--s6); border-bottom: 1px solid var(--line); }
  .d-h :global(.back) { display: none; }
  .d-title { flex: 1; min-width: 0; }
  .ext { display: inline-flex; align-items: center; gap: 6px; flex: none; height: 32px; padding: 0 var(--s2); border-radius: var(--r); color: var(--accent); font-size: var(--fs-sm); text-decoration: none; }
  .ext:hover { background: var(--raised); text-decoration: underline; }
  h2 { margin: 0 0 var(--s2); font-size: var(--fs-lg); font-weight: 600; line-height: 1.35; }
  h2 .num { font-weight: 400; }
  .d-meta { display: flex; flex-wrap: wrap; align-items: center; gap: var(--s2) var(--s4); margin: 0; font-size: var(--fs-sm); color: var(--muted); }
  .d-meta code { color: var(--text); }
  .sec { padding: var(--s4) var(--s6); border-bottom: 1px solid var(--line); }
  .md-body { max-width: 760px; line-height: 1.6; }
  .merge { display: flex; flex-wrap: wrap; align-items: center; gap: var(--s2) var(--s3); background: var(--panel); }
  .conflict { display: inline-flex; align-items: center; gap: 6px; color: var(--danger); font-size: var(--fs-sm); }
  .sec-t { display: flex; align-items: center; gap: var(--s2); margin: 0; padding: var(--s5) var(--s6) var(--s2); font-size: var(--fs-md); font-weight: 600; }
  .count { font-size: var(--fs-count); color: var(--muted); background: var(--raised); border-radius: 8px; padding: 0 6px; line-height: 18px; font-weight: 400; font-variant-numeric: tabular-nums; }
  .diffbox { margin: 0 var(--s6); border: 1px solid var(--line); border-radius: var(--r); overflow: hidden; }
  .diffbox :global(.diffv) { overflow-x: auto; }
  .comments { display: flex; flex-direction: column; gap: var(--s3); padding: 0 var(--s6) var(--s6); max-width: 820px; }
  .comment { border: 1px solid var(--line); border-radius: var(--r); padding: var(--s3) var(--s4); background: var(--panel); }
  .c-h { margin: 0 0 var(--s2); font-size: var(--fs-sm); display: flex; gap: var(--s2); }
  .reply { display: flex; flex-direction: column; gap: var(--s2); }

  .new { display: flex; flex-direction: column; gap: var(--s4); }
  .err { margin: 0; color: var(--danger); font-size: var(--fs-sm); }

  @media (max-width: 767px) {
    .pulls { grid-template-columns: minmax(0, 1fr); }
    .has-detail .list, .pulls:not(.has-detail) .pdetail { display: none; }
    .pulls:not(.has-detail):has(.alert) .pdetail { display: block; }
    .list { border-right: 0; }
    .list-h { flex-wrap: wrap; }
    .list-h :global(.btn) { height: var(--touch); }
    .list-h :global(.seg button) { height: 42px; }
    .pr-meta { flex-wrap: wrap; gap: 0 var(--s3); }
    .d-h { padding: var(--s2) var(--s4) var(--s4) 0; flex-wrap: wrap; }
    .d-h :global(.back) { display: inline-grid; }
    .d-title { flex-basis: calc(100% - 60px); }
    .ext { margin-left: 44px; height: var(--touch); }
    .sec, .sec-t { padding-left: var(--s4); padding-right: var(--s4); }
    .diffbox { margin: 0; border-left: 0; border-right: 0; border-radius: 0; }
    .comments { padding: 0 var(--s4) var(--s6); }
    .merge :global(.btn), .merge :global(select) { height: var(--touch); }
  }
</style>
