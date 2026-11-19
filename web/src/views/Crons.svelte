<script lang="ts">
  import { onDestroy } from 'svelte';
  import {
    crons, createCron, updateCron, deleteCron, runCron, cronRuns, cronPreview, projects, presets, message,
    type Cron, type CronBody, type CronRun, type Project, type Preset,
  } from '../lib/api';

  let list = $state<Cron[] | null>(null);
  let tree = $state<Project[]>([]);
  let presetList = $state<Preset[]>([]);
  presets().then((p) => (presetList = p)).catch(() => {});
  let error = $state('');
  let form = $state<(CronBody & { id: number | null }) | null>(null);
  let formError = $state('');
  let preview = $state<number[]>([]);
  let previewError = $state('');
  let history = $state<{ cron: Cron; runs: CronRun[] } | null>(null);

  const examples = [
    { s: 'every:30', l: 'toutes les 30 min' },
    { s: '0 9 * * *', l: 'chaque jour à 9 h' },
    { s: '0 8 * * 1-5', l: 'en semaine à 8 h' },
    { s: '0 7 * * 1', l: 'le lundi à 7 h' },
  ];
  const statusLabel: Record<string, string> = { running: 'en cours', ok: 'réussi', failed: 'échec', skipped: 'sauté' };
  const date = (t: number | null) =>
    t ? new Date(t * 1000).toLocaleString('fr-FR', { dateStyle: 'short', timeStyle: 'short' }) : '—';
  const duration = (r: { started_at: number; ended_at: number | null }) => {
    if (!r.ended_at) return '';
    const s = r.ended_at - r.started_at;
    return s < 60 ? `${s} s` : `${Math.floor(s / 60)} min ${s % 60} s`;
  };
  const scheduleText = (s: string) => {
    const m = /^every:(\d+)$/.exec(s);
    return m ? `toutes les ${m[1]} min` : (examples.find((e) => e.s === s)?.l ?? s);
  };

  async function load() {
    try {
      list = await crons();
      error = '';
      if (history) {
        const c = list.find((x) => x.id === history!.cron.id);
        if (c) history = { cron: c, runs: await cronRuns(c.id) };
      }
    } catch (e) {
      error = message(e);
    }
  }
  load();
  projects().then((p) => (tree = p)).catch((e) => (error = message(e)));
  const timer = setInterval(() => {
    if (!form && document.visibilityState === 'visible') load();
  }, 5000);
  onDestroy(() => clearInterval(timer));

  async function act(f: () => Promise<unknown>) {
    error = '';
    try {
      await f();
    } catch (e) {
      error = message(e);
    }
    await load();
  }

  function edit(c: Cron | null) {
    formError = '';
    const first = tree[0]?.topics[0]?.id ?? 0;
    form = c
      ? { id: c.id, name: c.name, topic_id: c.topic_id, kind: c.kind, body: c.body, schedule: c.schedule, tz: c.tz, timeout_s: c.timeout_s, enabled: c.enabled, preset_id: c.preset_id }
      : { id: null, name: '', topic_id: first, kind: 'prompt', body: '', schedule: '0 9 * * *', tz: 'Europe/Paris', timeout_s: 1800, enabled: true, preset_id: null };
  }

  // Aperçu des 3 prochaines exécutions pendant la saisie.
  let previewTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const s = form?.schedule;
    const tz = form?.tz;
    clearTimeout(previewTimer);
    if (!s) return;
    previewTimer = setTimeout(async () => {
      try {
        preview = (await cronPreview(s, tz)).next;
        previewError = '';
      } catch (e) {
        preview = [];
        previewError = message(e);
      }
    }, 300);
  });

  async function save(e: SubmitEvent) {
    e.preventDefault();
    if (!form) return;
    const { id, ...body } = form;
    formError = '';
    try {
      if (id === null) await createCron(body);
      else await updateCron(id, body);
      form = null;
      await load();
    } catch (err) {
      formError = message(err);
    }
  }

  function remove(c: Cron) {
    if (!confirm(`Supprimer le cron « ${c.name} » et son historique ? Sa session reste.`)) return;
    if (history?.cron.id === c.id) history = null;
    act(() => deleteCron(c.id));
  }

  async function showHistory(c: Cron) {
    try {
      history = { cron: c, runs: await cronRuns(c.id) };
    } catch (e) {
      error = message(e);
    }
  }

  const toggle = (c: Cron) =>
    act(() => updateCron(c.id, { name: c.name, topic_id: c.topic_id, kind: c.kind, body: c.body, schedule: c.schedule, tz: c.tz, timeout_s: c.timeout_s, enabled: !c.enabled, preset_id: c.preset_id }));
