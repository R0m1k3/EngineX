<script lang="ts">
  // « Nouvelle session » : choix du topic (et de la copie séparée si le topic est un dépôt git).
  import { createSession, message, type Project } from '../../lib/api';
  import Dialog from '../../lib/ui/Dialog.svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Select from '../../lib/ui/Select.svelte';
  import Switch from '../../lib/ui/Switch.svelte';
  import { confirmDialog } from '../../lib/ui/dialog.svelte';
  import { store } from './phone.svelte';

  let { open = $bindable(false), projects, topic = null, created }: {
    open?: boolean;
    projects: Project[];
    /** Topic proposé (celui de la session ouverte, sinon le dernier utilisé). */
    topic?: number | null;
    created: (id: string) => void;
  } = $props();

  let chosen = $state(0);
  let copy = $state(false);
  let busy = $state(false);
  let error = $state('');

  const options = $derived(
    projects.flatMap((p) => p.topics.map((t) => ({ value: t.id, label: `${p.name} / ${t.name}` }))),
  );
  const gitTopic = $derived(projects.flatMap((p) => p.topics).find((t) => t.id === chosen)?.git ?? false);

  $effect(() => {
    if (!open) return;
    const ids = options.map((o) => o.value);
    let last = 0;
    try {
      last = Number(localStorage.getItem('enginex.sessions.topic')) || 0;
    } catch {
      /* rien */
    }
    chosen = [topic, last].find((t) => t != null && ids.includes(t)) ?? ids[0] ?? 0;
    copy = false;
    error = '';
  });

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    if (!chosen || busy) return;
    if (
      copy &&
      !(await confirmDialog(
        'La session travaillera sur sa propre branche, dans une copie du dépôt git de ce topic. Les modifications non commitées du topic ne sont pas copiées.',
        { title: 'Créer une copie séparée ?', confirm: 'Créer la copie' },
      ))
    )
      return;
    busy = true;
    error = '';
    try {
      const s = await createSession(chosen, 'auto', copy);
      store('enginex.sessions.topic', chosen);
      open = false;
      created(s.id);
    } catch (err) {
      error = message(err);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog bind:open title="Nouvelle session" width="460px">
  <form id="new-session" class="form" onsubmit={submit}>
    {#if options.length}
      <Select label="Topic" bind:value={chosen} options={options} size="lg" />
      {#if gitTopic}
        <Switch bind:checked={copy}>Copie séparée du dépôt git (branche à part)</Switch>
      {/if}
      <p class="muted small">La sandbox démarre au premier message.</p>
    {:else}
      <p class="muted">Créez d’abord un projet et un topic dans la liste des sessions.</p>
    {/if}
    {#if error}<p class="error small" role="alert">{error}</p>{/if}
  </form>
  {#snippet actions()}
    <Button onclick={() => (open = false)}>Annuler</Button>
    <Button variant="primary" type="submit" form="new-session" disabled={!chosen} {busy}>
      {busy && copy ? 'Création de la copie…' : 'Créer la session'}
    </Button>
  {/snippet}
</Dialog>

<style>
  .form { display: flex; flex-direction: column; gap: var(--s4); }
  .form p { margin: 0; }
</style>
