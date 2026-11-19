// Relais d'aperçu d'EngineX, lancé avec la sandbox (port 4501).
//
// Une app écoute souvent sur 127.0.0.1 dans son conteneur : EngineX ne peut
// pas la joindre directement. Il ouvre une connexion ici et envoie une ligne :
//   ENGINEX-FWD <jeton> <port>   → « OK », puis relais vers 127.0.0.1:<port>
//   ENGINEX-FWD <jeton> LIST     → « OK [ports en écoute] » (JSON), fin
// Le jeton est celui de l'app-server (/run/enginex/token) : une autre sandbox
// du réseau ne peut pas s'en servir.
import { createHash, timingSafeEqual } from 'node:crypto';
import { readFileSync } from 'node:fs';
import net from 'node:net';

const PORT = Number(process.env.ENGINEX_FWD_PORT || 4501);
const APP_SERVER = 4500;
const TOKEN_FILE = process.env.ENGINEX_FWD_TOKEN_FILE || '/run/enginex/token';
const MAX_LINE = 512;
const MAX_CONN = 512;

const digest = (s) => createHash('sha256').update(s).digest();

function tokenOk(given) {
  let want;
  try {
    want = readFileSync(TOKEN_FILE, 'utf8').trim();
  } catch {
    return false;
  }
  return want.length > 0 && timingSafeEqual(digest(want), digest(given));
}

// Ports TCP en écoute (état 0A) de /proc/net/tcp et tcp6, sans les nôtres.
export function listening(tables) {
  const ports = new Set();
  for (const t of tables) {
    for (const line of t.split('\n').slice(1)) {
      const f = line.trim().split(/\s+/);
      if (f.length < 4 || f[3] !== '0A') continue;
      const port = parseInt(f[1].split(':').pop(), 16);
      if (port && port !== APP_SERVER && port !== PORT) ports.add(port);
    }
  }
  return [...ports].sort((a, b) => a - b);
}

function readTables() {
  return ['/proc/net/tcp', '/proc/net/tcp6'].map((p) => {
    try {
      return readFileSync(p, 'utf8');
    } catch {
      return '';
    }
  });
}

function connectLocal(port, done) {
  const tryHost = (hosts) => {
    const up = net.connect({ host: hosts[0], port });
    up.once('connect', () => done(null, up));
    up.once('error', (e) => (hosts.length > 1 ? tryHost(hosts.slice(1)) : done(e)));
  };
  tryHost(['127.0.0.1', '::1']);
}

let open = 0;
const server = net.createServer((sock) => {
  if (open >= MAX_CONN) return sock.destroy();
  open += 1;
  sock.once('close', () => (open -= 1));
  sock.setTimeout(10000, () => sock.destroy());
  let buf = Buffer.alloc(0);
  const onData = (chunk) => {
    buf = Buffer.concat([buf, chunk]);
    const nl = buf.indexOf(10);
    if (nl < 0) {
      if (buf.length > MAX_LINE) sock.destroy();
      return;
    }
    sock.off('data', onData);
    sock.pause();
    const [magic, token, cmd] = buf.subarray(0, nl).toString('utf8').trim().split(' ');
    const rest = buf.subarray(nl + 1);
    if (magic !== 'ENGINEX-FWD' || !token || !tokenOk(token)) {
      return sock.end('ERR jeton refusé\n');
    }
    if (cmd === 'LIST') {
      return sock.end(`OK ${JSON.stringify(listening(readTables()))}\n`);
    }
    const port = Number(cmd);
    if (!Number.isInteger(port) || port < 1 || port > 65535 || port === PORT || port === APP_SERVER) {
      return sock.end('ERR port refusé\n');
    }
    connectLocal(port, (err, up) => {
      if (err) return sock.end(`ERR rien n'écoute sur le port ${port}\n`);
      sock.setTimeout(0);
      sock.write('OK\n');
      if (rest.length) up.write(rest);
      sock.pipe(up);
      up.pipe(sock);
      sock.resume();
      const kill = () => {
        sock.destroy();
        up.destroy();
      };
      sock.once('error', kill);
      up.once('error', kill);
      sock.once('close', kill);
      up.once('close', kill);
    });
  };
  sock.on('data', onData);
  sock.on('error', () => sock.destroy());
});

if (!process.env.ENGINEX_FWD_NO_LISTEN) server.listen(PORT, '0.0.0.0');
