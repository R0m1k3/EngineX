<script lang="ts">
  import { message, sessionCommit, sessionGit, sessionPullRequest, sessionPush, type SessionGit } from '../../lib/api';

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
  {#if !st}<p class="muted small">Chargement…</p>
  {:else if !st.git}
    <p class="muted small">Le dossier de cette session n’est pas un dépôt git (lancez <code>git init</code> dans le terminal, ou demandez-le à Codex).</p>
  {:else}
    <p class="small">
      Branche <code>{st.branch ?? 'HEAD détachée'}</code>
      {#if st.ahead}<span class="muted"> · {st.ahead} commit(s) à pousser</span>{/if}
    </p>
    <p class="muted small">{st.origin ? `origin : ${st.origin}` : 'Pas de remote origin.'}</p>

    <label>Message du commit (tout le dossier)
      <textarea rows="3" bind:value={msg} placeholder="Ce qui a changé"></textarea>
    </label>
    <button onclick={commit} disabled={!msg.trim() || !!busy}>{busy === 'commit' ? 'Validation…' : 'Valider (commit)'}</button>

    {#if st.origin}
      <button class="secondary" onclick={push} disabled={!!busy || !st.branch}>{busy === 'push' ? 'Push…' : 'Pousser'}</button>
    {/if}

    {#if st.forge}
      <h3>Pull request ({st.forge === 'github' ? 'GitHub' : 'Gitea'})</h3>
      <p class="muted small">Pousse la branche puis ouvre la pull request vers la branche par défaut du dépôt.</p>
      <label>Titre <input bind:value={title} /></label>
      <label>Description <textarea rows="4" bind:value={body}></textarea></label>
      <button onclick={pr} disabled={!title.trim() || !!busy}>{busy === 'pr' ? 'Création…' : 'Créer la pull request'}</button>
    {/if}
  {/if}
  {#if error}<p class="error small pre">{error}</p>{/if}
  {#if notice}<p class="small pre">{notice}</p>{/if}
</div>

<style>
  .git { display: flex; flex-direction: column; gap: var(--s2); overflow: auto; height: 100%; }
  .git > button { align-self: flex-start; }
  h3 { margin: var(--s3) 0 0; }
  textarea { width: 100%; background: var(--bg); color: var(--text); border: 1px solid var(--line); border-radius: var(--radius); padding: var(--s2); font: inherit; }
  .pre { white-space: pre-wrap; overflow-wrap: anywhere; }
  p { margin: 0; }
</style>
