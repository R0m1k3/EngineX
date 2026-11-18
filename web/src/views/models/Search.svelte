<script lang="ts">
  import {
    hfSearch, hfRepo, hfFit, startDownload, message, size, VERDICT,
    type RepoHit, type RepoInfo, type FitResult,
  } from '../../lib/api';

  let text = $state('');
  let hits = $state<RepoHit[]>([]);
  let info = $state<RepoInfo | null>(null);
  let error = $state('');
  let notice = $state('');
  let busy = $state(false);
  let ctx = $state(65536);
  let kv = $state('q8_0');
  let mmproj = $state<string | null>(null);
  let fits = $state<Record<string, FitResult | string>>({});

  async function search(e: SubmitEvent) {
    e.preventDefault();
    if (!text.trim()) return;
    busy = true;
    error = '';
    info = null;
    try {
      hits = await hfSearch(text.trim());
    } catch (err) {
      error = message(err);
    } finally {
      busy = false;
    }
  }

  async function open(repo: string) {
    error = '';
    notice = '';
    fits = {};
    try {
      info = await hfRepo(repo);
      mmproj = info.mmproj[0]?.name ?? null;
      loadFits();
    } catch (err) {
      error = message(err);
    }
  }

  async function loadFits() {
    if (!info) return;
    const r = info;
    fits = {};
    for (const g of r.groups) {
      hfFit(r.repo, g.files[0].name, ctx, kv, mmproj ?? undefined)
        .then((f) => (fits[g.quant] = f))
        .catch((err) => (fits[g.quant] = message(err)));
    }
  }

  async function download(quant: string) {
    if (!info) return;
    notice = '';
    error = '';
    try {
      await startDownload(info.repo, quant, mmproj);
      notice = `${quant} ajouté aux téléchargements.`;
    } catch (err) {
      error = message(err);
    }
  }

  const date = (s: string | null) => (s ? new Date(s).toLocaleDateString('fr-FR') : '');
</script>

<form class="row gap" onsubmit={search}>
  <input class="grow" placeholder="Chercher un modèle GGUF sur Hugging Face (ex. Qwen3 32B)" bind:value={text} />
  <button type="submit" disabled={busy}>Chercher</button>
</form>

{#if error}<p class="error">{error}</p>{/if}
{#if notice}<p class="ok">{notice} <a href="#models/downloads">Voir</a></p>{/if}

{#if info}
  <section class="card">
    <div class="row wrap">
      <h3>{info.repo}</h3>
      <button class="link" onclick={() => (info = null)}>Retour aux résultats</button>
    </div>
    {#if info.base_model}<p class="muted small">Modèle de base : {info.base_model}</p>{/if}
    {#if info.gated}<p class="error">Accès restreint : non téléchargeable sans compte.</p>{/if}
    <div class="actions">
      <label>Contexte
        <select bind:value={ctx} onchange={loadFits}>
          <option value={32768}>32k</option><option value={65536}>64k</option><option value={131072}>128k</option>
        </select>
      </label>
      <label>KV
        <select bind:value={kv} onchange={loadFits}>
          <option value="q8_0">q8_0</option><option value="f16">f16</option><option value="q4_0">q4_0</option>
        </select>
      </label>
      {#if info.mmproj.length}
        <label>Vision (mmproj)
          <select bind:value={mmproj} onchange={loadFits}>
            <option value={null}>sans</option>
            {#each info.mmproj as m}<option value={m.name}>{m.name} ({size(m.size)})</option>{/each}
          </select>
        </label>
      {/if}
    </div>
    <table>
      <thead><tr><th>Quantification</th><th>Taille</th><th>Place en VRAM</th><th></th></tr></thead>
      <tbody>
        {#each info.groups as g}
          {@const f = fits[g.quant]}
          <tr>
            <td>{g.quant}{#if g.files.length > 1}<span class="muted small"> · {g.files.length} tranches</span>{/if}</td>
            <td>{size(g.total_size)}</td>
            <td>
              {#if f === undefined}<span class="muted">…</span>
              {:else if typeof f === 'string'}<span class="muted small">{f}</span>
              {:else}
                <span class="verdict v-{f.report.verdict}">{VERDICT[f.report.verdict]}</span>
                {#if f.report.cards.length}
                  <span class="muted small">
                    {f.report.cards.map((c, i) => `CUDA${i} ${(c.need_mib / 1024).toFixed(1)}/${(c.avail_mib / 1024).toFixed(1)}`).join(' · ')} Gio
                  </span>
                {/if}
              {/if}
            </td>
            <td><button disabled={info.gated} onclick={() => download(g.quant)}>Télécharger</button></td>
          </tr>
        {/each}
      </tbody>
    </table>
    <p class="muted small">Estimation prudente ; les modèles à fenêtre glissante (Gemma 3…) sont surestimés.</p>
  </section>
{:else if hits.length}
  <table>
    <thead><tr><th>Dépôt</th><th>Téléchargements</th><th>Mis à jour</th><th></th></tr></thead>
    <tbody>
      {#each hits as h}
        <tr>
          <td><button class="link" onclick={() => open(h.repo)}>{h.repo}</button>{#if h.gated} <span class="muted small">accès restreint</span>{/if}</td>
          <td>{h.downloads.toLocaleString('fr-FR')}</td>
          <td>{date(h.updated)}</td>
          <td><button class="secondary" onclick={() => open(h.repo)}>Ouvrir</button></td>
        </tr>
      {/each}
    </tbody>
  </table>
{/if}
