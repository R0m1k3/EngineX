<script lang="ts">
  import { browserStatus, message, type BrowserStatus } from '../../lib/api';

  let { sessionId }: { sessionId: string } = $props();

  let st = $state<BrowserStatus | null>(null);
  let open = $state(false);
  let taken = $state(false);
  let src = $state('');
  let url = $state('');
  let typed = $state('');
  let error = $state('');
  let connected = $state(false);
  let size = { w: 1280, h: 800 };
  let img: HTMLImageElement | null = $state(null);
  let ws: WebSocket | null = null;

  const send = (v: unknown) => {
    if (ws?.readyState === WebSocket.OPEN) ws.send(JSON.stringify(v));
  };

  function connect() {
    const proto = location.protocol === 'https:' ? 'wss' : 'ws';
    const sock = new WebSocket(`${proto}://${location.host}/api/sessions/${sessionId}/browser/live`);
    ws = sock;
    sock.onopen = () => (connected = true);
    sock.onclose = () => {
      connected = false;
      if (ws === sock) ws = null;
    };
    sock.onmessage = (m) => {
      const v = JSON.parse(m.data);
      if (v.type === 'status') {
        open = v.open;
        taken = v.taken;
        if (v.url) url = v.url;
        if (!v.open) src = '';
      } else if (v.type === 'frame') {
        src = `data:image/jpeg;base64,${v.data}`;
        if (v.w && v.h) size = { w: v.w, h: v.h };
      } else if (v.type === 'url') {
        url = v.url;
      } else if (v.type === 'error') {
        error = v.error;
      }
    };
  }

  $effect(() => {
    browserStatus()
      .then((s) => {
        st = s;
        if (s.enabled) connect();
      })
      .catch((e) => (error = message(e)));
    return () => ws?.close();
  });

  // Coordonnées de la page (l'image est réduite à la largeur du panneau).
  function at(e: MouseEvent) {
    const r = img!.getBoundingClientRect();
    return { x: Math.round(((e.clientX - r.left) * size.w) / r.width), y: Math.round(((e.clientY - r.top) * size.h) / r.height) };
  }
  const mouse = (action: string) => (e: MouseEvent) => {
    if (!taken) return;
    e.preventDefault();
    send({ type: 'mouse', action, ...at(e) });
  };
  function wheel(e: WheelEvent) {
    if (!taken) return;
    e.preventDefault();
    send({ type: 'wheel', ...at(e), dx: e.deltaX, dy: e.deltaY });
  }
  function key(e: KeyboardEvent) {
    if (!taken) return;
    e.preventDefault();
    if (e.key.length === 1 && !e.ctrlKey && !e.metaKey) send({ type: 'text', text: e.key });
    else send({ type: 'key', key: e.key });
  }
  function paste(e: ClipboardEvent) {
    if (!taken) return;
    e.preventDefault();
    const t = e.clipboardData?.getData('text') ?? '';
    if (t) send({ type: 'text', text: t });
  }
  function go(e: Event) {
    e.preventDefault();
    if (typed.trim()) send({ type: 'navigate', url: typed.trim() });
    typed = '';
  }
</script>

<div class="browser">
  {#if st && !st.enabled}
    <p class="muted small">
      Pas de navigateur sur ce serveur : ajoutez le service <code>browser</code> de la stack et
      <code>ENGINEX_BROWSER_URL</code> (voir le README).
    </p>
  {:else if st && st.ok === false}
    <p class="error small">Navigateur injoignable : {st.error}</p>
  {/if}
  {#if error}<p class="error small">{error}</p>{/if}
  {#if st?.enabled}
    <div class="row small top">
      {#if open}
        <button class="link" onclick={() => send({ type: 'take', on: !taken })}>{taken ? 'Rendre la main à l’IA' : 'Prendre la main'}</button>
        {#if taken}
          <button class="link" onclick={() => send({ type: 'back' })}>← Retour</button>
          <form onsubmit={go} class="go"><input placeholder="Adresse" bind:value={typed} /></form>
        {/if}
      {:else if connected}
        <button class="link" onclick={() => send({ type: 'open' })}>Ouvrir un onglet</button>
      {/if}
      {#if !connected}<button class="link" onclick={connect}>Reconnecter</button>{/if}
    </div>
    {#if url}<p class="muted small url" title={url}>{url}</p>{/if}
    {#if open && src}
      <!-- Écran distant : souris et clavier relayés à la page. -->
      <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
      <div
        class="screen"
        class:taken
        role="application"
        aria-label="Page du navigateur de la session"
        tabindex="0"
        onmousedown={mouse('down')}
        onmouseup={mouse('up')}
        onmousemove={(e) => taken && e.buttons && mouse('move')(e)}
        onwheel={wheel}
        onkeydown={key}
        onpaste={paste}
      >
        <img bind:this={img} {src} alt="" draggable="false" />
      </div>
      <p class="muted small">
        {taken
          ? 'Vous avez la main : cliquez dans la page, tapez au clavier (cliquez d’abord l’image). L’IA attend que vous la rendiez.'
          : 'L’IA pilote cet onglet. « Prendre la main » pour vous connecter à un site ou l’aider.'}
      </p>
    {:else if open}
      <p class="muted small">En attente de la première image…</p>
    {:else if connected}
      <p class="muted small">Aucun onglet : l’IA en ouvre un à son premier outil browser_*, ou ouvrez-le pour vous connecter à un site à l’avance (les cookies sont gardés).</p>
    {/if}
  {/if}
</div>

<style>
  .browser { display: flex; flex-direction: column; gap: var(--s2); height: 100%; min-height: 0; overflow: auto; }
  .top { gap: var(--s3); flex-wrap: wrap; align-items: center; }
  .go { flex: 1; min-width: 10em; }
  .go input { width: 100%; }
  .url { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; margin: 0; }
  .screen { border: 1px solid var(--line, #ddd); border-radius: 6px; outline: none; line-height: 0; }
  .screen.taken { border-color: var(--accent, #c60); box-shadow: 0 0 0 2px var(--accent, #c60); cursor: pointer; }
  img { width: 100%; height: auto; user-select: none; border-radius: 6px; }
</style>
