<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import '@xterm/xterm/css/xterm.css';

  let { sessionId }: { sessionId: string } = $props();

  let host: HTMLDivElement | undefined = $state();
  let phase = $state<'connecting' | 'starting' | 'ready' | 'closed'>('connecting');
  let term: Terminal | null = null;
  let fit: FitAddon | null = null;
  let ws: WebSocket | null = null;
  let observer: ResizeObserver | null = null;
  const enc = new TextEncoder();

  const note = (s: string) => term?.write(`\r\n\x1b[2m${s}\x1b[0m\r\n`);

  function sendSize() {
    if (ws?.readyState === WebSocket.OPEN && term) {
      ws.send(JSON.stringify({ type: 'resize', cols: term.cols, rows: term.rows }));
    }
  }

  function connect() {
    ws?.close();
    phase = 'connecting';
    const proto = location.protocol === 'https:' ? 'wss' : 'ws';
    const sock = new WebSocket(`${proto}://${location.host}/api/sessions/${sessionId}/terminal`);
    sock.binaryType = 'arraybuffer';
    ws = sock;
    sock.onmessage = (ev) => {
      if (ev.data instanceof ArrayBuffer) {
        term?.write(new Uint8Array(ev.data));
        return;
      }
      const m = JSON.parse(ev.data);
      if (m.type === 'status') {
        phase = m.state === 'ready' ? 'ready' : 'starting';
        if (m.state === 'starting') note('Démarrage de la sandbox…');
        if (m.state === 'ready') sendSize();
      } else if (m.type === 'dropped') {
        note(`[${m.bytes} octets de sortie non affichés : affichage trop lent]`);
      } else if (m.type === 'error') {
        note(`Erreur : ${m.message}`);
      } else if (m.type === 'exit') {
        note(m.reason ? `Terminal fermé : ${m.reason}` : `Shell terminé (code ${m.code ?? '?'})`);
      }
    };
    sock.onclose = () => {
      if (ws === sock) phase = 'closed';
    };
  }

  onMount(() => {
    term = new Terminal({
      fontSize: 13,
      fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
      cursorBlink: true,
      scrollback: 5000,
      theme: { background: '#111316', foreground: '#e6e8eb' },
    });
    fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host!);
    fit.fit();
    term.onData((d) => {
      if (ws?.readyState === WebSocket.OPEN) ws.send(enc.encode(d));
    });
    term.onResize(sendSize);
    observer = new ResizeObserver(() => {
      if (host && host.offsetParent !== null) fit?.fit();
    });
    observer.observe(host!);
    connect();
  });

  onDestroy(() => {
    observer?.disconnect();
    const s = ws;
    ws = null;
    s?.close();
    term?.dispose();
  });
</script>

<div class="terminal">
  <div class="row small">
    <span class="muted">
      {phase === 'ready' ? 'Shell dans la sandbox de la session' : phase === 'closed' ? 'Terminal fermé' : 'Connexion…'}
    </span>
    {#if phase === 'closed'}<button class="link" onclick={connect}>Rouvrir</button>{/if}
  </div>
  <div class="xterm-host" bind:this={host}></div>
  <p class="tiny muted">Un seul terminal par session : en ouvrir un autre ferme celui-ci. Les secrets affichés sont masqués.</p>
</div>

<style>
  .terminal { display: flex; flex-direction: column; gap: var(--s2); height: 100%; min-height: 0; }
  .xterm-host { flex: 1; min-height: 240px; background: #111316; border: 1px solid var(--line); border-radius: var(--radius); padding: var(--s1); }
  .tiny { font-size: 12px; margin: 0; }
</style>
