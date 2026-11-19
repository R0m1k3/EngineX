<script lang="ts">
  import { int, octets } from '../../lib/ui/format';
  import HardDrive from '@lucide/svelte/icons/hard-drive';
  import Trash2 from '@lucide/svelte/icons/trash-2';
  import Button from '../../lib/ui/Button.svelte';
  import Badge from '../../lib/ui/Badge.svelte';
  import EmptyState from '../../lib/ui/EmptyState.svelte';
  import { confirmDialog } from '../../lib/ui/dialog.svelte';
  import { toast } from '../../lib/ui/toast.svelte';
  import Note from '../settings/Note.svelte';
  import { models, deleteModel, message, type LocalModel } from '../../lib/api';

  let list = $state<LocalModel[]>([]);
  let loaded = $state(false);
  let error = $state('');

  async function load() {
    try {
      list = await models();
      error = '';
    } catch (e) {
      error = message(e);
    } finally {
      loaded = true;
    }
  }

  async function remove(m: LocalModel) {
    const n = m.presets.length;
    const ok = await confirmDialog(`Supprimer ${m.file} ?${n ? ` Supprime aussi ${n} preset${n > 1 ? 's' : ''}.` : ''}`, {
      title: 'Supprimer le modèle',
      confirm: 'Supprimer',
      danger: true,
    });
    if (!ok) return;
    try {
      await deleteModel(m.id);
      toast(`${m.file} supprimé.`, 'ok');
      await load();
    } catch (e) {
      error = message(e);
    }
  }

  load();
</script>

<div class="loc">
  {#if error}<Note tone="danger">{error}</Note>{/if}
  {#if loaded && list.length === 0}
    <EmptyState icon={HardDrive} title="Aucun modèle local" text="Les fichiers GGUF téléchargés depuis Hugging Face apparaissent ici.">
      {#snippet action()}<Button href="#models">Chercher un modèle</Button>{/snippet}
    </EmptyState>
  {/if}

  {#if list.length}
    <table class="tbl">
      <thead>
        <tr><th>Modèle</th><th>Architecture</th><th class="r">Taille</th><th class="r">Contexte max</th><th>Vision</th><th><span class="sr-only">Actions</span></th></tr>
      </thead>
      <tbody>
        {#each list as m (m.id)}
          <tr>
            <td>
              <div class="name">
                <strong>{m.repo}</strong>
                <span class="file">{m.file}{m.shards.length > 1 ? ` (${m.shards.length} tranches)` : ''}</span>
              </div>
            </td>
            <td class="mono" data-l="Architecture">{m.meta.arch}</td>
            <td class="r num" data-l="Taille">{octets(m.size_bytes)}</td>
            <td class="r num" data-l="Contexte max">{m.meta.context_length != null ? int(m.meta.context_length) : '?'}</td>
            <td data-l="Vision">{#if m.mmproj}<Badge tone="ok">oui</Badge>{:else}<span class="faint">non</span>{/if}</td>
            <td class="act"><Button variant="ghost" icon={Trash2} label="Supprimer {m.file}" onclick={() => remove(m)} /></td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

<style>
  .loc { display: flex; flex-direction: column; gap: var(--s4); padding: var(--s6); max-width: 1100px; }
  .tbl { width: 100%; border-collapse: collapse; font-size: var(--fs-sm); }
  .tbl th { text-align: left; font-weight: 400; color: var(--muted); padding: 0 var(--s4) var(--s2) 0; border-bottom: 1px solid var(--line); white-space: nowrap; }
  .tbl td { padding: var(--s3) var(--s4) var(--s3) 0; border-bottom: 1px solid var(--line); vertical-align: middle; }
  .tbl tbody tr:hover { background: none; }
  .r { text-align: right !important; }
  .name { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .name strong { font-size: var(--fs-md); font-weight: 600; overflow-wrap: anywhere; }
  .file { color: var(--muted); font-family: var(--mono); overflow-wrap: anywhere; }
  .act { width: 1%; padding-right: 0 !important; }
  .faint { color: var(--faint); }

  @media (max-width: 767px) {
    .loc { padding: var(--s4); }
    .tbl thead { display: none; }
    .tbl, .tbl tbody, .tbl tr, .tbl td { display: block; width: auto; }
    .tbl tr { position: relative; padding: var(--s3) 0; border-bottom: 1px solid var(--line); }
    .tbl td { border: 0; padding: 2px 52px 2px 0; text-align: left !important; }
    .tbl td[data-l] { display: flex; justify-content: space-between; gap: var(--s3); color: var(--text); }
    .tbl td[data-l]::before { content: attr(data-l); color: var(--muted); }
    .act { position: absolute; top: var(--s2); right: 0; padding: 0 !important; }
    .act :global(.btn) { width: var(--touch); height: var(--touch); }
  }
</style>
