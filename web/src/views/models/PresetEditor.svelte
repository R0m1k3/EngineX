<script lang="ts">
  import { gb, int, octets } from '../../lib/ui/format';
  import type { Snippet } from 'svelte';
  import Save from '@lucide/svelte/icons/save';
  import ExternalLink from '@lucide/svelte/icons/external-link';
  import Button from '../../lib/ui/Button.svelte';
  import Input from '../../lib/ui/Input.svelte';
  import Select from '../../lib/ui/Select.svelte';
  import Switch from '../../lib/ui/Switch.svelte';
  import Segmented from '../../lib/ui/Segmented.svelte';
  import Gauge from '../../lib/ui/Gauge.svelte';
  import Badge from '../../lib/ui/Badge.svelte';
  import Icon from '../../lib/ui/Icon.svelte';
  import { confirmDialog } from '../../lib/ui/dialog.svelte';
  import { toast } from '../../lib/ui/toast.svelte';
  import type { GaugeRow, Tone } from '../../lib/ui/types';
  import Section from '../settings/Section.svelte';
  import Note from '../settings/Note.svelte';
  import {
    savePreset, activatePreset, presetFit, message, engine, hasMtp, SAMPLING, VERDICT,
    type Preset, type LocalModel, type PresetSettings, type SamplingKey, type CandidateSet, type EngineInfo,
    type FitReport, type Gpu, type Verdict,
  } from '../../lib/api';

  let { preset, model, models = [], saved, actions }: {
    preset: Preset; model: LocalModel | undefined; models?: LocalModel[]; saved: () => void; actions?: Snippet;
  } = $props();

  // Copie modifiable du preset (l'original reste celui du serveur).
  const init = () => $state.snapshot(preset);
  let name = $state(init().name);
  let s = $state<PresetSettings>(init().settings);
  let split = $state(init().settings.tensor_split?.join(',') ?? '');
  let extra = $state(init().settings.extra_args.join(' '));
  let error = $state('');
  let busy = $state(false);
  let caps = $state<EngineInfo['caps']>(null);
  let gpus = $state<Gpu[]>([]);
  engine().then((e) => { caps = e.caps; gpus = e.gpus; }).catch(() => {});

  const mtpOk = $derived(hasMtp(model?.meta));
  const isMoe = $derived(!!model?.meta.expert_count);
  // Brouillon possible : un autre modèle local, plus petit, de la même architecture.
  const drafts = $derived(
    models.filter((m) => m.id !== model?.id && m.size_bytes < (model?.size_bytes ?? 0) && m.meta.arch === model?.meta.arch),
  );
  const optNum = (v: string) => (v.trim() === '' ? null : Number(v));

  function setDraft(v: string) {
    if (v === 'mtp' || v === 'none') {
      s.spec.draft = v;
      s.spec.draft_model_id = null;
    } else {
      s.spec.draft = 'model';
      s.spec.draft_model_id = Number(v);
    }
  }
  const draftValue = $derived(s.spec.draft === 'model' ? String(s.spec.draft_model_id) : s.spec.draft);
  const draftOptions = $derived([
    { value: 'none', label: 'Aucun' },
    { value: 'mtp', label: mtpOk ? 'MTP du modèle' : 'MTP du modèle (absent de ce GGUF)', disabled: !mtpOk },
    ...drafts.map((d) => ({ value: String(d.id), label: `${d.repo.split('/').pop()} (${d.file.split('/').pop()})` })),
  ]);

  const sourceLabel: Record<string, string> = {
    card_repo: 'page HF du dépôt',
    card_base: 'page HF du modèle de base',
    generation_config: 'generation_config.json',
    gguf: 'métadonnées du GGUF',
    defaut: 'non trouvé (défaut de llama.cpp)',
  };
  const labelName: Record<string, string> = { thinking: 'raisonnement', non_thinking: 'sans raisonnement', general: 'général' };

  function setSampling(k: SamplingKey, v: string) {
    s.sampling[k] = v.trim() === '' ? null : Number(v);
  }

  function choose(k: SamplingKey, c: CandidateSet) {
    s.sampling[k] = c.values[k] ?? null;
  }

  // Place estimée avec les réglages en cours d'édition.
  let fit = $state<FitReport | null>(null);
  let fitTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const settings = $state.snapshot(s);
    clearTimeout(fitTimer);
    fitTimer = setTimeout(() => {
      presetFit(preset.id, settings).then((r) => (fit = r)).catch(() => (fit = null));
    }, 400);
  });

  // Jauge : une ligne par carte (besoin estimé sur place disponible) et une pour le processeur.
  // L'estimation donne le cache KV en total : il est réparti entre les cartes au prorata
  // de leur besoin (les couches, donc leur cache, suivent --fit). Sans flash attention,
  // la matrice d'attention réservée par carte forme le segment « calcul ».
  const rows = $derived.by<GaugeRow[]>(() => {
    if (!fit) return [];
    const f = fit;
    const need = f.cards.reduce((a, c) => a + c.need_mib, 0);
    const out: GaugeRow[] = f.cards.map((c, i) => {
      const g = gpus[i];
      const kv = need > 0 ? Math.min(c.need_mib, (f.kv_mib * c.need_mib) / need) : 0;
      const extra = (s.flash_attn ? 0 : f.no_fa_mib) + (i === 0 ? (f.mask_mib ?? 0) : 0);
      const compute = Math.min(extra, Math.max(0, c.need_mib - kv));
      return {
        label: `CUDA${i}`,
        sub: g?.name,
        total_mib: c.avail_mib,
        weights_mib: Math.max(0, c.need_mib - kv - compute),
        kv_mib: kv,
        compute_mib: compute || undefined,
      };
    });
    out.push({
      label: 'CPU',
      sub: 'RAM système',
      total_mib: 0,
      note: f.cpu_mib > 0 ? `${gb(f.cpu_mib)} Go` : 'rien',
    });
    return out;
  });
  // Seuil de compactage de Codex (miroir de `codex_config::compact_limit`) : contexte
  // par requête, moins la réflexion et 10 240 de marge, entre 50 % et 75 % du contexte.
  const compactAt = $derived.by(() => {
    const ctx = Math.floor((s.ctx || 0) / Math.max(1, s.parallel || 1));
    const reserve = (s.reasoning === 'on' ? (s.reasoning_budget ?? 0) : 0) + 10240;
    return Math.max(Math.min(Math.round(ctx * 0.75), Math.max(0, ctx - reserve)), Math.floor(ctx / 2));
  });
  const verdictTone: Record<Verdict, Tone> = { fits: 'ok', tight: 'warn', offload: 'warn', no_fit: 'danger', unknown: 'neutral' };

  async function save() {
    error = '';
    const ts = split.trim() ? split.split(',').map((x) => Number(x.trim())) : null;
    if (ts && ts.some((x) => !Number.isFinite(x) || x < 0)) {
      error = 'Répartition : nombres positifs séparés par des virgules (ex. 16,12).';
      return;
    }
    const settings: PresetSettings = {
      ...$state.snapshot(s),
      tensor_split: ts,
      extra_args: extra.trim() ? extra.trim().split(/\s+/) : [],
    };
    busy = true;
    try {
      const r = await savePreset(preset.id, name, settings);
      if (r.reload_needed) {
        const now = await confirmDialog('Ces changements demandent de recharger le modèle. Recharger maintenant ?', {
          title: 'Recharger le modèle',
          confirm: 'Recharger maintenant',
          cancel: 'Plus tard',
        });
        if (now) {
          await activatePreset(preset.id, true);
          toast('Enregistré, rechargement lancé (après la fin des requêtes en cours).', 'ok');
        } else {
          toast('Enregistré : pris en compte au prochain chargement.', 'ok');
        }
      } else {
        toast('Enregistré : appliqué dès la prochaine requête.', 'ok');
      }
      saved();
    } catch (e) {
      error = message(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="pe">
  <header class="top">
    <div class="ttl">
      <h2 class="ell">{name || preset.name}</h2>
      {#if model}
        <p class="sub">
          {model.repo}, {model.file.split('/').pop()}{model.shards.length > 1 ? ` (${model.shards.length} tranches)` : ''}
          ({octets(model.size_bytes)}){#if model.mmproj} et {model.mmproj.split('/').pop()}{/if}
        </p>
      {:else}
        <p class="sub">Modèle introuvable.</p>
      {/if}
    </div>
    <div class="acts">
      {#if actions}{@render actions()}{/if}
      <Button variant="primary" icon={Save} onclick={save} {busy}>Enregistrer</Button>
    </div>
  </header>
  {#if error}<div class="err"><Note tone="danger">{error}</Note></div>{/if}

  <Section title="Place en mémoire" text="Estimation pour les réglages en cours. --fit répartit les couches sur les cartes.">
    {#snippet aside()}
      {#if fit && fit.verdict !== 'unknown'}
        <div class="verdict"><Badge tone={verdictTone[fit.verdict]} dot>{VERDICT[fit.verdict]}</Badge></div>
      {/if}
    {/snippet}
    {#if fit && fit.verdict !== 'unknown'}
      <Gauge {rows} size="lg" legend />
      {#if fit.cpu_mib > 0}
        <Note tone="danger">Environ {gb(fit.cpu_mib)} Go calculés par le processeur : génération bien plus lente.</Note>
      {/if}
    {:else if fit}
      <p class="muted">Estimation impossible pour ce modèle.</p>
    {:else}
      <p class="muted">Estimation en cours…</p>
    {/if}
    {#if fit?.mask_mib}
      <Note tone="warn">
        Inkling 2 : llama.cpp réserve sur CUDA0 un masque d’attention d’environ {gb(fit.mask_mib)} Go,
        proportionnel au lot physique (-ub) × contexte. Baissez le lot physique (256 ou 512) ou le contexte.
      </Note>
    {/if}
    {#if !s.flash_attn}
      <Note tone="warn">
        Sans flash attention, llama.cpp réserve la matrice d’attention entière sur chaque carte
        {#if fit?.no_fa_mib}(≈ {gb(fit.no_fa_mib)} Go de plus par carte à ce contexte){/if}.
        {#if s.kv_type !== 'f16'}Et un cache quantifié ({s.kv_type}) demande la flash attention.{/if}
      </Note>
    {/if}
  </Section>

  <Section title="Contexte" text="Taille de la fenêtre et format du cache. Rechargement nécessaire.">
    <div class="g4">
      <Input label="Contexte" unit="jetons" type="number" min="512" max={model?.meta.context_length ?? 1048576} step="1024"
        bind:value={s.ctx} hint={model?.meta.context_length ? `${int(model.meta.context_length)} au maximum` : undefined} />
      <Select label="Cache KV" bind:value={s.kv_type} options={[
        { value: 'q8_0', label: 'q8_0' }, { value: 'f16', label: 'f16' }, { value: 'q4_0', label: 'q4_0' },
      ]} />
      <Input label="Requêtes en parallèle" type="number" min="1" max="8" bind:value={s.parallel} />
    </div>
    <div class="switches">
      <Switch bind:checked={s.flash_attn}>Flash attention</Switch>
      {#if model?.mmproj}<Switch bind:checked={s.mmproj}>Vision (mmproj chargé)</Switch>{/if}
    </div>
  </Section>

  <Section title="Échantillonnage" text="Envoyé avec chaque requête, sans recharger. Valeurs prises sur la page du modèle.">
    {#if preset.sources.error}<Note tone="danger">Sources non lues : {preset.sources.error}</Note>{/if}
    <div class="samp">
      {#each SAMPLING as k (k)}
        {@const o = preset.sources.origins[k]}
        {@const options = preset.sources.to_choose[k]}
        <div class="sk">
          <Input label={k} class="mono-in" value={s.sampling[k] ?? ''} oninput={(e) => setSampling(k, e.currentTarget.value)} placeholder="défaut" inputmode="decimal" />
          <div class="src">
            {sourceLabel[o?.source ?? 'defaut']}
            {#if o?.url}<a href={o.url} target="_blank" rel="noreferrer noopener" class="ext">page<Icon icon={ExternalLink} size={14} /></a>{/if}
          </div>
          {#if o?.excerpt}
            <details class="exc"><summary>Voir l’extrait</summary><pre>{o.excerpt}</pre></details>
          {/if}
          {#if options}
            <div class="pick">
              <span class="muted">À choisir :</span>
              {#each options as c, i (i)}
                <button type="button" class="chip" title={c.excerpt} onclick={() => choose(k, c)}>{c.values[k]} ({labelName[c.label]})</button>
              {/each}
            </div>
          {/if}
        </div>
      {/each}
    </div>
  </Section>

  <Section title="Accélération" text="Décodage spéculatif, experts et taille des lots. Rechargement nécessaire.">
    {#if caps && !caps.spec}<Note tone="warn">Ce llama-server n’a pas le décodage spéculatif (MTP, brouillon) : options ignorées.</Note>{/if}
    <div class="g2">
      <Select label="Brouillon (décodage spéculatif)" value={draftValue} options={draftOptions}
        onchange={(e) => setDraft(e.currentTarget.value)} />
      {#if s.spec.draft !== 'none'}
        <Input label="Jetons proposés par étape" type="number" min="1" max="16" bind:value={s.spec.draft_max} />
      {/if}
    </div>
    <div class="switches">
      <Switch bind:checked={s.spec.ngram}>Brouillon ngram (répétitions du contexte, sans VRAM)</Switch>
    </div>
    {#if isMoe}
      <div class="g2">
        <Select label="Experts MoE ({model?.meta.expert_count} experts, {model?.meta.expert_used_count ?? '?'} actifs)" bind:value={s.moe.mode} options={[
          { value: 'auto', label: 'automatique (--fit)' },
          { value: 'all_cpu', label: 'tous en RAM' },
          { value: 'layers', label: 'N premières couches en RAM' },
        ]} />
        {#if s.moe.mode === 'layers'}
          <Input label="Couches d’experts en RAM" type="number" min="1" max={model?.meta.block_count ?? 1024} bind:value={s.moe.cpu_layers} />
        {/if}
        {#if s.moe.mode !== 'auto'}
          <Input label="Cache GPU des experts" unit="Mio" hint="0 = sans" type="number" min="0" max="65536" step="512" bind:value={s.moe.cache_mib} />
        {/if}
      </div>
    {/if}
    <div class="g2">
      <Input label="Lot logique -b" placeholder="2048" hint="Vide = 2048" type="number" min="32" max="16384" step="512"
        value={s.batch ?? ''} oninput={(e) => (s.batch = optNum(e.currentTarget.value))} />
      <Input label="Lot physique -ub" placeholder="512" hint="Vide = 512" type="number" min="32" max="16384" step="256"
        value={s.ubatch ?? ''} oninput={(e) => (s.ubatch = optNum(e.currentTarget.value))} />
    </div>
    {#if s.spec.ngram && s.moe.mode !== 'auto'}
      <p class="hint">ngram est retiré au lancement tant que des experts sont en RAM : la vérification passerait par le CPU.</p>
    {/if}
    {#if s.spec.draft !== 'none'}
      <p class="hint">Si le brouillon empêche le chargement, le modèle est relancé sans et la barre du moteur le signale.</p>
    {/if}
  </Section>

  <Section title="Réflexion" text="Bornes par réponse, pour qu’une réflexion ne remplisse pas tout le contexte.">
    <div class="field">
      <span class="lab">Raisonnement</span>
      <Segmented label="Raisonnement" bind:value={s.reasoning} options={[{ value: 'on', label: 'Activé' }, { value: 'off', label: 'Désactivé' }]} />
    </div>
    <div class="g2">
      {#if s.reasoning === 'on'}
        <Input label="Réflexion max par réponse" unit="jetons" type="number" min="256" step="1024" placeholder="illimitée"
          hint="Au-delà, le modèle passe à la réponse. Vide = illimitée."
          value={s.reasoning_budget ?? ''} oninput={(e) => (s.reasoning_budget = optNum(e.currentTarget.value))} />
      {/if}
      <Input label="Sortie max par réponse" unit="jetons" type="number" min="256" step="1024" placeholder="illimitée"
        hint="Réflexion comprise. Vide = jusqu’à la fin du contexte."
        value={s.max_output ?? ''} oninput={(e) => (s.max_output = optNum(e.currentTarget.value))} />
    </div>
    {#if compactAt > 0 && compactAt < 20480}
      <Note tone="warn">
        Le contexte sera résumé dès {int(compactAt)} jetons. Après un résumé, il en reste déjà environ 7 000
        (consignes et résumé) : le modèle risque de résumer en boucle sans avancer. Augmentez le contexte
        ou baissez la réflexion max.
      </Note>
    {/if}
  </Section>

  <Section title="Avancé" text="Nom, placement sur les cartes et options passées telles quelles à llama-server.">
    <div class="g2">
      <Input label="Nom" bind:value={name} maxlength={120} />
      <Input label="Marge VRAM par carte" unit="Mio" type="number" min="0" max="16384" step="256" bind:value={s.fit_target_mib} />
      <Input label="Répartition manuelle" placeholder="ex. 16,12" hint="Vide = automatique (--fit)" bind:value={split} />
      <Input label="Options llama-server en plus" class="mono-in" placeholder="ex. --no-mmap" bind:value={extra} />
    </div>
  </Section>
</div>

<style>
  .pe { display: flex; flex-direction: column; min-width: 0; }
  .top {
    position: sticky;
    top: 0;
    z-index: 2;
    display: flex;
    align-items: flex-start;
    gap: var(--s4);
    padding: var(--s5) var(--s6);
    background: var(--bg);
    border-bottom: 1px solid var(--line);
  }
  .ttl { flex: 1; min-width: 0; }
  h2 { margin: 0; font-size: var(--fs-xl); font-weight: 600; line-height: 1.3; }
  .sub { margin: var(--s1) 0 0; color: var(--muted); font-size: var(--fs-sm); overflow-wrap: anywhere; }
  .acts { display: flex; gap: var(--s2); align-items: center; flex-wrap: wrap; justify-content: flex-end; }
  .err { padding: var(--s3) var(--s6) 0; }
  .verdict { margin-top: var(--s3); }
  .muted { color: var(--muted); margin: 0; }
  .hint { margin: 0; color: var(--muted); font-size: var(--fs-sm); }
  .g4 { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: var(--s4); }
  .g2 { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: var(--s4); }
  .switches { display: flex; flex-wrap: wrap; gap: var(--s2) var(--s6); }
  .field { display: flex; flex-direction: column; gap: var(--s2); }
  .lab { color: var(--muted); font-size: var(--fs-sm); }
  .samp { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: var(--s5) var(--s4); }
  .sk { display: flex; flex-direction: column; gap: var(--s1); min-width: 0; }
  .sk :global(.mono-in input), .g2 :global(.mono-in input) { font-family: var(--mono); }
  .src { display: flex; flex-wrap: wrap; align-items: center; gap: var(--s2); color: var(--muted); font-size: var(--fs-sm); }
  .ext { display: inline-flex; align-items: center; gap: 2px; }
  .exc summary { cursor: pointer; color: var(--accent); font-size: var(--fs-sm); width: fit-content; }
  .exc pre {
    margin: var(--s2) 0 0;
    padding: var(--s2) var(--s3);
    max-height: 200px;
    overflow: auto;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: var(--r);
    color: var(--muted);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .pick { display: flex; flex-wrap: wrap; align-items: center; gap: var(--s2); font-size: var(--fs-sm); margin-top: var(--s1); }
  .chip {
    height: 28px;
    min-height: 0;
    padding: 0 var(--s2);
    border: 1px solid var(--line-strong);
    border-radius: var(--r);
    background: var(--raised);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-sm);
    font-weight: 400;
    font-variant-numeric: tabular-nums;
    filter: none;
    cursor: pointer;
  }
  .chip:hover { border-color: var(--accent); color: var(--accent); }

  @media (max-width: 1279px) {
    .g4 { grid-template-columns: repeat(2, minmax(0, 1fr)); }
    .samp { grid-template-columns: minmax(0, 1fr); }
  }
  @media (max-width: 767px) {
    .top { position: static; flex-direction: column; padding: var(--s4); }
    .acts { justify-content: flex-start; width: 100%; }
    .acts :global(.btn) { height: var(--touch); min-width: var(--touch); }
    .acts :global(.btn.primary) { order: -1; flex-basis: 100%; }
    .err { padding: var(--s3) var(--s4) 0; }
    .g4, .g2 { grid-template-columns: minmax(0, 1fr); }
    .chip { height: var(--touch); padding: 0 var(--s3); }
    .switches :global(.sw) { height: var(--touch); }
    .pe :global(.box), .pe :global(.sel) { height: var(--touch); }
  }
</style>
