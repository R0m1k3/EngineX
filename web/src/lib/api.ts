// Client de l'API : JSON, cookie de session envoyé automatiquement.

export class ApiError extends Error {
  constructor(
    public status: number,
    public code: string,
    public retryAfter: number | null,
  ) {
    super(code);
  }
}

type Listener = () => void;
let onUnauthorized: Listener = () => {};

/** Appelé quand une route sous session répond 401 (session perdue). */
export function setUnauthorizedHandler(fn: Listener) {
  onUnauthorized = fn;
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(path, {
    method,
    credentials: 'same-origin',
    headers: body === undefined ? {} : { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) {
    const retry = res.headers.get('Retry-After');
    const err = new ApiError(res.status, data.error ?? 'internal', retry ? Number(retry) : null);
    if (res.status === 401 && err.code === 'unauthorized') onUnauthorized();
    throw err;
  }
  return data as T;
}

export interface AuthState {
  setup_required: boolean;
  authenticated: boolean;
}

export interface Device {
  id: string;
  name: string;
  total_mib: number;
  free_mib: number;
}

export interface Gpu {
  index: number;
  name: string;
  total_mib: number;
  used_mib: number;
  temp_c: number;
  driver: string;
}

export interface SystemInfo {
  version: string;
  uptime_s: number;
  llama: { bin: string; version: string | null; devices: Device[]; error: string | null } | null;
  gpus: Gpu[];
  gpus_error: string | null;
}

export const authState = () => request<AuthState>('GET', '/api/auth/state');
export const setup = (code: string, password: string) => request('POST', '/api/auth/setup', { code, password });
export const login = (password: string) => request('POST', '/api/auth/login', { password });
export const logout = () => request('POST', '/api/auth/logout');
export const changePassword = (current: string, next: string) =>
  request('POST', '/api/auth/password', { current, new: next });
export const system = (refresh = false) => request<SystemInfo>('GET', refresh ? '/api/system?refresh=1' : '/api/system');

// ---- Modèles, presets, moteur ----

export interface RepoHit {
  repo: string;
  downloads: number;
  likes: number;
  updated: string | null;
  gated: boolean;
}

export interface FileSpec {
  name: string;
  size: number;
  sha256: string;
}

export interface QuantGroup {
  quant: string;
  files: FileSpec[];
  total_size: number;
}

export interface RepoInfo {
  repo: string;
  gated: boolean;
  base_model: string | null;
  groups: QuantGroup[];
  mmproj: FileSpec[];
}

export interface GgufMeta {
  arch: string;
  name: string | null;
  context_length: number | null;
  block_count: number | null;
  [k: string]: unknown;
}

export type Verdict = 'fits' | 'tight' | 'no_fit' | 'unknown';

export interface FitReport {
  cards: { need_mib: number; avail_mib: number }[];
  verdict: Verdict;
  kv_mib: number;
}

export interface FitResult {
  report: FitReport;
  meta: GgufMeta;
  measured_at: number | null;
}

export type DownloadState = 'queued' | 'running' | 'paused' | 'done' | 'failed' | 'cancelled';

export interface Download {
  id: number;
  repo: string;
  files: FileSpec[];
  mmproj: string | null;
  received_bytes: number;
  total_bytes: number;
  state: DownloadState;
  error: string | null;
  speed_bps: number;
}

export interface LocalModel {
  id: number;
  repo: string;
  file: string;
  shards: string[];
  mmproj: string | null;
  size_bytes: number;
  meta: GgufMeta;
  presets: number[];
}

export const SAMPLING = ['temperature', 'top_p', 'top_k', 'min_p', 'presence_penalty', 'repeat_penalty'] as const;
export type SamplingKey = (typeof SAMPLING)[number];
export type Sampling = Record<SamplingKey, number | null>;

export interface PresetSettings {
  ctx: number;
  kv_type: 'f16' | 'q8_0' | 'q4_0';
  flash_attn: boolean;
  parallel: number;
  fit_target_mib: number;
  tensor_split: number[] | null;
  reasoning: 'on' | 'off';
  sampling: Sampling;
  mmproj: boolean;
  extra_args: string[];
}

export interface Origin {
  source: 'card_repo' | 'card_base' | 'generation_config' | 'gguf' | 'defaut';
  excerpt: string | null;
  url: string | null;
}

export interface CandidateSet {
  label: 'thinking' | 'non_thinking' | 'general';
  values: Sampling;
  excerpt: string;
}

export interface Sources {
  values: Sampling;
  origins: Partial<Record<SamplingKey, Origin>>;
  to_choose: Partial<Record<SamplingKey, CandidateSet[]>>;
  error: string | null;
}

export interface Preset {
  id: number;
  name: string;
  model_id: number;
  settings: PresetSettings;
  sources: Sources;
  active: boolean;
}

export type EngineState =
  | { state: 'stopped' }
  | { state: 'starting'; preset_id: number; progress: number }
  | { state: 'ready'; preset_id: number; pid: number }
  | { state: 'failed'; preset_id: number; cause: string; last_lines: string[] }
  | { state: 'stopping' };

export interface EngineInfo {
  state: EngineState;
  preset_id: number | null;
  gpus: Gpu[];
  tools_ok: boolean | null;
  n_ctx: number | null;
  build: string | null;
  origin: 'env' | 'active' | 'image' | 'secours';
  has_fit: boolean;
  vram_others: number[] | null;
  measured_at: number | null;
  auto_update: boolean;
  pending: { tag: string; build: string; since: number } | null;
  auto_last: { at: number; tag: string; ok: boolean; message: string } | null;
  skipped: string[];
}

export interface TestResult {
  ok: boolean;
  code: string | null;
  message: string | null;
  gpus?: number;
  has_fit?: boolean;
  at: number;
}

export interface Job {
  kind: 'install' | 'use' | 'rollback' | 'auto_update' | 'activate' | 'stop' | 'delete';
  tag: string;
  step: 'test' | 'download' | 'wait_requests' | 'loading' | 'done' | 'failed';
  done_bytes: number;
  total_bytes: number;
  finished: boolean;
  error: string | null;
  message: string | null;
}

export interface Versions {
  active: string;
  previous: string | null;
  installed: { tag: string; build: string; installed_at: number; last_test: TestResult | null }[];
  latest: { tag: string; build: string } | null;
  job: Job | null;
  env_override: boolean;
}

const q = encodeURIComponent;
export const hfSearch = (text: string) => request<RepoHit[]>('GET', `/api/hf/search?q=${q(text)}`);
export const hfRepo = (repo: string) => request<RepoInfo>('GET', `/api/hf/repo?repo=${q(repo)}`);
export const hfFit = (repo: string, file: string, ctx: number, kv: string, mmproj?: string) =>
  request<FitResult>(
    'GET',
    `/api/hf/fit?repo=${q(repo)}&file=${q(file)}&ctx=${ctx}&kv=${kv}${mmproj ? `&mmproj=${q(mmproj)}` : ''}`,
  );
export const downloads = () => request<Download[]>('GET', '/api/downloads');
export const startDownload = (repo: string, quant: string, mmproj: string | null) =>
  request<{ id: number }>('POST', '/api/downloads', { repo, quant, mmproj });
export const pauseDownload = (id: number) => request('POST', `/api/downloads/${id}/pause`);
export const resumeDownload = (id: number) => request('POST', `/api/downloads/${id}/resume`);
export const deleteDownload = (id: number) => request('DELETE', `/api/downloads/${id}`);
export const models = () => request<LocalModel[]>('GET', '/api/models');
export const deleteModel = (id: number) => request('DELETE', `/api/models/${id}`);
export const presets = () => request<Preset[]>('GET', '/api/presets');
export const copyPreset = (from: number, name: string) => request<{ id: number }>('POST', '/api/presets', { from, name });
export const savePreset = (id: number, name: string, settings: PresetSettings) =>
  request<{ reload_needed: boolean }>('PUT', `/api/presets/${id}`, { name, settings });
export const deletePreset = (id: number) => request('DELETE', `/api/presets/${id}`);
export const activatePreset = (id: number, confirm_nofit = false) =>
  request<{ started: boolean; fit: FitReport }>('POST', `/api/presets/${id}/activate`, { confirm_nofit });
export const refreshSources = (id: number) => request<Preset>('POST', `/api/presets/${id}/refresh-sources`);
export const engine = () => request<EngineInfo>('GET', '/api/engine');
export const stopEngine = () => request('POST', '/api/engine/stop');
export const engineLogs = (tail = 200) => request<{ lines: string[] }>('GET', `/api/engine/logs?tail=${tail}`);
export const versions = () => request<Versions>('GET', '/api/engine/versions');
export const checkVersion = () => request<{ tag: string; build: string }>('POST', '/api/engine/versions/check');
export const installVersion = (tag?: string) => request('POST', '/api/engine/versions/install', { tag: tag ?? null });
export const useVersion = (tag: string) => request('POST', `/api/engine/versions/${q(tag)}/use`);
export const rollbackVersion = () => request('POST', '/api/engine/versions/rollback');
export const deleteVersion = (tag: string) => request('DELETE', `/api/engine/versions/${q(tag)}`);
export const setAutoUpdate = (on: boolean) => request<{ on: boolean }>('PUT', '/api/engine/auto-update', { on });

// ---- API pour Codex ----

export interface ApiKey {
  id: number;
  name: string;
  created_at: number;
  last_used_at: number | null;
}

export interface CodexConfig {
  base_url: string;
  toml: string;
  env_key: string;
}

export const apiKeys = () => request<ApiKey[]>('GET', '/api/api-keys');
export const createApiKey = (name: string) => request<ApiKey & { key: string }>('POST', '/api/api-keys', { name });
export const deleteApiKey = (id: number) => request('DELETE', `/api/api-keys/${id}`);
export const codexConfig = () => request<CodexConfig>('GET', '/api/codex-config');

/** Libellés des tâches moteur. */
export const JOB_KIND: Record<Job['kind'], string> = {
  install: 'Installation',
  use: 'Bascule',
  rollback: 'Retour',
  auto_update: 'Mise à jour automatique',
  activate: 'Activation du preset',
  stop: 'Arrêt du moteur',
  delete: 'Suppression du preset actif',
};

export const JOB_STEP: Record<Job['step'], string> = {
  test: 'test du moteur',
  download: 'téléchargement',
  wait_requests: 'attente de la fin des requêtes',
  loading: 'chargement du modèle',
  done: 'terminé',
  failed: 'échec',
};

/** Taille lisible (octets). */
export function size(bytes: number): string {
  if (bytes >= 1 << 30) return `${(bytes / (1 << 30)).toFixed(1)} Gio`;
  if (bytes >= 1 << 20) return `${(bytes / (1 << 20)).toFixed(0)} Mio`;
  return `${Math.max(0, Math.round(bytes / 1024))} Kio`;
}

export const VERDICT: Record<Verdict, string> = {
  fits: 'tient',
  tight: 'juste',
  no_fit: 'ne tient pas',
  unknown: 'inconnu',
};

/** Erreurs des tâches de fond (téléchargements, moteurs). */
export function jobError(code: string | null): string {
  if (!code) return '';
  const c = code.split(' ')[0];
  const known: Record<string, string> = {
    sha256_mismatch: 'Fichier corrompu (sha256 différent) : supprimé, relancez.',
    disk_full: 'Disque plein.',
    network: 'Réseau coupé après plusieurs essais.',
    io: 'Erreur disque.',
    register_failed: 'Fichier reçu mais illisible.',
    version_not_found: 'Version introuvable sur ghcr.',
    digest_mismatch: 'Couche corrompue (digest différent).',
    engine_incomplete: 'Moteur incomplet (fichiers manquants).',
    bad_link: 'Lien refusé dans l’image.',
    test_failed: 'Test du moteur échoué.',
    load_failed: 'Le modèle ne charge pas avec ce moteur : retour au précédent.',
    tools_broken: 'Cette version ne rend plus les appels d’outils : retour au précédent, version écartée.',
    tools_unknown: 'La sonde « outils » n’a pas répondu : retour au précédent, nouvel essai plus tard.',
  };
  if (c.startsWith('http_')) return `Hugging Face répond ${c.slice(5)}.`;
  const key = c.replace(/:$/, '');
  return known[key] ? `${known[key]} ${code.includes(':') ? code.slice(code.indexOf(':') + 1).trim() : ''}`.trim() : code;
}

/** Message affichable pour un code d'erreur de l'API. */
export function message(e: unknown): string {
  if (!(e instanceof ApiError)) return 'Serveur injoignable.';
  switch (e.code) {
    case 'bad_setup_code':
      return 'Code incorrect (voir docker logs enginex).';
    case 'password_too_short':
      return '12 caractères minimum.';
    case 'bad_credentials':
      return 'Mot de passe incorrect.';
    case 'rate_limited':
      return `Trop d'essais, réessayez dans ${Math.ceil((e.retryAfter ?? 60) / 60)} min.`;
    case 'busy':
      return 'Serveur occupé, réessayez.';
    case 'already_setup':
      return 'Déjà configuré : connectez-vous.';
    case 'model_in_use':
      return 'Modèle utilisé par le preset actif : arrêtez-le ou activez-en un autre.';
    case 'engine_in_use':
      return 'Ce moteur est utilisé (actif ou moteur de l’image).';
    case 'job_running':
      return 'Une opération sur le moteur est déjà en cours.';
    case 'already_downloaded':
      return 'Déjà téléchargé (voir Mes modèles).';
    case 'already_queued':
      return 'Déjà dans les téléchargements.';
    case 'nofit_confirm_required':
      return 'Ne tient pas en VRAM : confirmation demandée.';
    case 'env_override':
      return 'Binaire imposé par ENGINEX_LLAMA_BIN : changement de moteur désactivé.';
    case 'gated_repo':
      return 'Dépôt à accès restreint : non pris en charge.';
    case 'disk_full':
      return 'Pas assez de place sur le disque des modèles.';
    case 'upstream_error':
      return 'Hugging Face ou ghcr injoignable, réessayez.';
    case 'not_found':
      return 'Introuvable.';
    case 'bad_request':
      return 'Demande refusée (valeur invalide).';
    case 'name_taken':
      return 'Ce nom est déjà pris.';
    case 'no_active_preset':
      return 'Aucun preset actif : activez-en un dans Modèles.';
    case 'engine_not_ready':
      return 'Le modèle n’est pas encore chargé : attendez la fin du chargement.';
    case 'no_vision':
      return 'Le preset actif ne lit pas les images (pas de fichier mmproj).';
    case 'turn_in_progress':
      return 'Un tour est déjà en cours dans cette session.';
    case 'too_many_sandboxes':
      return 'Trop de sandboxes actives : attendez qu’une session se termine ou montez la limite dans Paramètres → Sessions.';
    case 'sandbox_unavailable':
      return 'Sandboxes indisponibles : Docker injoignable ou dossier /data introuvable (voir Paramètres → Sessions).';
    case 'sandbox_timeout':
      return 'La sandbox n’a pas démarré à temps.';
    case 'vault_locked':
      return 'Coffre verrouillé : restaurez /data/master.key ou supprimez les secrets.';
    case 'invalid_name':
      return 'Nom refusé (majuscules, chiffres et _, sans ENGINEX_ ni CODEX_ au début).';
    case 'not_empty':
      return 'Pas vide : supprimez d’abord ce qu’il contient.';
    case 'attachment_invalid':
      return e.status === 413 ? 'Image trop lourde (20 Mo au plus).' : 'Image refusée (PNG, JPEG, WebP ou GIF).';
    default:
      return `Erreur : ${e.code}`;
  }
}

// ---- Projets, sessions, coffre ----

export interface Topic {
  id: number;
  slug: string;
  name: string;
  sessions: number;
}

export interface Project {
  id: number;
  slug: string;
  name: string;
  topics: Topic[];
}

export type Approval = 'auto' | 'prudent';

export interface Session {
  id: string;
  topic_id: number;
  title: string;
  thread_id: string | null;
  approval: Approval;
  archived: boolean;
  created_at: number;
  updated_at: number;
  running: boolean;
}

/** Élément d'un fil (forme Codex : `type`, `id` et les champs du type). */
export type ItemData = Record<string, any>;

export interface StoredItem {
  turn_id: string | null;
  item_id: string;
  seq: number;
  kind: string;
  data: ItemData;
}

export interface TurnInfo {
  id: string;
  status: 'completed' | 'interrupted' | 'failed' | 'inProgress';
  error: string | null;
  usage: Usage | null;
  started_at?: number;
  completed_at?: number | null;
}

export interface Usage {
  last?: { inputTokens: number; outputTokens: number; cachedInputTokens: number; totalTokens: number };
  total?: { totalTokens: number };
  modelContextWindow?: number | null;
}

export interface SessionDetail {
  session: Session;
  turns: TurnInfo[];
  items: StoredItem[];
  more: boolean;
}

export interface PendingRequest {
  rid: string;
  method: string;
  params: Record<string, any>;
}

export interface Attachment {
  id: string;
  name: string;
  url: string;
  width: number;
  height: number;
}

export interface Secret {
  id: number;
  scope: string;
  name: string;
  created_at: number;
  updated_at: number;
}

export interface SandboxStatus {
  backend: 'docker' | 'local';
  available: boolean;
  docker: { reachable: boolean; error: string | null };
  host_data: { path: string | null; error: string | null };
  image: { name: string; present: boolean; last_pull_at: number | null; last_pull_error: string | null };
  live: number;
}

export interface SessionSettings {
  idle_minutes: number;
  max: number;
}

export const projects = () => request<{ projects: Project[] }>('GET', '/api/projects').then((r) => r.projects);
export const createProject = (name: string) => request<{ id: number }>('POST', '/api/projects', { name });
export const deleteProject = (id: number) => request('DELETE', `/api/projects/${id}`);
export const createTopic = (project: number, name: string) =>
  request<{ id: number }>('POST', `/api/projects/${project}/topics`, { name });
export const deleteTopic = (id: number) => request('DELETE', `/api/topics/${id}`);

export function sessions(f: { topic?: number; q?: string; archived?: boolean }) {
  const p = new URLSearchParams();
  if (f.topic !== undefined) p.set('topic', String(f.topic));
  if (f.q) p.set('q', f.q);
  if (f.archived) p.set('archived', '1');
  return request<{ sessions: Session[] }>('GET', `/api/sessions?${p}`).then((r) => r.sessions);
}
export const createSession = (topic_id: number, approval: Approval = 'auto') =>
  request<{ id: string }>('POST', '/api/sessions', { topic_id, approval });
export const sessionDetail = (id: string, before?: number) =>
  request<SessionDetail>('GET', `/api/sessions/${id}${before === undefined ? '' : `?before=${before}`}`);
export const patchSession = (id: string, patch: { title?: string; approval?: Approval; archived?: boolean }) =>
  request('PATCH', `/api/sessions/${id}`, patch);
export const deleteSession = (id: string) => request('DELETE', `/api/sessions/${id}`);
export const postTurn = (id: string, text: string, attachments: string[]) =>
  request('POST', `/api/sessions/${id}/turns`, { text, attachments });
export const interruptTurn = (id: string) => request('POST', `/api/sessions/${id}/interrupt`);
export const answerRequest = (id: string, rid: string, body: { decision?: string; answers?: unknown }) =>
  request('POST', `/api/sessions/${id}/requests/${q(rid)}`, body);

/** Envoie une image (corps brut, type MIME du fichier). */
export async function uploadAttachment(id: string, file: Blob): Promise<Attachment> {
  const res = await fetch(`/api/sessions/${id}/attachments`, {
    method: 'POST',
    credentials: 'same-origin',
    headers: { 'Content-Type': file.type || 'application/octet-stream' },
    body: file,
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new ApiError(res.status, data.error ?? 'internal', null);
  return data as Attachment;
}

/** Image produite dans le topic (élément imageView). */
export const fileUrl = (id: string, path: string) => `/api/sessions/${id}/file?path=${q(path)}`;

export const vault = () => request<{ secrets: Secret[]; locked: boolean }>('GET', '/api/vault');
export const putSecret = (scope: string, name: string, value: string) =>
  request<{ id: number }>('PUT', '/api/vault', { scope, name, value });
export const deleteSecret = (id: number) => request('DELETE', `/api/vault/${id}`);
export const sandboxStatus = () => request<SandboxStatus>('GET', '/api/sandbox/status');
export const sessionSettings = () => request<SessionSettings>('GET', '/api/settings/sessions');
export const saveSessionSettings = (s: SessionSettings) => request<SessionSettings>('PUT', '/api/settings/sessions', s);
