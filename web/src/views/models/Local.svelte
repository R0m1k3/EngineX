<script lang="ts">
  import { models, deleteModel, message, size, type LocalModel } from '../../lib/api';

  let list = $state<LocalModel[]>([]);
  let error = $state('');

  async function load() {
    try {
      list = await models();
      error = '';
    } catch (e) {
      error = message(e);
    }
  }

  async function remove(m: LocalModel) {
    const n = m.presets.length;
    if (!confirm(`Supprimer ${m.file} ?${n ? ` Supprime aussi ${n} preset${n > 1 ? 's' : ''}.` : ''}`)) return;
    try {
      await deleteModel(m.id);
      await load();
    } catch (e) {
      error = message(e);
    }
  }

  load();
</script>

{#if error}<p class="error">{error}</p>{/if}
{#if list.length === 0}<p class="muted">Aucun modèle local.</p>{/if}

{#if list.length}
  <table>
    <thead><tr><th>Modèle</th><th>Architecture</th><th>Taille</th><th>Contexte max</th><th>Vision</th><th></th></tr></thead>
    <tbody>
      {#each list as m (m.id)}
        <tr>
          <td><strong>{m.repo}</strong><br /><span class="muted small">{m.file}{m.shards.length > 1 ? ` (${m.shards.length} tranches)` : ''}</span></td>
          <td>{m.meta.arch}</td>
          <td>{size(m.size_bytes)}</td>
          <td>{m.meta.context_length?.toLocaleString('fr-FR') ?? '?'}</td>
          <td>{m.mmproj ? 'oui' : 'non'}</td>
          <td><button class="link" onclick={() => remove(m)}>Supprimer</button></td>
        </tr>
      {/each}
    </tbody>
  </table>
{/if}
