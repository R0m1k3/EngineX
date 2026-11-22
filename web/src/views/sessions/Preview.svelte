<script lang="ts">
  import { message, previewOpen, previewStatus, type PreviewStatus } from '../../lib/api';
  import RotateCw from '@lucide/svelte/icons/rotate-cw';
  import RefreshCw from '@lucide/svelte/icons/refresh-cw';
  import ExternalLink from '@lucide/svelte/icons/external-link';
  import AppWindow from '@lucide/svelte/icons/app-window';
  import IconButton from '../../lib/ui/IconButton.svelte';
  import EmptyState from '../../lib/ui/EmptyState.svelte';
  import Skeleton from '../../lib/ui/Skeleton.svelte';

  // `idle` : texte montré quand rien n'est affichable (sandbox arrêtée, aucune page).
  let { sessionId, idle }: { sessionId: string; idle?: { title: string; text: string } } = $props();

  let st = $state<PreviewStatus | null>(null);
  // Port de l'app montrée ; FILES (0) = fichiers du dossier (page sans serveur).
  const FILES = 0;
  let port = $state<number | null>(null);
  // Page du dossier montrée en mode FILES.
  let page = $state('');
  const pages = $derived(st?.pages ?? []);
  let url = $state('');
  let frame = $state(0);
  let error = $state('');

  async function load() {
    try {
      st = await previewStatus(sessionId);
      error = '';
      // Un seul port : ouvert tout de suite ; aucune app mais un index.html : la page.
      if (port === null && st.running && st.ports.length === 1) await show(st.ports[0]);
      else if (port === null && (!st.running || !st.ports.length) && st.pages?.length) await show(FILES, st.pages[0]);
    } catch (e) {
      error = message(e);
    }
  }

  async function show(p: number, name = '') {
    error = '';
    try {
      const r = await previewOpen(sessionId, p, p === FILES ? name : undefined);
      port = p;
      page = p === FILES ? name : '';
      url = r.url;
      frame++;
    } catch (e) {
      error = message(e);
    }
  }

  $effect(() => {
    load();
    // Ports relus tant qu'aucune app n'est montrée (la page du dossier compte comme aucune).
    const t = setInterval(() => {
      if (port === null || port === FILES) load();
    }, 5000);
    return () => clearInterval(t);
  });
</script>

<div class="preview">
  <div class="tb">
    {#if (st?.running && st.ports.length) || pages.length}
      <span class="faint">{pages.length ? 'Afficher' : 'Port'}</span>
      <div class="ports" role="group" aria-label="Page ou port de l’app">
        {#each pages as n (n)}
          <button class="port" aria-pressed={port === FILES && page === n} title="{n} du dossier, sans serveur" onclick={() => show(FILES, n)}>{n === 'index.html' ? 'Page' : n}</button>
        {/each}
        {#each st?.running ? st.ports : [] as p (p)}
          <button class="port num" aria-pressed={p === port} onclick={() => show(p)}>{p}</button>
        {/each}
      </div>
    {/if}
    <span class="grow url ell" title={url}>{url}</span>
    {#if url}
      <IconButton icon={RotateCw} label="Recharger l’app" size="sm" onclick={() => (port === null ? undefined : show(port, page))} />
      <IconButton icon={ExternalLink} label="Ouvrir dans un nouvel onglet" size="sm" href={url} target="_blank" rel="noopener" />
    {/if}
    <IconButton icon={RefreshCw} label="Relire les ports" size="sm" tip="left" onclick={load} />
  </div>
  {#if error}<p class="error small pad">{error}</p>{/if}
  {#if url}
    <!-- une page ou une app est montrée : pas d'état vide -->
  {:else if !st && !error}<div class="pad"><Skeleton lines={3} /></div>
  {:else if st && !st.enabled}
    <div class="pad"><EmptyState icon={AppWindow} title="Aperçu désactivé" text="L’aperçu est désactivé sur ce serveur (ENGINEX_PREVIEW_BIND=off)." /></div>
  {:else if st && !st.running && idle}
    <div class="pad"><EmptyState icon={AppWindow} title={idle.title} text={idle.text} /></div>
  {:else if st && !st.running}
    <div class="pad"><EmptyState icon={AppWindow} title="Sandbox arrêtée" text="Lancez l’app dans la session (par exemple « lance le serveur de dev ») : elle apparaîtra ici." /></div>
  {:else if st && !st.ports.length}
    <div class="pad"><EmptyState icon={AppWindow} title="Aucune app ne tourne" text="Aucune app n’écoute dans la sandbox pour l’instant. Demandez à l’IA de démarrer le serveur de dev, ou lancez-le dans le Terminal." /></div>
  {:else if st?.error}
    <p class="error small pad">{st.error}</p>
  {/if}
  {#if url}
    {#key frame}
      <iframe title="Aperçu de l'app" src={url}></iframe>
    {/key}
    <p class="note">Servie sur une adresse à part : l’app n’a pas accès à EngineX. Un aperçu à la fois par navigateur.</p>
  {/if}
</div>

<style>
  .preview { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .tb { display: flex; align-items: center; gap: var(--s2); min-height: 36px; padding: 0 var(--s1) 0 var(--s3); border-bottom: 1px solid var(--line); font-size: var(--fs-sm); flex: none; }
  .ports { display: inline-flex; border: 1px solid var(--line-strong); border-radius: var(--r); overflow: hidden; }
  .port { height: 28px; min-height: 0; padding: 0 var(--s2); border: 0; border-radius: 0; background: none; color: var(--muted); font: inherit; font-size: var(--fs-sm); font-weight: 400; filter: none; cursor: pointer; }
  .port + .port { border-left: 1px solid var(--line-strong); }
  .port[aria-pressed='true'] { background: var(--accent-soft); color: var(--accent); font-weight: 600; }
  .url { color: var(--faint); font-family: var(--mono); font-size: var(--fs-xs); }
  .pad { margin: 0; padding: var(--s3) var(--s4); }
  iframe { flex: 1; min-height: 240px; width: 100%; border: 0; background: #fff; }
  .note { margin: 0; padding: var(--s2) var(--s3); border-top: 1px solid var(--line); font-size: var(--fs-xs); color: var(--faint); flex: none; }
  @media (max-width: 767px) { .port { height: 40px; padding: 0 var(--s3); } }
</style>
