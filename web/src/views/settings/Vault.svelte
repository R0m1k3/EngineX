<script lang="ts">
  import { stamp } from '../../lib/ui/format';
  import KeyRound from '@lucide/svelte/icons/key-round';
  import Trash2 from '@lucide/svelte/icons/trash-2';
  import Button from '../../lib/ui/Button.svelte';
  import Input from '../../lib/ui/Input.svelte';
  import Select from '../../lib/ui/Select.svelte';
  import Badge from '../../lib/ui/Badge.svelte';
  import EmptyState from '../../lib/ui/EmptyState.svelte';
  import { confirmDialog } from '../../lib/ui/dialog.svelte';
  import { toast } from '../../lib/ui/toast.svelte';
  import Section from './Section.svelte';
  import Note from './Note.svelte';
  import {
    vault, putSecret, deleteSecret, projects as loadProjects, message, type Secret, type Project,
  } from '../../lib/api';

  let secrets = $state<Secret[]>([]);
  let locked = $state(false);
  let projects = $state<Project[]>([]);
  let error = $state('');
  let scope = $state('global');
  let name = $state('');
  let value = $state('');

  async function load() {
    try {
      const v = await vault();
      secrets = v.secrets;
      locked = v.locked;
      projects = await loadProjects();
      error = '';
    } catch (e) {
      error = message(e);
    }
  }

  const scopeLabel = (s: string) => {
    if (s === 'global') return 'Tous les projets';
    const id = Number(s.replace('project:', ''));
    return `Projet ${projects.find((p) => p.id === id)?.name ?? id}`;
  };

  async function save(e: SubmitEvent) {
    e.preventDefault();
    error = '';
    try {
      await putSecret(scope, name.trim(), value);
      toast(`${name.trim()} enregistré.`, 'ok');
      name = '';
      value = '';
      await load();
    } catch (err) {
      error = message(err);
    }
  }

  async function remove(s: Secret) {
    const ok = await confirmDialog(
      `Supprimer ${s.name} (${scopeLabel(s.scope)}) ? Une sandbox déjà démarrée le garde jusqu’à son arrêt.`,
      { title: 'Supprimer le secret', confirm: 'Supprimer', danger: true },
    );
    if (!ok) return;
    try {
      await deleteSecret(s.id);
      toast(`${s.name} supprimé.`, 'ok');
      await load();
    } catch (err) {
      error = message(err);
    }
  }

  const date = stamp;
  const scopes = $derived([
    { value: 'global', label: 'Tous les projets' },
    ...projects.map((p) => ({ value: `project:${p.id}`, label: `Projet ${p.name}` })),
  ]);
  load();
</script>

{#if locked || error}
  <div class="err">
    {#if locked}
      <Note tone="danger">
        Coffre verrouillé : le fichier <code>/data/master.key</code> manque ou ne correspond pas aux secrets enregistrés.
        Restaurez-le depuis une sauvegarde, ou supprimez les secrets pour repartir de zéro.
      </Note>
    {/if}
    {#if error}<Note tone="danger">{error}</Note>{/if}
  </div>
{/if}

<Section title="Ajouter un secret" text="Chaque secret devient une variable d’environnement dans les sandboxes.">
  <p class="hint">
    Les commandes de Codex le lisent avec <code>$NOM</code>. Il est chiffré sur le disque et remplacé par <code>***</code>
    dans l’historique et dans ce que voit le modèle. Les valeurs de moins de 6 caractères ne sont pas masquées.
  </p>
  <form class="form" onsubmit={save}>
    <Select label="Portée" bind:value={scope} options={scopes} />
    <Input label="Nom" class="mono-in" bind:value={name} placeholder="GITHUB_TOKEN" pattern="[A-Z_][A-Z0-9_]*" maxlength={64} required />
    <Input label="Valeur" type="password" bind:value={value} autocomplete="new-password" maxlength={16384} required />
    <div class="submit"><Button type="submit" variant="primary" disabled={locked || !name.trim() || !value}>Enregistrer</Button></div>
  </form>
  <p class="hint">
    Un nom déjà pris dans la même portée remplace l’ancienne valeur. Un secret de projet l’emporte sur un secret global
    de même nom. Une sandbox déjà démarrée garde ses variables jusqu’à son arrêt : un ajout ou une modification compte
    au démarrage suivant.
  </p>
</Section>

<Section title="Secrets enregistrés" text="Les valeurs ne sont jamais réaffichées.">
  {#if secrets.length}
    <ul class="rows" aria-label="Secrets">
      {#each secrets as s (s.id)}
        <li>
          <div class="rmain">
            <span class="mono rname">{s.name}</span>
            <span class="rmeta">Modifié le {date(s.updated_at)}</span>
          </div>
          <Badge>{scopeLabel(s.scope)}</Badge>
          <Button variant="ghost" icon={Trash2} label="Supprimer {s.name}" onclick={() => remove(s)} />
        </li>
      {/each}
    </ul>
  {:else}
    <EmptyState icon={KeyRound} title="Aucun secret" text="Les jetons GitHub, Gitea ou d’un service ajoutés ici seront lisibles par les commandes des sandboxes." />
  {/if}
</Section>

<style>
  .err { display: flex; flex-direction: column; gap: var(--s2); padding: var(--s4) var(--s6) 0; }
  .hint { margin: 0; color: var(--muted); font-size: var(--fs-sm); line-height: 1.5; }
  .form { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: var(--s4); }
  .form :global(.mono-in input) { font-family: var(--mono); }
  .submit { grid-column: 1 / -1; }
  .rows { list-style: none; margin: 0; padding: 0; border-top: 1px solid var(--line); }
  .rows li { display: flex; align-items: center; gap: var(--s3); padding: var(--s2) 0; border-bottom: 1px solid var(--line); }
  .rmain { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .rname { font-weight: 600; overflow-wrap: anywhere; }
  .rmeta { color: var(--muted); font-size: var(--fs-sm); }
  @media (max-width: 767px) {
    .err { padding: var(--s4) var(--s4) 0; }
    .form { grid-template-columns: minmax(0, 1fr); }
    .form :global(.box), .form :global(.sel), .form :global(.btn) { height: var(--touch); }
    .rows li { flex-wrap: wrap; }
    .rows :global(.btn) { width: var(--touch); height: var(--touch); }
  }
</style>
