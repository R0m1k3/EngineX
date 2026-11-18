<script lang="ts">
  import {
    savePreset, activatePreset, message, SAMPLING,
    type Preset, type LocalModel, type PresetSettings, type SamplingKey, type CandidateSet,
  } from '../../lib/api';

  let { preset, model, saved }: { preset: Preset; model: LocalModel | undefined; saved: () => void } = $props();

  // Copie modifiable du preset (l'original reste celui du serveur).
  const init = () => $state.snapshot(preset);
  let name = $state(init().name);
  let s = $state<PresetSettings>(init().settings);
  let split = $state(init().settings.tensor_split?.join(',') ?? '');
  let extra = $state(init().settings.extra_args.join(' '));
  let error = $state('');
  let notice = $state('');

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
