<script lang="ts">
  import { gb, int, octets } from '../../lib/ui/format';
  import SearchIcon from '@lucide/svelte/icons/search';
  import ArrowLeft from '@lucide/svelte/icons/arrow-left';
  import Download from '@lucide/svelte/icons/download';
  import Lock from '@lucide/svelte/icons/lock';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import Button from '../../lib/ui/Button.svelte';
  import Input from '../../lib/ui/Input.svelte';
  import Select from '../../lib/ui/Select.svelte';
  import Badge from '../../lib/ui/Badge.svelte';
  import Icon from '../../lib/ui/Icon.svelte';
  import EmptyState from '../../lib/ui/EmptyState.svelte';
  import { toast } from '../../lib/ui/toast.svelte';
  import type { Tone } from '../../lib/ui/types';
  import Note from '../settings/Note.svelte';
  import {
    hfSearch, hfRepo, hfFit, startDownload, message, VERDICT,
    type RepoHit, type RepoInfo, type FitResult, type Verdict,
  } from '../../lib/api';

  let text = $state('');
  let hits = $state<RepoHit[]>([]);
  let searched = $state(false);
  let info = $state<RepoInfo | null>(null);
  let error = $state('');
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
      searched = true;
    } catch (err) {
      error = message(err);
    } finally {
      busy = false;
    }
  }

  async function open(repo: string) {
    error = '';
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
    error = '';
    try {
      await startDownload(info.repo, quant, mmproj);
      toast(`${quant} ajouté aux téléchargements.`, 'ok');
    } catch (err) {
      error = message(err);
    }
  }

  const date = (s: string | null) => (s ? new Date(s).toLocaleDateString('fr-FR') : '');
  const tone: Record<Verdict, Tone> = { fits: 'ok', tight: 'warn', offload: 'warn', no_fit: 'danger', unknown: 'neutral' };
  const mmOptions = $derived([
    { value: '', label: 'sans' },
    ...(info?.mmproj ?? []).map((m) => ({ value: m.name, label: `${m.name} (${octets(m.size)})` })),
  ]);
</script>

