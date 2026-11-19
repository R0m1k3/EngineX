<script lang="ts">
  import { answerRequest, message, type PendingRequest } from '../../lib/api';

  let { req, sessionId }: { req: PendingRequest; sessionId: string } = $props();
  let error = $state('');
  let sending = $state(false);
  // Réponses aux questions : id de la question -> valeur choisie ou saisie.
  let values = $state<Record<string, string>>({});

  async function send(body: { decision?: string; answers?: unknown }) {
    sending = true;
    error = '';
    try {
      await answerRequest(sessionId, req.rid, body);
    } catch (e) {
      error = message(e);
      sending = false;
    }
  }

  function submitAnswers(e: SubmitEvent) {
    e.preventDefault();
    const answers: Record<string, { answers: string[] }> = {};
    for (const q of req.params.questions ?? []) answers[q.id] = { answers: values[q.id] ? [values[q.id]] : [] };
    send({ answers });
  }

  let isQuestion = $derived(req.method === 'item/tool/requestUserInput');
  let title = $derived(
    req.method === 'item/commandExecution/requestApproval'
      ? 'Codex veut lancer une commande'
      : req.method === 'item/fileChange/requestApproval'
        ? 'Codex veut modifier des fichiers'
        : 'Codex pose une question',
  );
</script>

<div class="card approval">
  <h3>{title}</h3>
  {#if req.params.command}<pre class="cmd">{req.params.command}</pre>{/if}
  {#if req.params.cwd}<p class="small muted">Dossier : <code>{req.params.cwd}</code></p>{/if}
  {#if req.params.reason}<p class="small">{req.params.reason}</p>{/if}
  {#if req.params.grantRoot}<p class="small">Demande l’écriture sous <code>{req.params.grantRoot}</code>.</p>{/if}

  {#if isQuestion}
    <form onsubmit={submitAnswers}>
      {#each req.params.questions ?? [] as q (q.id)}
        <fieldset>
          {#if q.header}<legend>{q.header}</legend>{/if}
          <p>{q.question}</p>
          {#each q.options ?? [] as o (o.label)}
            <label class="check">
              <input type="radio" name={q.id} value={o.label} bind:group={values[q.id]} />
              <span>{o.label}{#if o.description} <span class="muted small">{o.description}</span>{/if}</span>
            </label>
          {/each}
          {#if !q.options?.length || q.isOther}
            <input type={q.isSecret ? 'password' : 'text'} bind:value={values[q.id]} placeholder="Votre réponse" />
          {/if}
        </fieldset>
      {/each}
      <div class="actions">
        <button type="submit" disabled={sending}>Répondre</button>
        <button type="button" class="secondary" disabled={sending} onclick={() => send({ answers: {} })}>Passer</button>
      </div>
    </form>
  {:else}
    <div class="actions">
      <button disabled={sending} onclick={() => send({ decision: 'accept' })}>Autoriser</button>
      <button class="secondary" disabled={sending} onclick={() => send({ decision: 'acceptForSession' })}>Autoriser pour la session</button>
      <button class="secondary" disabled={sending} onclick={() => send({ decision: 'decline' })}>Refuser</button>
      <button class="link" disabled={sending} onclick={() => send({ decision: 'cancel' })}>Arrêter le tour</button>
    </div>
  {/if}
  {#if error}<p class="error small">{error}</p>{/if}
</div>

<style>
  .approval { border-color: var(--warn); display: flex; flex-direction: column; gap: var(--s2); }
  .approval h3 { margin: 0; }
  .cmd { margin: 0; white-space: pre-wrap; word-break: break-all; background: var(--bg); padding: var(--s2); border-radius: var(--radius); font-size: var(--fs-sm); }
  fieldset { border: 1px solid var(--line); border-radius: var(--radius); margin: 0 0 var(--s2); padding: var(--s2) var(--s3); display: flex; flex-direction: column; gap: var(--s1); }
  legend { color: var(--muted); font-size: var(--fs-sm); }
</style>
