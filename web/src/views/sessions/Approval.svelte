<script lang="ts">
  import { answerRequest, message, type PendingRequest } from '../../lib/api';
  import Shield from '@lucide/svelte/icons/shield';
  import CircleQuestion from '@lucide/svelte/icons/circle-question-mark';
  import Check from '@lucide/svelte/icons/check';
  import X from '@lucide/svelte/icons/x';
  import Button from '../../lib/ui/Button.svelte';
  import Icon from '../../lib/ui/Icon.svelte';
  import { screen } from './phone.svelte';

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
      ? 'EX veut lancer une commande'
      : req.method === 'item/fileChange/requestApproval'
        ? 'EX veut modifier des fichiers'
        : 'EX pose une question',
  );
  const size = $derived(screen.phone ? 'lg' : 'md');
</script>

<section class="approval" class:phone={screen.phone} aria-label={title}>
  <div class="ah"><Icon icon={isQuestion ? CircleQuestion : Shield} />{title}</div>
  {#if req.params.command}<pre class="cmd">{req.params.command}</pre>{/if}
  {#if req.params.cwd}<p class="why">Dossier : <code>{req.params.cwd}</code></p>{/if}
  {#if req.params.reason}<p class="why">{req.params.reason}</p>{/if}
  {#if req.params.grantRoot}<p class="why">Demande l’écriture sous <code>{req.params.grantRoot}</code>.</p>{/if}

  {#if isQuestion}
    <form onsubmit={submitAnswers}>
      {#each req.params.questions ?? [] as q (q.id)}
        <fieldset>
          {#if q.header}<legend>{q.header}</legend>{/if}
          <p>{q.question}</p>
          {#each q.options ?? [] as o (o.label)}
            <label class="opt">
              <input type="radio" name={q.id} value={o.label} bind:group={values[q.id]} />
              <span>{o.label}{#if o.description} <span class="muted">{o.description}</span>{/if}</span>
            </label>
          {/each}
          {#if !q.options?.length || q.isOther}
            <input type={q.isSecret ? 'password' : 'text'} bind:value={values[q.id]} placeholder="Votre réponse" aria-label={q.question} />
          {/if}
        </fieldset>
      {/each}
      <div class="acts">
        <Button variant="primary" type="submit" {size} disabled={sending}>Répondre</Button>
        <Button {size} disabled={sending} onclick={() => send({ answers: {} })}>Passer</Button>
      </div>
    </form>
  {:else if screen.phone}
    <div class="big">
      <Button icon={X} size="lg" disabled={sending} onclick={() => send({ decision: 'decline' })}>Refuser</Button>
      <Button variant="primary" icon={Check} size="lg" disabled={sending} onclick={() => send({ decision: 'accept' })}>Accepter</Button>
    </div>
    <div class="acts small-acts">
      <Button variant="ghost" size="lg" disabled={sending} onclick={() => send({ decision: 'acceptForSession' })}>Accepter pour la session</Button>
      <Button variant="ghost" size="lg" disabled={sending} onclick={() => send({ decision: 'cancel' })}>Arrêter le tour</Button>
    </div>
  {:else}
    <div class="acts">
      <Button variant="primary" icon={Check} disabled={sending} onclick={() => send({ decision: 'accept' })}>Accepter</Button>
      <Button icon={X} disabled={sending} onclick={() => send({ decision: 'decline' })}>Refuser</Button>
      <Button variant="ghost" disabled={sending} onclick={() => send({ decision: 'acceptForSession' })}>Accepter pour la session</Button>
      <span class="grow"></span>
      <Button variant="ghost" size="sm" disabled={sending} onclick={() => send({ decision: 'cancel' })}>Arrêter le tour</Button>
    </div>
  {/if}
  {#if error}<p class="error small" role="alert">{error}</p>{/if}
</section>

<style>
  .approval {
    border: 1px solid var(--warn);
    background: var(--warn-soft);
    border-radius: var(--r);
    padding: var(--s4);
    display: flex;
    flex-direction: column;
    gap: var(--s3);
  }
  .ah { display: flex; align-items: center; gap: var(--s2); font-weight: 600; color: var(--text); }
  .ah :global(.ui-icon) { color: var(--warn); flex: none; }
  .cmd {
    margin: 0;
    padding: var(--s2) var(--s3);
    background: var(--bg);
    border: 1px solid var(--line-strong);
    border-radius: var(--r);
    white-space: pre-wrap;
    word-break: break-all;
    color: var(--text);
  }
  .why { margin: 0; font-size: var(--fs-sm); color: var(--muted); overflow-wrap: anywhere; }
  .acts { display: flex; gap: var(--s2); align-items: center; flex-wrap: wrap; }
  .grow { flex: 1; }
  .big { display: grid; grid-template-columns: 1fr 1fr; gap: var(--s3); }
  .big :global(.btn) { height: 48px; }
  .small-acts { justify-content: space-between; }
  .error { margin: 0; }
  form { display: flex; flex-direction: column; gap: var(--s3); }
  fieldset { border: 1px solid var(--line-strong); border-radius: var(--r); margin: 0; padding: var(--s2) var(--s3) var(--s3); display: flex; flex-direction: column; gap: var(--s2); background: var(--bg); }
  fieldset p { margin: 0; }
  legend { color: var(--muted); font-size: var(--fs-sm); padding: 0 var(--s1); }
  .opt { flex-direction: row; align-items: center; gap: var(--s2); color: var(--text); font-size: var(--fs-md); min-height: 32px; cursor: pointer; }
  .phone .opt { min-height: var(--touch); }
  .phone input[type='text'], .phone input[type='password'] { height: var(--touch); }
</style>
