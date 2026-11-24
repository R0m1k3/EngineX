<script lang="ts">
  // Fiche d'un test : verdict, contrôles à points, captures, note visuelle.
  import { untrack } from 'svelte';
  import { rateBench, sessionDetail, benchShotUrl, message, type BenchCase, type ChangedFile } from '../../lib/api';
  import { Button, Textarea, Badge, toast } from '../../lib/ui';
  import Preview from '../sessions/Preview.svelte';
  import ArrowLeft from '@lucide/svelte/icons/arrow-left';
  import MessageSquare from '@lucide/svelte/icons/message-square';
  import { VERDICT, STATUS, VISUAL_SCALE, dur, pts } from './shared';

  let {
    runId,
    c,
    elapsed,
    onback,
    onsaved,
  }: {
    runId: number;
    c: BenchCase;
    elapsed: number | null;
    onback: () => void;
    /** `next` : passer au prochain test visuel à noter. */
    onsaved: (next: boolean) => void;
  } = $props();

  let files = $state<ChangedFile[]>([]);
  let note = $state<number | null>(null);
  let comment = $state('');
  let saving = $state(false);
  let live = $state(false);

  // Le rafraîchissement remplace `c` toutes les 3 s pendant un run : on ne
  // repart qu'au changement de test ou à la fin de son audit (saisie préservée).
  const selKey = $derived(`${c.id}:${c.audited_at ?? ''}`);
  $effect(() => {
    void selKey;
    const cur = untrack(() => c);
    files = [];
    note = cur.visual;
    comment = cur.comment ?? '';
    live = false;
    if (cur.session_id) {
      sessionDetail(cur.session_id)
        .then((d) => {
          const seen = new Map<string, ChangedFile>();
          for (const t of d.turns) for (const f of t.changes?.files ?? []) seen.set(f.path, f);
          files = [...seen.values()];
        })
        .catch(() => {});
    }
  });

  const isVisual = $derived(c.family === 'visual');
  const v = $derived(c.verdict ? VERDICT[c.verdict] : null);
  const failedEssential = $derived(c.audit?.checks.filter((k) => k.essential && !k.ok) ?? []);

  async function save(next: boolean) {
    saving = true;
    try {
      await rateBench(runId, c.id, { visual: isVisual && note != null ? note : undefined, comment });
      toast('Enregistré.');
      onsaved(next);
    } catch (e) {
      toast(message(e), 'error');
    } finally {
      saving = false;
    }
  }
</script>

