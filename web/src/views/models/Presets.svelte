<script lang="ts">
  import { onDestroy } from 'svelte';
  import {
    presets, models, activatePreset, copyPreset, deletePreset, refreshSources, ApiError, message,
    type Preset, type LocalModel,
  } from '../../lib/api';
  import PresetEditor from './PresetEditor.svelte';

  let list = $state<Preset[]>([]);
  let mods = $state<LocalModel[]>([]);
  let open = $state<number | null>(null);
  let error = $state('');
  let notice = $state('');

  async function load() {
    try {
      [list, mods] = await Promise.all([presets(), models()]);
      error = '';
    } catch (e) {
      error = message(e);
    }
  }

  async function run(fn: () => Promise<unknown>, ok = '') {
    error = '';
    notice = '';
    try {
      await fn();
      notice = ok;
      await load();
    } catch (e) {
      error = message(e);
    }
  }

  // L'activation est une tâche en fond (elle attend la fin des requêtes
  // /v1 en cours) : son avancement s'affiche dans la barre du moteur.
  const started = 'Chargement lancé : avancement dans la barre du moteur.';

  async function activate(p: Preset) {
    error = '';
    notice = '';
    try {
      await activatePreset(p.id);
      notice = started;
    } catch (e) {
      if (e instanceof ApiError && e.code === 'nofit_confirm_required') {
        if (!confirm('Ce preset ne tient pas en VRAM : llama.cpp mettra des couches sur le processeur, beaucoup plus lent. Activer quand même ?')) return;
        await run(() => activatePreset(p.id, true), started);
        return;
      }
      error = message(e);
      return;
    }
    await load();
  }

  async function duplicate(p: Preset) {
    const name = prompt('Nom de la copie', `${p.name} (copie)`);
    if (name) await run(() => copyPreset(p.id, name));
  }

  async function remove(p: Preset) {
    if (confirm(`Supprimer le preset ${p.name} ?${p.active ? ' Le moteur sera arrêté après les requêtes en cours.' : ''}`))
      await run(() => deletePreset(p.id), p.active ? 'Suppression lancée : le moteur s’arrête après les requêtes en cours.' : '');
  }

  const modelOf = (p: Preset) => mods.find((m) => m.id === p.model_id);

  load();
  // Relu régulièrement : une activation ou une suppression finit en fond.
  const timer = setInterval(load, 3000);
  onDestroy(() => clearInterval(timer));
</script>

{#if error}<p class="error">{error}</p>{/if}
{#if notice}<p class="ok">{notice}</p>{/if}
{#if list.length === 0}<p class="muted">Aucun preset : un preset est créé à la fin de chaque téléchargement.</p>{/if}

{#each list as p (p.id)}
  <article class="card">
    <div class="row wrap">
      <div>
        <strong>{p.name}</strong>
        {#if p.active}<span class="badge">actif</span>{/if}
        <span class="muted small"> · {modelOf(p)?.repo ?? '?'} · contexte {p.settings.ctx.toLocaleString('fr-FR')} · KV {p.settings.kv_type}</span>
        {#if p.sources.error}<span class="error small"> · sources non lues</span>{/if}
      </div>
      <div class="actions">
        <button onclick={() => activate(p)}>{p.active ? 'Relancer' : 'Activer'}</button>
        <button class="secondary" onclick={() => (open = open === p.id ? null : p.id)}>{open === p.id ? 'Fermer' : 'Modifier'}</button>
        <button class="link" onclick={() => duplicate(p)}>Dupliquer</button>
        <button class="link" onclick={() => run(() => refreshSources(p.id), 'Sources relues.')}>Relire les sources</button>
        <button class="link" onclick={() => remove(p)}>Supprimer</button>
      </div>
    </div>
    {#if open === p.id}
      <PresetEditor preset={p} model={modelOf(p)} saved={load} />
    {/if}
  </article>
{/each}
