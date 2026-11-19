<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import '@xterm/xterm/css/xterm.css';
  import { onThemeChange, token, isDark } from '../../lib/ui/theme.svelte';
  import RotateCw from '@lucide/svelte/icons/rotate-cw';
  import Button from '../../lib/ui/Button.svelte';
  import StatusDot from '../../lib/ui/StatusDot.svelte';

  let { sessionId }: { sessionId: string } = $props();

  let host: HTMLDivElement | undefined = $state();
  let phase = $state<'connecting' | 'starting' | 'ready' | 'closed'>('connecting');
  let term: Terminal | null = null;
  let fit: FitAddon | null = null;
  let ws: WebSocket | null = null;
  let observer: ResizeObserver | null = null;
  const enc = new TextEncoder();
  let unwatch: (() => void) | null = null;
  const FONT = '"JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, monospace';

  // Couleurs du terminal depuis les jetons (app.css), refaites à chaque bascule de thème.
  function palette() {
    const v = (n: string) => token(n);
    const dark = isDark();
    return {
      background: v('bg'),
      foreground: v('text'),
      cursor: v('accent'),
      cursorAccent: v('bg'),
      selectionBackground: v('accent-soft'),
      black: dark ? v('raised') : v('text'),
      red: v('danger'),
      green: v('accent'),
      yellow: v('warn'),
      blue: v('term-blue'),
      magenta: v('term-magenta'),
      cyan: v('term-cyan'),
      white: dark ? v('text') : v('muted'),
      brightBlack: v('faint'),
      brightRed: v('danger'),
      brightGreen: v('add-fg'),
      brightYellow: v('warn'),
      brightBlue: v('term-blue'),
      brightMagenta: v('term-magenta'),
      brightCyan: v('term-cyan'),
      brightWhite: dark ? '#ffffff' : v('text'),
    };
  }

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
      fontSize: 14,
      fontFamily: FONT,
      cursorBlink: true,
      scrollback: 5000,
      theme: palette(),
    });
    unwatch = onThemeChange(() => term && (term.options.theme = palette()));
    // Police embarquée chargée après coup : xterm remesure les cellules.
    document.fonts?.load('14px "JetBrains Mono"').then(() => {
      if (!term) return;
      term.options.fontFamily = FONT;
      fit?.fit();
    }).catch(() => {});
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
    unwatch?.();
    const s = ws;
    ws = null;
    s?.close();
    term?.dispose();
    term = null;
  });
</script>

<div class="terminal">
  <div class="tb">
    <StatusDot kind={phase === 'ready' ? 'ok' : phase === 'closed' ? 'off' : 'warn'} />
    <span class="grow">
      {phase === 'ready' ? 'Shell dans la sandbox de la session' : phase === 'closed' ? 'Terminal fermé' : phase === 'starting' ? 'Démarrage de la sandbox…' : 'Connexion…'}
    </span>
    {#if phase === 'closed'}<Button size="sm" icon={RotateCw} onclick={connect}>Rouvrir</Button>{/if}
  </div>
  <div class="xterm-host" bind:this={host}></div>
  <p class="note">Un seul terminal par session : en ouvrir un autre ferme celui-ci. Les secrets affichés sont masqués.</p>
</div>

<style>
  .terminal { display: flex; flex-direction: column; height: 100%; min-height: 0; background: var(--bg); }
  .tb { display: flex; align-items: center; gap: var(--s2); min-height: 36px; padding: 0 var(--s2) 0 var(--s3); border-bottom: 1px solid var(--line); background: var(--panel); font-size: var(--fs-sm); color: var(--muted); flex: none; }
  .xterm-host { flex: 1; min-height: 240px; background: var(--bg); padding: var(--s2) 0 0 var(--s3); }
  .note { margin: 0; padding: var(--s2) var(--s3); border-top: 1px solid var(--line); background: var(--panel); font-size: var(--fs-xs); color: var(--faint); flex: none; }
</style>