<div class="detail">
  <div class="rowline">
    <Button variant="ghost" icon={ArrowLeft} onclick={onback}>Retour</Button>
    <h2 class="grow">{c.title}</h2>
    {#if elapsed != null}<span class="num faint">{dur(elapsed)}</span>{/if}
    {#if c.session_id}<Button size="sm" icon={MessageSquare} href={`#sessions/${c.session_id}`}>Ouvrir la session</Button>{/if}
  </div>

  {#if !c.audit}
    <p class="faint">{c.status === 'running' || c.status === 'pending' ? `Test ${STATUS[c.status]} : les contrôles se font à la fin.` : `Pas de contrôle pour ce test${c.reason ? ` : ${c.reason}` : '.'}`}</p>
  {:else}
    <section class="verdict {c.verdict}">
      <div class="big">
        {#if v}<Badge tone={v.tone}>{v.label}</Badge>{/if}
        <span class="score num"><strong>{pts(c.points)}</strong> / {c.points_max} pts</span>
        {#if isVisual}<span class="faint">· visuel {c.visual == null ? 'à noter' : `${c.visual} / 5 (${VISUAL_SCALE[c.visual - 1].label})`}</span>{/if}
      </div>
      <p class="faint small">
        {#if c.verdict === 'failed'}
          Échoué : contrôle essentiel raté ({failedEssential.map((k) => k.name).join(' ; ')}).
        {:else if c.verdict === 'partial'}
          Partiel : les contrôles essentiels passent, d’autres non.
        {:else}
          Réussi : tous les contrôles passent.
        {/if}
        {#if c.reason} — {c.reason}{/if}
      </p>
      {#if c.audit.page_error}
        <p class="page-error">
          <strong>Le navigateur de l’audit n’a pas ouvert la page</strong> : les contrôles de page sont KO sans avoir été joués.
          <code>{c.audit.page_error}</code>
        </p>
      {/if}
    </section>

    <section>
      <h3>Contrôles de la machine</h3>
      <ul class="checks">
        {#each c.audit.checks as k (k.name)}
          <li>
            <details>
              <summary>
                <Badge tone={k.ok ? 'ok' : k.earned > 0 ? 'warn' : 'danger'}>{k.ok ? 'OK' : 'KO'}</Badge>
                <span class="grow">{k.name}{#if k.essential} <span class="ess" title="S’il rate, le test est échoué">essentiel</span>{/if}</span>
                <span class="num pts">{pts(k.earned)} / {k.points}</span>
              </summary>
              <pre class="proof">{k.proof}</pre>
            </details>
          </li>
        {/each}
      </ul>
    </section>

    {#if c.audit.shots.length}
      <section>
        <h3>Rendu</h3>
        <div class="shots">
          {#each c.audit.shots as s (s)}
            <figure class={s}>
              <figcaption class="faint small">{s === 'desktop' ? 'Ordinateur (1280 px)' : 'Téléphone (390 px)'}</figcaption>
              <a href={benchShotUrl(c.id, s)} target="_blank" rel="noopener"><img src={benchShotUrl(c.id, s)} alt={`Capture ${s}`} loading="lazy" /></a>
            </figure>
          {/each}
        </div>
        {#if c.session_id}
          {#if live}
            <div class="preview"><Preview sessionId={c.session_id} idle={{ title: 'Pas de page à afficher', text: 'La sandbox du test est arrêtée.' }} /></div>
          {:else}
            <Button size="sm" variant="ghost" onclick={() => (live = true)}>Essayer la page en vrai (aperçu vivant)</Button>
          {/if}
        {/if}
      </section>
    {/if}

    <section>
      <h3>{isVisual ? 'Votre note du visuel' : 'Votre commentaire'}</h3>
      {#if isVisual}
        <p class="faint small">Jugez seulement l’apparence (mise en page, couleurs, lisibilité, téléphone) : le fonctionnement est déjà contrôlé au-dessus.</p>
        <div class="scale" role="radiogroup" aria-label="Note du visuel">
          {#each VISUAL_SCALE as s (s.value)}
            <button type="button" role="radio" aria-checked={note === s.value} class:on={note === s.value} onclick={() => (note = s.value)} title={s.hint}>
              <strong>{s.value}</strong><span>{s.label}</span><small>{s.hint}</small>
            </button>
          {/each}
        </div>
      {/if}
      <Textarea label="Commentaire (facultatif)" bind:value={comment} rows={2} />
      <div class="rowline">
        <Button variant="primary" onclick={() => save(isVisual)} busy={saving} disabled={isVisual && note == null}>
          {isVisual ? 'Enregistrer et suivant' : 'Enregistrer'}
        </Button>
      </div>
    </section>

    <section>
      <h3>Déroulé</h3>
      <p class="num small">
        {dur(c.audit.metrics.duration_s)} · {c.audit.metrics.tokens} jetons · {c.audit.metrics.tps ? `${c.audit.metrics.tps.toFixed(1)} t/s` : 't/s —'} ·
        {c.audit.metrics.commands} commandes · {c.audit.metrics.tools} appels d’outils
      </p>
      {#if c.audit.alerts.length}
        <h4>Alertes (pour information, sans effet sur les points)</h4>
        <ul class="plain">
          {#each c.audit.alerts as a, i (i)}<li><Badge tone="warn">{a.kind === 'big_file' ? 'fichier long' : a.kind === 'repeated_command' ? 'commande répétée' : 'paquet ajouté'}</Badge> {a.text}</li>{/each}
        </ul>
      {/if}
      {#if files.length}
        <h4>Fichiers touchés</h4>
        <ul class="plain">
          {#each files as f (f.path)}<li><code>{f.status}</code> {f.path} <span class="faint small">+{f.added} −{f.deleted}</span></li>{/each}
        </ul>
      {/if}
    </section>
  {/if}
</div>

<style>
  .detail { display: grid; gap: 16px; padding: 0 var(--sp-4, 16px) var(--sp-4, 16px); }
  .rowline { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; }
  .grow { flex: 1; min-width: 0; }
  h2 { margin: 0; font-size: var(--fs-lg, 1.1rem); }
  h3 { margin: 0 0 8px; font-size: var(--fs-md, 1rem); }
  h4 { margin: 12px 0 4px; font-size: var(--fs-sm); }
  section { border: 1px solid var(--line); border-radius: var(--r); padding: 12px; }
  .verdict.passed { border-color: var(--accent); }
  .verdict.failed { border-color: var(--danger, #c0392b); }
  .big { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; }
  .score { font-size: 1.3rem; }
  .checks, .plain { list-style: none; margin: 0; padding: 0; display: grid; gap: 4px; }
  .checks summary { display: flex; align-items: center; gap: 8px; cursor: pointer; padding: 4px 0; }
  .pts { white-space: nowrap; color: var(--muted); }
  .ess { font-size: var(--fs-xs); color: var(--muted); border: 1px solid var(--line); border-radius: var(--r); padding: 0 4px; }
  .page-error { margin: var(--s2) 0 0; color: var(--danger, #c0392b); font-size: var(--fs-sm); }
  .page-error code { display: block; margin-top: 4px; white-space: pre-wrap; color: var(--text); }
  .proof { margin: 6px 0 0; padding: 8px; background: var(--raised); border-radius: var(--r); overflow: auto; max-height: 220px; font-size: var(--fs-sm); white-space: pre-wrap; }
  .shots { display: flex; gap: 16px; align-items: flex-start; flex-wrap: wrap; margin-bottom: 8px; }
  figure { margin: 0; display: grid; gap: 4px; }
  figure.desktop { flex: 3 1 420px; }
  figure.phone { flex: 1 1 160px; max-width: 260px; }
  figure img { width: 100%; height: auto; max-height: 560px; object-fit: contain; object-position: top center; background: var(--raised); border: 1px solid var(--line); border-radius: var(--r); display: block; }
  .preview { height: 480px; border: 1px solid var(--line); border-radius: var(--r); overflow: hidden; }
  .scale { display: grid; grid-template-columns: repeat(5, 1fr); gap: 8px; margin: 8px 0 12px; }
  @media (max-width: 800px) { .scale { grid-template-columns: 1fr; } }
  .scale button { all: unset; cursor: pointer; display: grid; gap: 2px; padding: 10px; border: 1px solid var(--line); border-radius: var(--r); text-align: center; }
  .scale button:hover, .scale button:focus-visible { border-color: var(--accent); }
  .scale button.on { border-color: var(--accent); background: var(--raised); box-shadow: inset 0 0 0 1px var(--accent); }
  .scale strong { font-size: 1.3rem; }
  .scale small { color: var(--muted); font-size: var(--fs-xs); }
  .faint { color: var(--faint, var(--muted)); }
  .small { font-size: var(--fs-sm); }
  p { margin: 4px 0; }
</style>