</script>

<section>
  <div class="row head">
    <h1>Crons</h1>
    <span class="grow"></span>
    <button onclick={() => edit(null)} disabled={!tree.length}>Nouveau cron</button>
  </div>
  <p class="muted small">
    Une consigne lance un tour de l'IA dans la session « Cron : nom » du topic ; une commande s'exécute dans sa sandbox.
    Une exécution à la fois par cron ; après un arrêt d'EngineX, une seule exécution de rattrapage.
  </p>
  {#if error}<p class="error">{error}</p>{/if}

  {#if list && list.length === 0}
    <p class="muted">Aucun cron.</p>
  {:else if list}
    <table class="crons">
      <thead>
        <tr><th>Nom</th><th>Topic</th><th>Quand</th><th>Prochaine</th><th>Dernière</th><th></th></tr>
      </thead>
      <tbody>
        {#each list as c (c.id)}
          <tr class:off={!c.enabled}>
            <td>
              <button class="link strong" onclick={() => showHistory(c)}>{c.name}</button>
              <span class="badge">{c.kind === 'prompt' ? 'consigne' : 'commande'}</span>
            </td>
            <td class="muted">{c.topic ?? '?'}</td>
            <td title="{c.schedule} ({c.tz})">{scheduleText(c.schedule)}</td>
            <td>{c.enabled ? date(c.next_run) : 'désactivé'}</td>
            <td>
              {#if c.running}
                <span class="ok">en cours…</span>
              {:else if c.last}
                <span class:ok={c.last.status === 'ok'} class:error={c.last.status === 'failed'}>{statusLabel[c.last.status]}</span>
                <span class="muted small">{date(c.last.started_at)}</span>
              {:else}<span class="muted">jamais</span>{/if}
            </td>
            <td class="actions">
              <button class="link" onclick={() => act(() => runCron(c.id))} disabled={c.running}>Lancer</button>
              <button class="link" onclick={() => edit(c)}>Modifier</button>
              <button class="link" onclick={() => toggle(c)}>{c.enabled ? 'Désactiver' : 'Activer'}</button>
              {#if c.session_id}<a href="#sessions/{c.session_id}">Session</a>{/if}
              <button class="link error" onclick={() => remove(c)}>Supprimer</button>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {:else}
    <p class="muted">Chargement…</p>
  {/if}

  {#if history}
    <div class="card history">
      <div class="row">
        <h2>Historique : {history.cron.name}</h2>
        <span class="grow"></span>
        <button class="link" onclick={() => (history = null)}>Fermer</button>
      </div>
      {#each history.runs as r (r.id)}
        <details open={r === history.runs[0]}>
          <summary>
            <span class:ok={r.status === 'ok'} class:error={r.status === 'failed'}>{statusLabel[r.status]}</span>
            · {date(r.started_at)} · {r.trigger === 'manual' ? 'à la main' : 'planifié'}
            {#if duration(r)}· {duration(r)}{/if}
            {#if r.session_id}· <a href="#sessions/{r.session_id}">session</a>{/if}
          </summary>
          {#if r.output}<pre>{r.output}</pre>{:else}<p class="muted small">Pas de sortie.</p>{/if}
        </details>
      {:else}
        <p class="muted">Aucune exécution.</p>
      {/each}
    </div>
  {/if}
</section>

{#if form}
  <div class="overlay" role="presentation" onclick={(e) => e.target === e.currentTarget && (form = null)}>
    <form class="card" onsubmit={save}>
      <h2>{form.id === null ? 'Nouveau cron' : 'Modifier le cron'}</h2>
      <div class="row">
        <label class="grow">Nom <input bind:value={form.name} maxlength="80" required /></label>
        <label class="grow">Topic
          <select bind:value={form.topic_id}>
            {#each tree as p}
              <optgroup label={p.name}>
                {#each p.topics as t}<option value={t.id}>{p.slug} / {t.slug}</option>{/each}
              </optgroup>
            {/each}
          </select>
        </label>
      </div>
      <div class="row kind">
        <label><input type="radio" bind:group={form.kind} value="prompt" /> Consigne à l'IA</label>
        <label><input type="radio" bind:group={form.kind} value="shell" /> Commande dans la sandbox</label>
      </div>
      <label>{form.kind === 'prompt' ? 'Consigne' : 'Commande (bash)'}
        <textarea rows="5" class:mono={form.kind === 'shell'} bind:value={form.body} required
          placeholder={form.kind === 'prompt' ? 'Fais le point sur les tests qui échouent et propose un correctif.' : 'df -h /workspace'}></textarea>
      </label>
      <div class="row">
        <label class="grow">Quand <input class="mono" bind:value={form.schedule} required /></label>
        <label>Fuseau <input bind:value={form.tz} /></label>
        <label>Délai max (s) <input type="number" min="10" max="7200" bind:value={form.timeout_s} /></label>
      </div>
      {#if form.kind === 'prompt'}
        <label title="Un autre modèle que le chargé est chargé pour ce cron, après les tours de Michael">Modèle
          <select bind:value={form.preset_id}>
            <option value={null}>Par défaut</option>
            {#each presetList as p (p.id)}<option value={p.id}>{p.name}</option>{/each}
          </select>
        </label>
      {/if}
      <p class="small">
        {#each examples as ex}<button type="button" class="link" onclick={() => (form!.schedule = ex.s)}>{ex.l}</button> {/each}
      </p>
      <p class="small muted">
        Cron à 5 champs (minute heure jour mois jour-semaine) ou <code>every:N</code> (toutes les N minutes).
        {#if previewError}<span class="error">{previewError}</span>
        {:else if preview.length}Prochaines : {preview.map(date).join(' · ')}{/if}
      </p>
      <label class="check"><input type="checkbox" bind:checked={form.enabled} /> Activé</label>
      {#if formError}<p class="error">{formError}</p>{/if}
      <div class="row">
        <button type="submit">Enregistrer</button>
        <button type="button" class="link" onclick={() => (form = null)}>Annuler</button>
      </div>
    </form>
  </div>
{/if}

<style>
  .head { align-items: center; }
  .head h1 { margin: 0; }
  .crons { width: 100%; border-collapse: collapse; margin-top: var(--s4); }
  .crons th { text-align: left; font-weight: 500; color: var(--muted); font-size: var(--fs-sm); }
  .crons th, .crons td { padding: var(--s2); border-bottom: 1px solid var(--line); vertical-align: top; }
  .crons tr.off td { opacity: 0.6; }
  .actions { white-space: nowrap; display: flex; gap: var(--s2); }
  .strong { font-weight: 600; }
  .history { margin-top: var(--s6); }
  .history pre { white-space: pre-wrap; overflow-wrap: anywhere; max-height: 320px; overflow: auto; background: var(--bg); padding: var(--s2); border-radius: var(--radius); font-size: var(--fs-sm); }
  summary { cursor: pointer; padding: var(--s1) 0; }
  .overlay { position: fixed; inset: 0; background: rgb(0 0 0 / 0.5); display: grid; place-items: center; z-index: 10; }
  .overlay form { width: min(680px, 94vw); max-height: 92vh; overflow: auto; }
  .kind { gap: var(--s4); }
  .check { display: flex; gap: var(--s2); align-items: center; }
  textarea { width: 100%; }
  .mono { font-family: ui-monospace, monospace; }
</style>
