<script lang="ts">
  import {
    savePreset, activatePreset, message, engine, hasMtp, SAMPLING,
    type Preset, type LocalModel, type PresetSettings, type SamplingKey, type CandidateSet, type EngineInfo,
  } from '../../lib/api';

  let { preset, model, models = [], saved }: {
    preset: Preset; model: LocalModel | undefined; models?: LocalModel[]; saved: () => void;
  } = $props();

  // Copie modifiable du preset (l'original reste celui du serveur).
  const init = () => $state.snapshot(preset);
  let name = $state(init().name);
  let s = $state<PresetSettings>(init().settings);
  let split = $state(init().settings.tensor_split?.join(',') ?? '');
  let extra = $state(init().settings.extra_args.join(' '));
  let error = $state('');
  let notice = $state('');
  let caps = $state<EngineInfo['caps']>(null);
  engine().then((e) => (caps = e.caps)).catch(() => {});

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

  async function save() {
    error = '';
    notice = '';
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
    try {
      const r = await savePreset(preset.id, name, settings);
      if (r.reload_needed) {
        if (confirm('Ces changements demandent de recharger le modèle. Recharger maintenant ?')) {
          await activatePreset(preset.id, true);
          notice = 'Enregistré, rechargement lancé (après la fin des requêtes en cours).';
        } else {
          notice = 'Enregistré : pris en compte au prochain chargement.';
        }
      } else {
        notice = 'Enregistré : appliqué dès la prochaine requête.';
      }
      saved();
    } catch (e) {
      error = message(e);
    }
  }
</script>

<div class="editor">
  <div class="fields">
    <label>Nom <input bind:value={name} maxlength="120" /></label>
    <label>Contexte (jetons)
      <input type="number" min="512" max={model?.meta.context_length ?? 1048576} step="1024" bind:value={s.ctx} />
    </label>
    <label>Type de KV
      <select bind:value={s.kv_type}><option value="q8_0">q8_0</option><option value="f16">f16</option><option value="q4_0">q4_0</option></select>
    </label>
    <label>Requêtes en parallèle <input type="number" min="1" max="8" bind:value={s.parallel} /></label>
    <label>Marge VRAM par carte (Mio) <input type="number" min="0" max="16384" step="256" bind:value={s.fit_target_mib} /></label>
    <label>Répartition manuelle (vide = automatique --fit)
      <input placeholder="ex. 16,12" bind:value={split} />
    </label>
    <label>Raisonnement
      <select bind:value={s.reasoning}><option value="on">activé</option><option value="off">désactivé</option></select>
    </label>
    <label class="check"><input type="checkbox" bind:checked={s.flash_attn} /> Flash attention</label>
    {#if model?.mmproj}<label class="check"><input type="checkbox" bind:checked={s.mmproj} /> Vision (mmproj)</label>{/if}
    <label>Options llama-server en plus <input placeholder="ex. --no-mmap" bind:value={extra} /></label>
  </div>

  <h4>Accélération <span class="muted small">(rechargement nécessaire)</span></h4>
  {#if caps && !caps.spec}<p class="small error">Ce llama-server n’a pas le décodage spéculatif (MTP, brouillon) : options ignorées.</p>{/if}
  <div class="fields">
    <label>Brouillon (décodage spéculatif)
      <select value={draftValue} onchange={(e) => setDraft(e.currentTarget.value)}>
        <option value="none">aucun</option>
        <option value="mtp" disabled={!mtpOk}>MTP du modèle{mtpOk ? '' : ' (absent de ce GGUF)'}</option>
        {#each drafts as d}<option value={String(d.id)}>{d.repo.split('/').pop()} ({d.file.split('/').pop()})</option>{/each}
      </select>
    </label>
    {#if s.spec.draft !== 'none'}
      <label>Jetons proposés par étape <input type="number" min="1" max="16" bind:value={s.spec.draft_max} /></label>
    {/if}
    <label class="check" title="Propose la suite à partir des répétitions du contexte (code, fichiers relus). Sans VRAM.">
      <input type="checkbox" bind:checked={s.spec.ngram} /> Brouillon ngram (répétitions)
    </label>
    {#if isMoe}
      <label>Experts MoE ({model?.meta.expert_count} experts, {model?.meta.expert_used_count ?? '?'} actifs)
        <select bind:value={s.moe.mode}>
          <option value="auto">automatique (--fit)</option>
          <option value="all_cpu">tous en RAM</option>
          <option value="layers">N premières couches en RAM</option>
        </select>
      </label>
      {#if s.moe.mode === 'layers'}
        <label>Couches d’experts en RAM <input type="number" min="1" max={model?.meta.block_count ?? 1024} bind:value={s.moe.cpu_layers} /></label>
      {/if}
      {#if s.moe.mode !== 'auto'}
        <label>Cache GPU des experts (Mio, 0 = sans) <input type="number" min="0" max="65536" step="512" bind:value={s.moe.cache_mib} /></label>
      {/if}
    {/if}
    <label>Lot logique -b (vide = 2048)
      <input type="number" min="32" max="16384" step="512" value={s.batch ?? ''} oninput={(e) => (s.batch = optNum(e.currentTarget.value))} />
    </label>
    <label>Lot physique -ub (vide = 512)
      <input type="number" min="32" max="16384" step="256" value={s.ubatch ?? ''} oninput={(e) => (s.ubatch = optNum(e.currentTarget.value))} />
    </label>
  </div>
  {#if s.spec.ngram && s.moe.mode !== 'auto'}
    <p class="small muted">ngram est retiré au lancement tant que des experts sont en RAM : la vérification passerait par le CPU.</p>
  {/if}
  {#if s.spec.draft !== 'none'}
    <p class="small muted">Si le brouillon empêche le chargement, le modèle est relancé sans et la barre du moteur le signale.</p>
  {/if}

  <h4>Échantillonnage <span class="muted small">(envoyé avec chaque requête, sans recharger)</span></h4>
  {#if preset.sources.error}<p class="error small">Sources non lues : {preset.sources.error}</p>{/if}
  <table>
    <tbody>
      {#each SAMPLING as k}
        {@const o = preset.sources.origins[k]}
        {@const options = preset.sources.to_choose[k]}
        <tr>
          <td><code>{k}</code></td>
          <td><input class="num" value={s.sampling[k] ?? ''} oninput={(e) => setSampling(k, e.currentTarget.value)} placeholder="défaut" /></td>
          <td class="small">
            <span class="muted">{sourceLabel[o?.source ?? 'defaut']}</span>
            {#if o?.url}· <a href={o.url} target="_blank" rel="noreferrer noopener">page</a>{/if}
            {#if o?.excerpt}<div class="excerpt">{o.excerpt}</div>{/if}
            {#if options}
              <div class="choose">
                À choisir :
                {#each options as c}
                  <button class="link" title={c.excerpt} onclick={() => choose(k, c)}>{c.values[k]} ({labelName[c.label]})</button>
                {/each}
              </div>
            {/if}
          </td>
        </tr>
      {/each}
    </tbody>
  </table>

  {#if error}<p class="error">{error}</p>{/if}
  {#if notice}<p class="ok">{notice}</p>{/if}
  <div class="actions"><button onclick={save}>Enregistrer</button></div>
</div>