<div class="hf">
  <form class="bar" onsubmit={search} role="search">
    <Input icon={SearchIcon} class="q" placeholder="Chercher un modèle GGUF sur Hugging Face (ex. Qwen3 32B)" bind:value={text} aria-label="Chercher sur Hugging Face" />
    <Button type="submit" variant="primary" {busy}>Chercher</Button>
  </form>

  {#if error}<Note tone="danger">{error}</Note>{/if}

  {#if info}
    {@const r = info}
    <section class="repo" aria-label={r.repo}>
      <button type="button" class="back" onclick={() => (info = null)}><Icon icon={ArrowLeft} size={16} />Retour aux résultats</button>
      <h2 class="rname">{r.repo}</h2>
      {#if r.base_model}<p class="muted">Modèle de base : {r.base_model}</p>{/if}
      {#if r.gated}<Note tone="warn">Accès restreint : non téléchargeable sans compte.</Note>{/if}

      <div class="opts">
        <Select label="Contexte" bind:value={ctx} onchange={loadFits} options={[
          { value: 32768, label: '32k' }, { value: 65536, label: '64k' }, { value: 131072, label: '128k' },
        ]} />
        <Select label="Cache KV" bind:value={kv} onchange={loadFits} options={[
          { value: 'q8_0', label: 'q8_0' }, { value: 'f16', label: 'f16' }, { value: 'q4_0', label: 'q4_0' },
        ]} />
        {#if r.mmproj.length}
          <Select label="Vision (mmproj)" value={mmproj ?? ''} options={mmOptions}
            onchange={(e) => { mmproj = e.currentTarget.value || null; loadFits(); }} />
        {/if}
      </div>

      <ul class="quants" aria-label="Quantifications">
        {#each r.groups as g (g.quant)}
          {@const f = fits[g.quant]}
          <li>
            <div class="qn">
              <span class="mono">{g.quant}</span>
              <span class="muted num">{octets(g.total_size)}{#if g.files.length > 1}, {g.files.length} tranches{/if}</span>
            </div>
            <div class="qfit">
              {#if f === undefined}<span class="muted">Estimation…</span>
              {:else if typeof f === 'string'}<span class="muted">{f}</span>
              {:else}
                <Badge tone={tone[f.report.verdict]} dot>{VERDICT[f.report.verdict]}</Badge>
                {#if f.report.cards.length}
                  <span class="muted num">
                    {f.report.cards.map((c, i) => `CUDA${i} ${gb(c.need_mib)} / ${gb(c.avail_mib)}`).join(', ')} Go
                  </span>
                {/if}
              {/if}
            </div>
            <Button icon={Download} disabled={r.gated} onclick={() => download(g.quant)}>Télécharger</Button>
          </li>
        {/each}
      </ul>
      <p class="muted foot">
        Estimation prudente ; les modèles à fenêtre glissante (Gemma 3…) sont surestimés.
        Suivre l’avancement dans <a href="#models/downloads">Téléchargements</a>.
      </p>
    </section>
  {:else if hits.length}
    <ul class="hits" aria-label="Résultats">
      {#each hits as h (h.repo)}
        <li>
          <button type="button" class="hit" onclick={() => open(h.repo)}>
            <span class="hname">
              <span class="ell">{h.repo}</span>
              {#if h.gated}<Badge tone="warn"><Icon icon={Lock} size={14} />accès restreint</Badge>{/if}
            </span>
            <span class="hmeta num">{int(h.downloads)} téléchargements{#if h.updated}, mis à jour le {date(h.updated)}{/if}</span>
            <span class="chev"><Icon icon={ChevronRight} size={18} /></span>
          </button>
        </li>
      {/each}
    </ul>
  {:else if searched}
    <EmptyState icon={SearchIcon} title="Aucun dépôt GGUF trouvé" text="Essayez un nom plus court : la famille et la taille suffisent (Qwen3 32B)." />
  {:else}
    <EmptyState icon={SearchIcon} title="Chercher un modèle" text="Les dépôts GGUF de Hugging Face s’affichent ici, avec la place que chaque quantification prendrait sur vos cartes." />
  {/if}
</div>

<style>
  .hf { display: flex; flex-direction: column; gap: var(--s4); padding: var(--s6); max-width: 1000px; }
  .bar { display: flex; gap: var(--s2); align-items: flex-end; }
  .bar :global(.f) { flex: 1; }
  .muted { color: var(--muted); margin: 0; font-size: var(--fs-sm); }
  .hits, .quants { list-style: none; margin: 0; padding: 0; border-top: 1px solid var(--line); }
  .hits li, .quants li { border-bottom: 1px solid var(--line); }
  .hit {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 2px var(--s4);
    width: 100%;
    min-height: 0;
    padding: var(--s3) var(--s2);
    border: 0;
    border-radius: 0;
    background: none;
    color: var(--text);
    font: inherit;
    font-weight: 400;
    text-align: left;
    cursor: pointer;
    filter: none;
  }
  .hit:hover { background: var(--raised); }
  .hname { display: flex; align-items: center; gap: var(--s2); min-width: 0; font-weight: 600; }
  .hmeta { grid-column: 1; color: var(--muted); font-size: var(--fs-sm); }
  .chev { grid-column: 2; grid-row: 1 / span 2; align-self: center; color: var(--faint); }
  .repo { display: flex; flex-direction: column; gap: var(--s3); }
  .back {
    display: inline-flex;
    align-items: center;
    gap: var(--s2);
    width: fit-content;
    min-height: 0;
    height: 32px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--muted);
    font: inherit;
    font-size: var(--fs-sm);
    font-weight: 400;
    filter: none;
    cursor: pointer;
  }
  .back:hover { color: var(--text); }
  .rname { margin: 0; font-size: var(--fs-xl); font-weight: 600; overflow-wrap: anywhere; }
  .opts { display: flex; flex-wrap: wrap; gap: var(--s4); }
  .opts > :global(*) { min-width: 140px; }
  .quants li { display: grid; grid-template-columns: 200px minmax(0, 1fr) auto; align-items: center; gap: var(--s4); padding: var(--s3) 0; }
  .qn { display: flex; flex-direction: column; gap: 2px; }
  .qn .mono { font-weight: 600; }
  .qfit { display: flex; flex-wrap: wrap; align-items: center; gap: var(--s2); min-width: 0; }
  .foot { margin-top: var(--s2); }

  @media (max-width: 767px) {
    .hf { padding: var(--s4); }
    .bar :global(.box) { height: var(--touch); }
    .bar :global(.btn) { height: var(--touch); }
    .hit { min-height: 60px; }
    .quants li { grid-template-columns: minmax(0, 1fr) auto; }
    .qfit { grid-column: 1 / -1; grid-row: 2; }
    .quants :global(.btn) { height: var(--touch); }
    .opts :global(.sel) { height: var(--touch); }
  }
</style>
