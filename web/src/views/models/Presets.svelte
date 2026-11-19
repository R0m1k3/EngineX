<script lang="ts">
  import { onDestroy } from 'svelte';
  import Layers from '@lucide/svelte/icons/layers';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import ArrowLeft from '@lucide/svelte/icons/arrow-left';
  import Copy from '@lucide/svelte/icons/copy';
  import RefreshCw from '@lucide/svelte/icons/refresh-cw';
  import Trash2 from '@lucide/svelte/icons/trash-2';
  import Power from '@lucide/svelte/icons/power';
  import Icon from '../../lib/ui/Icon.svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Badge from '../../lib/ui/Badge.svelte';
  import StatusDot from '../../lib/ui/StatusDot.svelte';
  import EmptyState from '../../lib/ui/EmptyState.svelte';
  import { confirmDialog, promptDialog } from '../../lib/ui/dialog.svelte';
  import { toast } from '../../lib/ui/toast.svelte';
  import Note from '../settings/Note.svelte';
  import {
    presets, models, activatePreset, copyPreset, deletePreset, refreshSources, ApiError, message,
    type Preset, type LocalModel,
  } from '../../lib/api';
  import PresetEditor from './PresetEditor.svelte';

  let list = $state<Preset[]>([]);
  let mods = $state<LocalModel[]>([]);
  let open = $state<number | null>(null);
  /** Choisi à la main (au téléphone : la fiche remplace la liste). */
  let picked = $state(false);
  let loaded = $state(false);
  let error = $state('');

  async function load() {
    try {
      [list, mods] = await Promise.all([presets(), models()]);
      error = '';
      // Sélection par défaut : le preset chargé, sinon le premier.
      if (open === null || !list.some((p) => p.id === open)) open = (list.find((p) => p.active) ?? list[0])?.id ?? null;
    } catch (e) {
      error = message(e);
    } finally {
      loaded = true;
    }
  }

  async function run(fn: () => Promise<unknown>, ok = '') {
    error = '';
    try {
      await fn();
      if (ok) toast(ok, 'ok');
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
    try {
      await activatePreset(p.id);
      toast(started, 'ok');
    } catch (e) {
      if (e instanceof ApiError && e.code === 'nofit_confirm_required') {
        const ok = await confirmDialog(
          'Ce preset ne tient pas en VRAM : llama.cpp mettra des couches sur le processeur, beaucoup plus lent. Activer quand même ?',
          { title: 'Activer sans place en VRAM', confirm: 'Activer quand même' },
        );
        if (!ok) return;
        await run(() => activatePreset(p.id, true), started);
        return;
      }
      error = message(e);
      return;
    }
    await load();
  }

  // Dupliquer : nom demandé dans une modale (remplace prompt()).
  async function duplicate(p: Preset) {
    const name = await promptDialog('Nom de la copie', { title: 'Dupliquer le preset', value: `${p.name} (copie)`, confirm: 'Dupliquer', maxlength: 120 });
    if (name) await run(() => copyPreset(p.id, name), `Copie « ${name} » créée.`);
  }

  async function remove(p: Preset) {
    const ok = await confirmDialog(
      `Supprimer le preset ${p.name} ?${p.active ? ' Le moteur sera arrêté après les requêtes en cours.' : ''}`,
      { title: 'Supprimer le preset', confirm: 'Supprimer', danger: true },
    );
    if (ok) await run(() => deletePreset(p.id), p.active ? 'Suppression lancée : le moteur s’arrête après les requêtes en cours.' : 'Preset supprimé.');
  }

  const modelOf = (p: Preset) => mods.find((m) => m.id === p.model_id);
  const k = (n: number) => `${Math.round(n / 1024)}k`;
  const draftLabel = (p: Preset) => {
    const d = p.settings.spec.draft;
    const parts = [d === 'mtp' ? 'MTP' : d === 'model' ? 'petit modèle' : '', p.settings.spec.ngram ? 'ngram' : ''];
    return parts.filter(Boolean).join(', ');
  };
  const meta = (p: Preset) => [k(p.settings.ctx), `KV ${p.settings.kv_type}`, draftLabel(p)].filter(Boolean).join(', ');
  let current = $derived(list.find((p) => p.id === open) ?? null);

  function pick(id: number) {
    open = id;
    picked = true;
  }

  load();
  // Relu régulièrement : une activation ou une suppression finit en fond.
  const timer = setInterval(load, 3000);
  onDestroy(() => clearInterval(timer));
</script>

{#if error}<div class="err"><Note tone="danger">{error}</Note></div>{/if}

{#if loaded && list.length === 0}
  <div class="empty">
    <EmptyState icon={Layers} title="Aucun preset" text="Un preset est créé à la fin de chaque téléchargement de modèle.">
      {#snippet action()}<Button href="#models">Chercher un modèle</Button>{/snippet}
    </EmptyState>
  </div>
{:else if list.length}
  <div class="split" class:picked>
    <div class="col">
    <ul class="list" aria-label="Presets">
      {#each list as p (p.id)}
        {@const m = modelOf(p)}
        <li>
          <button type="button" class="item" class:sel={p.id === open} aria-current={p.id === open ? 'true' : undefined} onclick={() => pick(p.id)}>
            <span class="iname">
              {#if p.active}<StatusDot kind="ok" label="Chargé" />{/if}
              <span class="ell">{p.name}</span>
            </span>
            <span class="imeta ell">{meta(p)}</span>
            {#if m}<span class="imeta ell faint">{m.repo}</span>{/if}
            {#if p.active || p.default || p.sources.error}
              <span class="tags">
                {#if p.active}<Badge tone="ok">Chargé</Badge>{/if}
                {#if p.default}<Badge>Par défaut</Badge>{/if}
                {#if p.sources.error}<Badge tone="danger">Sources non lues</Badge>{/if}
              </span>
            {/if}
            <span class="chev"><Icon icon={ChevronRight} size={18} /></span>
          </button>
        </li>
      {/each}
    </ul>
    </div>

    <div class="detail">
      {#if current}
        {@const p = current}
        <a class="back" href="#models/presets" onclick={(e) => { e.preventDefault(); picked = false; }}>
          <Icon icon={ArrowLeft} size={18} />Presets
        </a>
        {#key p.id}
          <PresetEditor preset={p} model={modelOf(p)} models={mods} saved={load}>
            {#snippet actions()}
              <Button icon={Power} onclick={() => activate(p)}>{p.active ? 'Relancer' : 'Activer'}</Button>
              <Button variant="ghost" icon={Copy} label="Dupliquer" onclick={() => duplicate(p)} />
              <Button variant="ghost" icon={RefreshCw} label="Relire les sources" onclick={() => run(() => refreshSources(p.id), 'Sources relues.')} />
              <Button variant="ghost" icon={Trash2} label="Supprimer" onclick={() => remove(p)} />
            {/snippet}
          </PresetEditor>
        {/key}
      {/if}
    </div>
  </div>
{/if}


<style>
  .err { padding: var(--s3) var(--s6) 0; }
  .empty { padding: var(--s6); }
  .split { flex: 1; display: grid; grid-template-columns: 340px minmax(0, 1fr); min-height: 0; }
  .col { background: var(--panel); border-right: 1px solid var(--line); min-width: 0; }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    overflow: auto;
    position: sticky;
    top: 0;
    max-height: calc(100dvh - var(--status-h));
  }
  .item {
    position: relative;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 2px;
    width: 100%;
    min-height: 0;
    padding: var(--s3) var(--s4) var(--s3) var(--s5);
    border: 0;
    border-bottom: 1px solid var(--line);
    border-radius: 0;
    background: none;
    color: var(--text);
    text-align: left;
    font: inherit;
    font-weight: 400;
    cursor: pointer;
    filter: none;
  }
  @media (hover: hover) { .item:hover { background: var(--raised); } }
  .item.sel { background: var(--accent-soft); }
  .item.sel::before { content: ''; position: absolute; left: 0; top: 0; bottom: 0; width: 2px; background: var(--accent); }
  .iname { display: flex; align-items: center; gap: var(--s2); font-size: var(--fs-md); font-weight: 600; min-width: 0; }
  .imeta { color: var(--muted); font-size: var(--fs-sm); }
  .imeta.faint { color: var(--faint); }
  .tags { display: flex; gap: var(--s2); flex-wrap: wrap; margin-top: var(--s2); }
  .chev { display: none; }
  .detail { min-width: 0; }
  .back { display: none; }

  @media (max-width: 1023px) {
    .split { grid-template-columns: 280px minmax(0, 1fr); }
  }
  @media (max-width: 767px) {
    .err { padding: var(--s3) var(--s4) 0; }
    .empty { padding: var(--s4); }
    .split { display: block; }
    .col { border-right: 0; background: var(--bg); }
    .list { position: static; max-height: none; }
    .item { grid-template-columns: minmax(0, 1fr) auto; min-height: 60px; padding: var(--s3) var(--s3) var(--s3) var(--s4); }
    .item > :not(.chev) { grid-column: 1; }
    .chev { display: flex; grid-column: 2; grid-row: 1 / span 4; align-self: center; color: var(--faint); }
    .item.sel { background: none; }
    .item.sel::before { display: none; }
    .split:not(.picked) .detail { display: none; }
    .split.picked .col { display: none; }
    .back {
      display: flex;
      align-items: center;
      gap: var(--s2);
      height: var(--touch);
      padding: 0 var(--s4);
      color: var(--muted);
      text-decoration: none;
      border-bottom: 1px solid var(--line);
    }
  }
</style>
