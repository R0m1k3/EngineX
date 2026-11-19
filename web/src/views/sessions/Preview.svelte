<script lang="ts">
  import { message, previewOpen, previewStatus, type PreviewStatus } from '../../lib/api';

  let { sessionId }: { sessionId: string } = $props();

  let st = $state<PreviewStatus | null>(null);
  let port = $state<number | null>(null);
  let url = $state('');
  let frame = $state(0);
  let error = $state('');

  async function load() {
    try {
      st = await previewStatus(sessionId);
      error = '';
      // Un seul port : ouvert tout de suite.
      if (!port && st.running && st.ports.length === 1) await show(st.ports[0]);
    } catch (e) {
      error = message(e);
    }
  }

  async function show(p: number) {
    error = '';
    try {
      const r = await previewOpen(sessionId, p);
      port = p;
      url = r.url;
      frame++;
    } catch (e) {
      error = message(e);
    }
  }

  $effect(() => {
    load();
    // Ports relus tant qu'aucune app n'est montrée.
    const t = setInterval(() => {
      if (!port) load();
    }, 5000);
    return () => clearInterval(t);
  });
</script>

<div class="preview">
  <div class="row small top">
    {#if st?.running && st.ports.length}
      <span class="muted">Port</span>
      {#each st.ports as p (p)}
        <button class="link" class:on={p === port} onclick={() => show(p)}>{p}</button>
      {/each}
    {/if}
    <span class="grow"></span>
    {#if url}
      <button class="link" onclick={() => show(port!)} title="Recharger l'app">Recharger</button>
      <a class="link" href={url} target="_blank" rel="noopener">Nouvel onglet ↗</a>
    {/if}
    <button class="link" onclick={load}>Actualiser</button>
  </div>
  {#if error}<p class="error small">{error}</p>{/if}
  {#if st && !st.enabled}
    <p class="muted small">Aperçu désactivé sur ce serveur (ENGINEX_PREVIEW_BIND=off).</p>
  {:else if st && !st.running}
    <p class="muted small">La sandbox est arrêtée. Lancez l’app dans la session (par exemple « lance le serveur de dev »), elle apparaîtra ici.</p>
  {:else if st && !st.ports.length}
    <p class="muted small">Aucune app n’écoute dans la sandbox pour l’instant. Demandez à l’IA de démarrer le serveur de dev, ou lancez-le dans le Terminal.</p>
  {:else if st?.error}
    <p class="error small">{st.error}</p>
  {/if}
  {#if url}
    {#key frame}
      <iframe title="Aperçu de l'app" src={url}></iframe>
    {/key}
    <p class="muted small">Servie sur une adresse à part : l’app n’a pas accès à EngineX. Un aperçu à la fois par navigateur.</p>
  {/if}
</div>

<style>
  .preview { display: flex; flex-direction: column; gap: var(--s2); height: 100%; min-height: 0; }
  .top { gap: var(--s2); flex-wrap: wrap; align-items: center; }
  .grow { flex: 1; }
  .on { font-weight: 600; text-decoration: underline; }
  iframe { flex: 1; min-height: 240px; width: 100%; border: 1px solid var(--line, #ddd); border-radius: 6px; background: #fff; }
</style>
