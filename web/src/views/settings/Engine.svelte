<script lang="ts">
  import ProgressBar from '../../lib/ui/ProgressBar.svelte';
  import { onDestroy } from 'svelte';
  import {
    versions, engine, checkVersion, installVersion, useVersion, rollbackVersion, deleteVersion, setAutoUpdate,
    message, size, jobError, JOB_KIND, JOB_STEP,
    type Versions, type EngineInfo,
  } from '../../lib/api';

  let v = $state<Versions | null>(null);
  let info = $state<EngineInfo | null>(null);
  let error = $state('');
  let checking = $state(false);

  async function load() {
    try {
      [v, info] = await Promise.all([versions(), engine()]);
      error = '';
    } catch (e) {
      error = message(e);
    }
  }

  async function run(fn: () => Promise<unknown>) {
    error = '';
    try {
      await fn();
      await load();
    } catch (e) {
      error = message(e);
    }
  }

  async function check() {
    checking = true;
    await run(checkVersion);
    checking = false;
  }

  const date = (t: number) => new Date(t * 1000).toLocaleString('fr-FR');
  let running = $derived(!!v?.job && !v.job.finished);
  // Même build que l'image ou qu'une version installée : rien à installer.
  let newer = $derived(!!v?.latest && !v.installed.some((i) => i.tag === v?.latest?.tag || i.build === v?.latest?.build));

  load();
  const timer = setInterval(load, 1500);
  onDestroy(() => clearInterval(timer));
</script>

{#if error}<p class="error">{error}</p>{/if}

{#if v && info}
  <section class="card">
    <h3>Mise à jour automatique</h3>
    <label class="check">
      <input
        type="checkbox"
        checked={info.auto_update}
        disabled={v.env_override}
        onchange={(e) => run(() => setAutoUpdate((e.currentTarget as HTMLInputElement).checked))}
      />
      Installer les nouvelles versions de llama.cpp toute seule, quand le modèle est au repos (10 min sans requête)
    </label>
    {#if info.pending}
      <p class="small">{info.pending.build} en attente de repos depuis le {date(info.pending.since)}</p>
    {/if}
    {#if info.auto_last}
      <p class="small {info.auto_last.ok ? 'ok' : 'error'}">
        {info.auto_last.tag.replace('server-cuda-', '')}
        {info.auto_last.ok ? `installée le ${date(info.auto_last.at)}` : `écartée le ${date(info.auto_last.at)} : ${jobError(info.auto_last.message)}`}
      </p>
    {/if}
    {#if info.skipped.length}
      <p class="small muted">Versions écartées (plus retentées seules ; une installation à la main les remet en jeu) : {info.skipped.join(', ')}</p>
    {/if}
  </section>

  <section class="card">
    <h3>llama.cpp</h3>
    {#if v.env_override}<p class="error small">Binaire imposé par ENGINEX_LLAMA_BIN : les boutons de changement sont désactivés.</p>{/if}
    <p>Actif : <strong>{v.active}</strong>{#if v.previous}<span class="muted"> · précédent : {v.previous}</span>{/if}</p>
    <p>
      Dernière publiée :
      {#if v.latest}<strong>{v.latest.tag}</strong>{#if !newer}<span class="muted"> (installée)</span>{/if}{:else}<span class="muted">non vérifiée</span>{/if}
    </p>
    <div class="actions">
      <button class="secondary" onclick={check} disabled={checking}>Vérifier</button>
      {#if newer}<button onclick={() => run(() => installVersion(v?.latest?.tag))} disabled={running || v.env_override}>Installer {v.latest?.build}</button>{/if}
      {#if v.previous}<button class="secondary" onclick={() => run(rollbackVersion)} disabled={running || v.env_override}>Revenir à la précédente</button>{/if}
    </div>
    {#if v.job}
      <div class="job">
        <p class="small">
          {JOB_KIND[v.job.kind]}{v.job.tag && ['install', 'use', 'rollback', 'auto_update'].includes(v.job.kind) ? ` de ${v.job.tag}` : ''} :
          <span class="state state-{v.job.step}">{JOB_STEP[v.job.step]}</span>
          {#if v.job.step === 'download' && v.job.total_bytes}<span class="muted"> · {size(v.job.done_bytes)} / {size(v.job.total_bytes)}</span>{/if}
        </p>
        {#if v.job.step === 'download' && v.job.total_bytes}
          <div class="pb"><ProgressBar value={v.job.done_bytes} max={v.job.total_bytes} label="Téléchargement" /></div>
        {/if}
        {#if v.job.error}<p class="error small">{jobError(v.job.error)}</p>{/if}
        {#if v.job.message}<p class="ok small">{v.job.message}</p>{/if}
      </div>
    {/if}
  </section>

  <table>
    <thead><tr><th>Version</th><th>Build</th><th>Installée le</th><th>Dernier test</th><th></th></tr></thead>
    <tbody>
      {#each v.installed as i (i.tag)}
        <tr>
          <td>{i.tag}{#if i.tag === v.active} <span class="badge">actif</span>{/if}</td>
          <td>{i.build}</td>
          <td class="small">{date(i.installed_at)}</td>
          <td class="small">
            {#if i.last_test?.ok}<span class="ok">OK</span><span class="muted"> · {i.last_test.gpus} carte(s){i.last_test.has_fit ? ' · --fit' : ' · sans --fit'}</span>
            {:else if i.last_test}<span class="error">{i.last_test.message ?? i.last_test.code}</span>
            {:else}<span class="muted">—</span>{/if}
          </td>
          <td class="actions">
            {#if i.tag !== v.active}
              <button class="secondary" onclick={() => run(() => useVersion(i.tag))} disabled={running || v.env_override}>Utiliser</button>
              {#if i.tag !== 'image'}<button class="link" onclick={() => run(() => deleteVersion(i.tag))} disabled={running}>Supprimer</button>{/if}
            {/if}
          </td>
        </tr>
      {/each}
    </tbody>
  </table>
{/if}

<style>
  .pb { margin-bottom: var(--s2); }
</style>
