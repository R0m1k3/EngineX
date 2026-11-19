<script lang="ts">
  import { message, sessionCommit, sessionGit, sessionPullRequest, sessionPush, type SessionGit } from '../../lib/api';
  import GitBranch from '@lucide/svelte/icons/git-branch';
  import GitCommit from '@lucide/svelte/icons/git-commit-horizontal';
  import Upload from '@lucide/svelte/icons/upload';
  import GitPullRequest from '@lucide/svelte/icons/git-pull-request';
  import Button from '../../lib/ui/Button.svelte';
  import Input from '../../lib/ui/Input.svelte';
  import Textarea from '../../lib/ui/Textarea.svelte';
  import Badge from '../../lib/ui/Badge.svelte';
  import Icon from '../../lib/ui/Icon.svelte';
  import Skeleton from '../../lib/ui/Skeleton.svelte';
  import EmptyState from '../../lib/ui/EmptyState.svelte';
  import { toast } from '../../lib/ui/toast.svelte';

  let { sessionId }: { sessionId: string } = $props();

  let st = $state<SessionGit | null>(null);
  let msg = $state('');
  let title = $state('');
  let body = $state('');
  let busy = $state('');
  let error = $state('');
  let notice = $state('');

  async function load() {
    try {
      st = await sessionGit(sessionId);
    } catch (e) {
      error = message(e);
    }
  }

  async function act(label: string, f: () => Promise<string>) {
    busy = label;
    error = '';
    notice = '';
    try {
      notice = await f();
      toast(notice, 'ok');
    } catch (e) {
      error = message(e);
    } finally {
      busy = '';
      await load();
    }
  }

  const commit = () =>
    act('commit', async () => {
      const r = await sessionCommit(sessionId, msg);
      msg = '';
      return `Validé : ${r.sha.slice(0, 8)}.`;
    });
  const push = () => act('push', async () => `Poussé : ${(await sessionPush(sessionId)).branch}.`);
  const pr = () =>
    act('pr', async () => {
      const r = await sessionPullRequest(sessionId, title, body);
      title = body = '';
      return `Pull request #${r.number} créée : ${r.url}`;
    });

  $effect(() => {
    void sessionId;
    load();
  });
</script>

<div class="git">
  {#if !st}<div class="sec"><Skeleton lines={4} /></div>
  {:else if !st.git}
    <div class="sec">
      <EmptyState icon={GitBranch} title="Pas de dépôt git"
        text="Le dossier de cette session n’est pas un dépôt git : lancez « git init » dans le terminal, ou demandez-le à Codex." />
    </div>
  {:else}
    <div class="sec head">
      <div class="br"><Icon icon={GitBranch} size={16} /><code>{st.branch ?? 'HEAD détachée'}</code>
        {#if st.ahead}<Badge tone="warn">{st.ahead} commit{st.ahead > 1 ? 's' : ''} à pousser</Badge>{/if}
      </div>
      <p class="faint ell" title={st.origin ?? undefined}>{st.origin ? `origin : ${st.origin}` : 'Pas de remote origin.'}</p>
    </div>

    <div class="sec">
      <Textarea label="Message du commit (tout le dossier)" rows={3} bind:value={msg} placeholder="Ce qui a changé" />
      <div class="acts">
        <Button variant="primary" icon={GitCommit} onclick={commit} disabled={!msg.trim() || !!busy} busy={busy === 'commit'}>
          {busy === 'commit' ? 'Validation…' : 'Valider (commit)'}
        </Button>
        {#if st.origin}
          <Button icon={Upload} onclick={push} disabled={!!busy || !st.branch} busy={busy === 'push'}>{busy === 'push' ? 'Push…' : 'Pousser'}</Button>
        {/if}
      </div>
    </div>

    {#if st.forge}
      <div class="sec">
        <h3><Icon icon={GitPullRequest} size={16} />Pull request ({st.forge === 'github' ? 'GitHub' : 'Gitea'})</h3>
        <p class="muted">Pousse la branche puis ouvre la pull request vers la branche par défaut du dépôt.</p>
        <Input label="Titre" bind:value={title} />
        <Textarea label="Description" rows={4} bind:value={body} />
        <div class="acts">
          <Button icon={GitPullRequest} onclick={pr} disabled={!title.trim() || !!busy} busy={busy === 'pr'}>{busy === 'pr' ? 'Création…' : 'Créer la pull request'}</Button>
        </div>
      </div>
    {/if}
  {/if}
  {#if error}<p class="error pre sec" role="alert">{error}</p>{/if}
  {#if notice.includes('://')}<p class="pre sec ok-note">{notice}</p>{/if}
</div>

<style>
  .git { display: flex; flex-direction: column; overflow: auto; height: 100%; font-size: var(--fs-sm); }
  .sec { display: flex; flex-direction: column; gap: var(--s3); padding: var(--s4); border-bottom: 1px solid var(--line); margin: 0; }
  .head { gap: var(--s1); }
  .br { display: flex; align-items: center; gap: var(--s2); flex-wrap: wrap; color: var(--text); }
  .br :global(.ui-icon) { color: var(--faint); }
  .acts { display: flex; gap: var(--s2); flex-wrap: wrap; }
  h3 { display: flex; align-items: center; gap: var(--s2); margin: 0; font-size: var(--fs-md); }
  h3 :global(.ui-icon) { color: var(--faint); }
  p { margin: 0; }
  .pre { white-space: pre-wrap; overflow-wrap: anywhere; }
  .ok-note { color: var(--muted); }
  .error { border-bottom: 0; }
</style>
