<script lang="ts">
  import { octets, stamp } from '../../lib/ui/format';
  import { onDestroy } from 'svelte';
  import RefreshCw from '@lucide/svelte/icons/refresh-cw';
  import Download from '@lucide/svelte/icons/download';
  import Undo2 from '@lucide/svelte/icons/undo-2';
  import Trash2 from '@lucide/svelte/icons/trash-2';
  import Button from '../../lib/ui/Button.svelte';
  import Switch from '../../lib/ui/Switch.svelte';
  import Badge from '../../lib/ui/Badge.svelte';
  import ProgressBar from '../../lib/ui/ProgressBar.svelte';
  import Section from './Section.svelte';
  import Note from './Note.svelte';
  import {
    versions, engine, checkVersion, installVersion, useVersion, rollbackVersion, deleteVersion, setAutoUpdate,
    message, jobError, JOB_KIND, JOB_STEP,
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

  const date = stamp;
  let running = $derived(!!v?.job && !v.job.finished);
  // Même build que l'image ou qu'une version installée : rien à installer.
  let newer = $derived(!!v?.latest && !v.installed.some((i) => i.tag === v?.latest?.tag || i.build === v?.latest?.build));

  load();
  const timer = setInterval(load, 1500);
  onDestroy(() => clearInterval(timer));
</script>

{#if error}<div class="err"><Note tone="danger">{error}</Note></div>{/if}

{#if v && info}
  <Section title="Mise à jour automatique" text="Une nouvelle version n’est installée qu’au repos : 10 min sans requête.">
    <Switch checked={info.auto_update} disabled={v.env_override} onchange={(c) => run(() => setAutoUpdate(c))}>
      Installer les nouvelles versions de llama.cpp toute seule
    </Switch>
    {#if info.pending}
      <p class="line">{info.pending.build} en attente de repos depuis le {date(info.pending.since)}</p>
    {/if}
    {#if info.auto_last}
      <p class="line" class:okc={info.auto_last.ok} class:bad={!info.auto_last.ok}>
        {info.auto_last.tag.replace('server-cuda-', '')}
        {info.auto_last.ok ? `installée le ${date(info.auto_last.at)}` : `écartée le ${date(info.auto_last.at)} : ${jobError(info.auto_last.message)}`}
      </p>
    {/if}
    {#if info.skipped.length}
      <p class="hint">Versions écartées (plus retentées seules ; une installation à la main les remet en jeu) : <span class="mono">{info.skipped.join(', ')}</span></p>
    {/if}
  </Section>

  <Section title="llama.cpp" text="Le moteur qui charge les modèles. Chaque version est testée avant d’être utilisée.">
    {#if v.env_override}<Note tone="warn">Binaire imposé par ENGINEX_LLAMA_BIN : les boutons de changement sont désactivés.</Note>{/if}
    <dl class="kv">
      <dt>Actif</dt>
      <dd><span class="mono">{v.active}</span>{#if v.previous}<span class="muted"> (précédent : <span class="mono">{v.previous}</span>)</span>{/if}</dd>
      <dt>Dernière publiée</dt>
      <dd>
        {#if v.latest}<span class="mono">{v.latest.tag}</span>{#if !newer} <Badge>installée</Badge>{/if}{:else}<span class="muted">non vérifiée</span>{/if}
      </dd>
    </dl>
    <div class="acts">
      <Button icon={RefreshCw} onclick={check} busy={checking}>Vérifier</Button>
      {#if newer}<Button variant="primary" icon={Download} onclick={() => run(() => installVersion(v?.latest?.tag))} disabled={running || v.env_override}>Installer {v.latest?.build}</Button>{/if}
      {#if v.previous}<Button icon={Undo2} onclick={() => run(rollbackVersion)} disabled={running || v.env_override}>Revenir à la précédente</Button>{/if}
    </div>
    {#if v.job}
      <div class="job">
        <p class="line">
          {JOB_KIND[v.job.kind]}{v.job.tag && ['install', 'use', 'rollback', 'auto_update'].includes(v.job.kind) ? ` de ${v.job.tag}` : ''} :
          <span class:okc={v.job.step === 'done'} class:bad={v.job.step === 'failed'}>{JOB_STEP[v.job.step]}</span>
          {#if v.job.step === 'download' && v.job.total_bytes}<span class="muted num"> {octets(v.job.done_bytes)} / {octets(v.job.total_bytes)}</span>{/if}
        </p>
        {#if v.job.step === 'download' && v.job.total_bytes}
          <ProgressBar value={v.job.done_bytes} max={v.job.total_bytes} label="Téléchargement" />
        {/if}
        {#if v.job.error}<Note tone="danger">{jobError(v.job.error)}</Note>{/if}
        {#if v.job.message}<p class="line okc">{v.job.message}</p>{/if}
      </div>
    {/if}
  </Section>

  <Section title="Versions installées" text="Revenir à une version déjà testée ne télécharge rien.">
    <table class="tbl">
      <thead><tr><th>Version</th><th>Build</th><th>Installée le</th><th>Dernier test</th><th><span class="sr-only">Actions</span></th></tr></thead>
      <tbody>
        {#each v.installed as i (i.tag)}
          <tr>
            <td><span class="mono">{i.tag}</span>{#if i.tag === v.active} <Badge tone="ok">actif</Badge>{/if}</td>
            <td class="num" data-l="Build">{i.build}</td>
            <td class="num" data-l="Installée le">{date(i.installed_at)}</td>
            <td data-l="Dernier test">
              {#if i.last_test?.ok}<span class="okc">OK</span><span class="muted">, {i.last_test.gpus} carte(s){i.last_test.has_fit ? ', --fit' : ', sans --fit'}</span>
              {:else if i.last_test}<span class="bad">{i.last_test.message ?? i.last_test.code}</span>
              {:else}<span class="muted">aucun</span>{/if}
            </td>
            <td class="act">
              {#if i.tag !== v.active}
                <Button size="sm" onclick={() => run(() => useVersion(i.tag))} disabled={running || v.env_override}>Utiliser</Button>
                {#if i.tag !== 'image'}<Button size="sm" variant="ghost" icon={Trash2} label="Supprimer {i.tag}" onclick={() => run(() => deleteVersion(i.tag))} disabled={running} />{/if}
              {/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  </Section>
{/if}

<style>
  .err { padding: var(--s4) var(--s6) 0; }
  .line { margin: 0; font-size: var(--fs-sm); }
  .hint { margin: 0; color: var(--muted); font-size: var(--fs-sm); }
  .muted { color: var(--muted); }
  .okc { color: var(--accent); }
  .bad { color: var(--danger); }
  .kv { display: grid; grid-template-columns: max-content 1fr; gap: var(--s2) var(--s5); margin: 0; font-size: var(--fs-sm); }
  .kv dt { color: var(--muted); }
  .kv dd { margin: 0; display: flex; flex-wrap: wrap; align-items: center; gap: var(--s2); overflow-wrap: anywhere; }
  .acts { display: flex; gap: var(--s2); flex-wrap: wrap; }
  .job { display: flex; flex-direction: column; gap: var(--s2); padding-top: var(--s3); border-top: 1px solid var(--line); }
  .tbl { width: 100%; border-collapse: collapse; font-size: var(--fs-sm); }
  .tbl th { text-align: left; font-weight: 400; color: var(--muted); padding: 0 var(--s3) var(--s2) 0; border-bottom: 1px solid var(--line); }
  .tbl td { padding: var(--s2) var(--s3) var(--s2) 0; border-bottom: 1px solid var(--line); vertical-align: middle; }
  .tbl tbody tr:hover { background: none; }
  .act { white-space: nowrap; text-align: right; padding-right: 0 !important; }
  .act :global(.btn) { margin-left: var(--s1); }
  @media (max-width: 767px) {
    .err { padding: var(--s4) var(--s4) 0; }
    .acts :global(.btn) { height: var(--touch); }
    .tbl thead { display: none; }
    .tbl, .tbl tbody, .tbl tr, .tbl td { display: block; }
    .tbl tr { padding: var(--s3) 0; border-bottom: 1px solid var(--line); }
    .tbl td { border: 0; padding: 2px 0; }
    .tbl td[data-l] { display: flex; justify-content: space-between; gap: var(--s3); }
    .tbl td[data-l]::before { content: attr(data-l); color: var(--muted); }
    .act { text-align: left; padding-top: var(--s2) !important; }
    .act :global(.btn) { height: var(--touch); margin: 0 var(--s2) 0 0; }
  }
</style>
