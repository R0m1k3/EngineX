// État du moteur et de la file pour la barre d'état (mêmes appels que le
// bandeau Moteur et le tableau de bord : GET /api/engine, /api/queue, /api/presets).
import { engine, presets, queueState, type EngineInfo, type Preset, type QueueState } from '../../lib/api';
import type { DotKind, GaugeRow } from '../../lib/ui/types';

export const status = $state<{ eng: EngineInfo | null; queue: QueueState | null; presets: Preset[]; error: boolean }>({
  eng: null,
  queue: null,
  presets: [],
  error: false,
});

let users = 0;
let timer: ReturnType<typeof setInterval> | undefined;

async function poll() {
  if (document.hidden) return;
  try {
    const [e, q] = await Promise.all([engine(), queueState()]);
    status.eng = e;
    status.queue = q;
    const id = e.preset_id;
    if (id !== null && !status.presets.some((p) => p.id === id)) status.presets = await presets();
    status.error = false;
  } catch {
    status.error = true;
  }
}

/** Démarre le relevé (toutes les 3 s, en pause quand l'onglet est caché) ; renvoie l'arrêt. */
export function watchStatus(): () => void {
  if (users++ === 0) {
    poll();
    timer = setInterval(poll, 3000);
    document.addEventListener('visibilitychange', poll);
  }
  return () => {
    if (--users === 0) {
      clearInterval(timer);
      document.removeEventListener('visibilitychange', poll);
    }
  };
}

const STATE: Record<string, string> = {
  stopped: 'arrêté',
  starting: 'chargement',
  ready: 'prêt',
  failed: 'en échec',
  stopping: 'arrêt',
};

export function engineSummary(e: EngineInfo | null, list: Preset[]) {
  if (!e) return { dot: 'off' as DotKind, model: 'Moteur', state: '…' };
  const st = e.state;
  const id = 'preset_id' in st ? st.preset_id : e.preset_id;
  const model = list.find((p) => p.id === id)?.name ?? (id != null ? `preset ${id}` : 'Aucun modèle');
  const dot: DotKind =
    st.state === 'ready' ? 'ok' : st.state === 'failed' ? 'err' : st.state === 'stopped' ? 'off' : 'warn';
  const state = st.state === 'starting' ? `${STATE.starting} ${st.progress} %` : STATE[st.state] ?? st.state;
  return { dot, model, state };
}

export function queueSummary(q: QueueState | null): string {
  if (!q) return 'File : …';
  if (q.swapping) return 'File : changement de modèle';
  const run = q.holders.length;
  const wait = q.waiting.length;
  if (!run && !wait) return 'File : libre';
  return `File : ${run} en cours${wait ? `, ${wait} en attente` : ''}`;
}

export function gpuRows(e: EngineInfo | null): GaugeRow[] {
  return (e?.gpus ?? []).map((g) => ({ label: `GPU${g.index}`, sub: g.name, total_mib: g.total_mib, used_mib: g.used_mib }));
}

export function rate(e: EngineInfo | null): string | null {
  const t = e?.state.state === 'ready' ? e.last_gen?.tokens_per_s : null;
  if (t == null) return null;
  return `${t >= 10 ? Math.round(t) : t.toFixed(1).replace('.', ',')} t/s`;
}
