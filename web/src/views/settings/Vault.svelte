<script lang="ts">
  import {
    vault, putSecret, deleteSecret, projects as loadProjects, message, type Secret, type Project,
  } from '../../lib/api';

  let secrets = $state<Secret[]>([]);
  let locked = $state(false);
  let projects = $state<Project[]>([]);
  let error = $state('');
  let saved = $state('');
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
    saved = '';
    try {
      await putSecret(scope, name.trim(), value);
      saved = `${name.trim()} enregistré.`;
      name = '';
      value = '';
      await load();
    } catch (err) {
      error = message(err);
    }
  }

  async function remove(s: Secret) {
    if (!confirm(`Supprimer ${s.name} (${scopeLabel(s.scope)}) ? Une sandbox déjà démarrée le garde jusqu’à son arrêt.`)) return;
    try {
      await deleteSecret(s.id);
      await load();
    } catch (err) {
      error = message(err);
    }
  }

  const date = (t: number) => new Date(t * 1000).toLocaleString('fr-FR');
  load();
</script>

{#if locked}
  <p class="banner">
    Coffre verrouillé : le fichier <code>/data/master.key</code> manque ou ne correspond pas aux secrets enregistrés.
    Restaurez-le depuis une sauvegarde, ou supprimez les secrets pour repartir de zéro.
  </p>
{/if}
{#if error}<p class="error">{error}</p>{/if}

<section class="card">
  <h3>Clés et secrets</h3>
  <p class="small muted">
    Chaque secret devient une variable d’environnement dans les sandboxes (les commandes de Codex le lisent avec
    <code>$NOM</code>). Il est chiffré sur le disque et remplacé par <code>***</code> dans l’historique et dans ce que
    voit le modèle. Les valeurs de moins de 6 caractères ne sont pas masquées.
  </p>
  <form class="fields" onsubmit={save}>
    <label>Portée
      <select bind:value={scope}>
        <option value="global">Tous les projets</option>
        {#each projects as p (p.id)}<option value="project:{p.id}">Projet {p.name}</option>{/each}
      </select>
    </label>
    <label>Nom <input bind:value={name} placeholder="GITHUB_TOKEN" pattern="[A-Z_][A-Z0-9_]*" maxlength="64" required /></label>
    <label>Valeur <input type="password" bind:value={value} autocomplete="new-password" maxlength="16384" required /></label>
    <div class="actions"><button type="submit" disabled={locked || !name.trim() || !value}>Enregistrer</button></div>
  </form>
  <p class="small muted">Un nom déjà pris dans la même portée remplace l’ancienne valeur. Un secret de projet l’emporte sur un secret global de même nom. Une sandbox déjà démarrée garde ses variables jusqu’à son arrêt : un ajout ou une modification compte au démarrage suivant.</p>
  {#if saved}<p class="small ok">{saved}</p>{/if}
  {#if secrets.length}
    <table>
      <thead><tr><th>Nom</th><th>Portée</th><th>Modifié le</th><th></th></tr></thead>
      <tbody>
        {#each secrets as s (s.id)}
          <tr>
            <td><code>{s.name}</code></td>
            <td class="small">{scopeLabel(s.scope)}</td>
            <td class="small">{date(s.updated_at)}</td>
            <td class="actions"><button class="link" onclick={() => remove(s)}>Supprimer</button></td>
          </tr>
        {/each}
      </tbody>
    </table>
  {:else}
    <p class="muted small">Aucun secret.</p>
  {/if}
</section>

<style>
  .banner { border: 1px solid var(--error); color: var(--error); border-radius: var(--radius); padding: var(--s2) var(--s3); }
</style>
