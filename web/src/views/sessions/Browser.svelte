<script lang="ts">
  import { browserStatus, message, type BrowserStatus } from '../../lib/api';
  import Hand from '@lucide/svelte/icons/hand';
  import ArrowLeft from '@lucide/svelte/icons/arrow-left';
  import Plus from '@lucide/svelte/icons/plus';
  import Plug from '@lucide/svelte/icons/plug';
  import GlobeIcon from '@lucide/svelte/icons/globe';
  import Button from '../../lib/ui/Button.svelte';
  import IconButton from '../../lib/ui/IconButton.svelte';
  import EmptyState from '../../lib/ui/EmptyState.svelte';
  import StatusDot from '../../lib/ui/StatusDot.svelte';

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
    <div class="pad">
      <EmptyState icon={GlobeIcon} title="Pas de navigateur sur ce serveur"
        text="Ajoutez le service browser de la stack et la variable ENGINEX_BROWSER_URL (voir le README)." />
    </div>
  {:else if st && st.ok === false}
    <p class="error small pad">Navigateur injoignable : {st.error}</p>
  {/if}
  {#if error}<p class="error small pad">{error}</p>{/if}
  {#if st?.enabled}
    <div class="tb">
      <StatusDot kind={!connected ? 'off' : taken ? 'warn' : open ? 'ok' : 'off'} label={connected ? 'Connecté' : 'Déconnecté'} />
      {#if open}
        {#if taken}
          <IconButton icon={ArrowLeft} label="Page précédente" size="sm" onclick={() => send({ type: 'back' })} />
          <form onsubmit={go} class="go"><input placeholder={url || 'Adresse'} aria-label="Adresse" bind:value={typed} /></form>
        {:else}
          <span class="grow url ell" title={url}>{url}</span>
        {/if}
        <Button size="sm" variant={taken ? 'primary' : 'secondary'} icon={Hand} onclick={() => send({ type: 'take', on: !taken })}>
          {taken ? 'Rendre la main à l’IA' : 'Prendre la main'}
        </Button>
      {:else}
        <span class="grow url ell" title={url}>{url}</span>
        {#if connected}<Button size="sm" icon={Plus} onclick={() => send({ type: 'open' })}>Ouvrir un onglet</Button>{/if}
      {/if}
      {#if !connected}<Button size="sm" icon={Plug} onclick={connect}>Reconnecter</Button>{/if}
    </div>
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
      <p class="note" class:on={taken}>
        {taken
          ? 'Vous avez la main : cliquez dans la page, tapez au clavier (cliquez d’abord l’image). L’IA attend que vous la rendiez.'
          : 'L’IA pilote cet onglet. « Prendre la main » pour vous connecter à un site ou l’aider.'}
      </p>
    {:else if open}
      <p class="muted small pad">En attente de la première image…</p>
    {:else if connected}
      <div class="pad">
        <EmptyState icon={GlobeIcon} title="Aucun onglet ouvert"
          text="L’IA en ouvre un à son premier outil browser_*. Vous pouvez aussi l’ouvrir pour vous connecter à un site à l’avance (les cookies sont gardés)." />
      </div>
    {/if}
  {/if}
</div>

<style>
  .browser { display: flex; flex-direction: column; height: 100%; min-height: 0; overflow: auto; }
  .tb { display: flex; align-items: center; gap: var(--s2); min-height: 40px; padding: var(--s1) var(--s2) var(--s1) var(--s3); border-bottom: 1px solid var(--line); font-size: var(--fs-sm); flex: none; flex-wrap: wrap; }
  .go { flex: 1; min-width: 10em; }
  .go input { width: 100%; height: 28px; }
  .url { color: var(--faint); font-family: var(--mono); font-size: var(--fs-xs); }
  .pad { margin: 0; padding: var(--s3) var(--s4); }
  .screen { margin: var(--s3); border: 1px solid var(--line-strong); border-radius: var(--r); outline: none; line-height: 0; overflow: hidden; }
  .screen:focus-visible { border-color: var(--accent); }
  .screen.taken { border-color: var(--accent); outline: 2px solid var(--accent); outline-offset: 0; cursor: pointer; }
  img { width: 100%; height: auto; user-select: none; }
  .note { margin: 0 var(--s3) var(--s3); font-size: var(--fs-sm); color: var(--muted); }
  .note.on { color: var(--accent); }
  @media (max-width: 767px) { .go input { height: var(--touch); } }
</style>
