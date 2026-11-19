// Client de l'API : JSON, cookie de session envoyé automatiquement.

export class ApiError extends Error {
  constructor(
    public status: number,
    public code: string,
    public retryAfter: number | null,
    public detail: string | null = null,
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
    const err = new ApiError(res.status, data.error ?? 'internal', retry ? Number(retry) : null, data.detail ?? null);
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
  expert_count?: number | null;
  expert_used_count?: number | null;
  nextn_predict_layers?: number | null;
  mtp_block?: number | null;
  [k: string]: unknown;
}

/** MTP utilisable : clé `nextn_predict_layers` et tenseur du dernier bloc. */
export const hasMtp = (m: GgufMeta | undefined) =>
  !!m?.nextn_predict_layers && m.block_count != null && m.block_count > 0 && m.mtp_block === m.block_count - 1;

export type Verdict = 'fits' | 'tight' | 'offload' | 'no_fit' | 'unknown';

export interface FitReport {
  cards: { need_mib: number; avail_mib: number }[];
  verdict: Verdict;
  kv_mib: number;
  /** Ce qui ne tient pas sur les cartes (calculé par le processeur). */
  cpu_mib: number;
  /** Matrice d'attention réservée en plus par carte sans flash attention. */
  no_fa_mib: number;
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
  spec: {
    draft: 'none' | 'mtp' | 'model';
    draft_model_id: number | null;
    draft_max: number;
    ngram: boolean;
  };
  moe: { mode: 'auto' | 'all_cpu' | 'layers'; cpu_layers: number; cache_mib: number };
  batch: number | null;
  ubatch: number | null;
  /** Jetons de réflexion au plus par réponse (null = illimité). */
  reasoning_budget: number | null;
  /** Jetons écrits au plus par réponse, réflexion comprise (null = illimité). */
  max_output: number | null;
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
  /** Chargé dans le moteur. */
  active: boolean;
  /** Preset par défaut des sessions (dernier activé par Michael). */
  default?: boolean;
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
  caps: { fit: boolean; spec: boolean; moe: boolean; budget?: boolean } | null;
  last_gen: {
    at: number;
    preset: string;
    tokens_per_s: number | null;
    predicted: number | null;
    draft_n: number | null;
    draft_accepted: number | null;
    /** Cœurs occupés par llama-server pendant la requête (≈ 1 si tout est sur GPU). */
    cpu_cores?: number | null;
  } | null;
  spec_fallback: { preset_id: number; why: string } | null;
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
export const presetFit = (id: number, settings: PresetSettings) =>
  request<FitReport>('POST', `/api/presets/${id}/fit`, { settings });
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
  offload: 'experts en partie en RAM',
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
    case 'folder_busy':
      return 'Un tour ou une vérification travaille sur ce dossier : réessayez quand il a fini.';
    case 'changed_since':
      return 'Ce fichier a changé depuis ce tour.';
    case 'no_snapshot':
      return 'Instantané absent : retour arrière indisponible pour ce message.';
    case 'recipe_exists':
      return 'Le fichier .enginex/verify.toml existe déjà.';
    case 'no_recipe':
      return 'Aucune recette de vérification (ni .enginex/verify.toml, ni projet reconnu).';
    case 'not_git':
      return 'Ce topic n’est pas un dépôt git.';
    case 'memory_changed':
      return 'Modifié entre-temps (par une session ?) : recharger.';
    case 'too_large':
      return 'Trop long : 16 Kio au plus pour memory.md, 4 Kio pour instructions.md, 64 Kio pour une skill.';
    case 'invalid_skill':
      return 'Skill refusée : le frontmatter doit avoir name (égal au nom de la skill) et description.';
    case 'copy_failed':
      return `La copie séparée a échoué.${e.detail ? `\n${e.detail}` : ''}`;
    case 'file_changed':
      return 'Fichier modifié ailleurs depuis son ouverture : rechargez-le (vos changements sont encore dans l’éditeur).';
    case 'protected':
      return 'Dossier géré par EngineX (projets, topics, mémoire, skills) : passez par sa page.';
    case 'dirty':
      return 'Modifications non validées dans le dossier : validez-les ou annulez-les d’abord.';
    case 'merge_conflict':
      return 'Conflit : fusion abandonnée, rien n’a changé.';
    case 'not_fast_forward':
      return 'La branche a divergé de son amont : pas d’avance rapide possible (fusionnez à la main).';
    case 'nothing_to_commit':
      return 'Rien à valider.';
    case 'push_rejected':
      return 'Push refusé par le dépôt distant (il a avancé : faites un pull d’abord). Jamais forcé.';
    case 'no_remote':
      return `Pas de remote origin utilisable.${e.detail ? ` ${e.detail}` : ''}`;
    case 'no_forge':
      return 'Remote ni GitHub ni Gitea : pour Gitea, ajoutez GITEA_URL dans Clés.';
    case 'no_token':
      return `Jeton absent : ${e.detail ?? 'GITHUB_TOKEN ou GITEA_TOKEN'} (page Clés).`;
    case 'no_upstream':
      return 'Cette branche n’a pas d’amont : poussez-la d’abord.';
    case 'no_branch':
      return 'HEAD détachée : choisissez une branche.';
    case 'same_branch':
      return 'La branche est déjà la branche de base.';
    case 'merge_refused':
      return `Fusion refusée par la forge.${e.detail ? ` ${e.detail}` : ''}`;
    case 'git_failed':
    case 'forge_error':
      return `Échec git.${e.detail ? `\n${e.detail}` : ''}`;
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
  /** Dépôt git : copie séparée possible. */
  git?: boolean;
}

export interface Project {
  id: number;
  slug: string;
  name: string;
  topics: Topic[];
}

export type Approval = 'auto' | 'prudent';

/** Réflexion du modèle : `medium` = budget du preset. */
export type Effort = 'none' | 'low' | 'medium' | 'high';

export interface Session {
  id: string;
  effort: Effort | null;
  topic_id: number;
  title: string;
  thread_id: string | null;
  approval: Approval;
  archived: boolean;
  created_at: number;
  updated_at: number;
  running: boolean;
  copy_branch: string | null;
  /** Note jointe au prochain message (après un retour arrière des fichiers). */
  pending_note: string | null;
  pending_revert: boolean;
  /** « Rétablir » possible (état d'avant le dernier retour arrière). */
  undo: boolean;
  /** Preset de la session ; `null` = preset par défaut. */
  preset_id: number | null;
}

/** Attente d'un tour dans la file commune (un seul modèle chargé). */
export interface QueueWait {
  position: number;
  running: { label: string; since: number; priority: number; session_id: string | null }[];
  swapping: number | null;
  needs_swap: boolean;
}

export interface QueueState {
  loaded: number | null;
  slots: number;
  swapping: number | null;
  holders: { session_id: string | null; label: string; priority: number; since: number }[];
  waiting: {
    session_id: string | null;
    label: string;
    priority: number;
    preset_id: number | null;
    since: number;
    needs_swap: boolean;
  }[];
}
export const queueState = () => request<QueueState>('GET', '/api/queue');

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
  origin?: 'user' | 'verify';
  has_snapshot?: boolean;
  has_post?: boolean;
  changes?: Changes | null;
  verifications?: Verification[];
}

export interface ChangedFile {
  path: string;
  status: 'A' | 'M' | 'D' | 'T';
  added: number;
  deleted: number;
  binary: boolean;
}

export interface Changes {
  files: ChangedFile[];
  truncated: boolean;
}

export interface StepResult {
  name?: string;
  run?: string;
  exit_code?: number | null;
  duration_ms?: number;
  timed_out?: boolean;
  output?: string;
  error?: string;
}

export interface Verification {
  id: number;
  turn_id: string;
  attempt: number;
  status: 'running' | 'passed' | 'failed' | 'error' | 'cancelled';
  steps: StepResult[] | null;
  fixes_done?: number;
  max_fixes?: number;
}

export interface Recipe {
  source: 'file' | 'detected' | 'none';
  steps: { name: string; run: string; timeout: number }[];
  error?: string;
}

export interface TreeEntry {
  name: string;
  kind: 'dir' | 'file' | 'link';
  size: number;
  mtime: number;
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
  green_turn: string | null;
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
  verify_enabled?: boolean;
  verify_max_fixes?: number;
  verify_app_check?: boolean;
  memory_auto?: boolean;
  git_name?: string;
  git_email?: string;
  disk_alert_pct?: number;
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
export const createSession = (topic_id: number, approval: Approval = 'auto', copy = false) =>
  request<{ id: string; copy_branch: string | null }>('POST', '/api/sessions', { topic_id, approval, copy });
export const sessionDetail = (id: string, before?: number) =>
  request<SessionDetail>('GET', `/api/sessions/${id}${before === undefined ? '' : `?before=${before}`}`);
export const patchSession = (
  id: string,
  patch: { title?: string; approval?: Approval; archived?: boolean; preset_id?: number | null; effort?: Effort | null },
) =>
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

// ---- Code dans une session ----

const S = (id: string) => `/api/sessions/${id}`;
export const turnDiff = (id: string, tid: string, path: string) =>
  request<{ diff: string; truncated: boolean; binary: boolean }>('GET', `${S(id)}/turns/${q(tid)}/diff?path=${q(path)}`);
export const undoFile = (id: string, tid: string, path: string, force = false) =>
  request('POST', `${S(id)}/turns/${q(tid)}/undo-file`, { path, force });
export const revertTurn = (id: string, tid: string, conversation: boolean) =>
  request<{ ok: boolean; user: ItemData | null }>('POST', `${S(id)}/turns/${q(tid)}/revert`, { conversation });
export const restoreUndo = (id: string) => request('POST', `${S(id)}/restore-undo`);
export const restoreGreen = (id: string) => request('POST', `${S(id)}/restore-green`);
export const verifyRecipe = (id: string) => request<Recipe>('GET', `${S(id)}/verify`);
export const saveRecipe = (id: string) => request('POST', `${S(id)}/verify/save`);
export const runVerify = (id: string) => request('POST', `${S(id)}/verify/run`);
export const tree = (id: string, path: string) =>
  request<{ path: string; entries: TreeEntry[]; truncated: boolean }>('GET', `${S(id)}/tree?path=${q(path)}`);
export const fileText = (id: string, path: string) =>
  request<{ text: string; truncated: boolean; binary: boolean }>('GET', `${S(id)}/file/text?path=${q(path)}`);
export const downloadUrl = (id: string, path: string) => `${fileUrl(id, path)}&download=1`;
export const bringBack = (id: string) =>
  request<{ result: 'merged' | 'branch_only'; reason?: 'topic_dirty' | 'conflict'; branch: string }>('POST', `${S(id)}/bring-back`);

// ---- Mémoire et skills ----

export interface MemFileInfo {
  size: number;
  limit: number;
  author: string | null;
  updated: number | null;
}

export interface MemScope {
  path: string;
  slug?: string;
  name?: string;
  instructions: MemFileInfo;
  memory: MemFileInfo;
}

export interface MemProject extends MemScope {
  topics: MemScope[];
}

export interface MemTree {
  global: MemScope;
  projects: MemProject[];
  warn: number;
}

export interface MemRevision {
  id: number;
  author: string;
  created_at: number;
  size: number;
}

export type MemFile = 'memory' | 'instructions';
const M = (path: string, file: MemFile) => `/api/memory/file?path=${q(path)}&file=${file}`;
export const memoryTree = () => request<MemTree>('GET', '/api/memory');
export const memoryFile = (path: string, file: MemFile) => request<{ content: string; sha: string }>('GET', M(path, file));
export const saveMemoryFile = (path: string, file: MemFile, content: string, base_sha: string) =>
  request<{ sha: string; size: number }>('PUT', M(path, file), { content, base_sha });
export const memoryRevisions = (path: string, file: MemFile) =>
  request<{ revisions: MemRevision[] }>('GET', `/api/memory/revisions?path=${q(path)}&file=${file}`).then((r) => r.revisions);
export const memoryRevision = (id: number) =>
  request<{ id: number; content: string; author: string; created_at: number }>('GET', `/api/memory/revisions/${id}`);

export interface Skill {
  name: string;
  description: string | null;
  valid: boolean;
  shipped: boolean;
  enabled: boolean;
}

export const skills = () => request<{ skills: Skill[] }>('GET', '/api/skills').then((r) => r.skills);
export const skill = (name: string) =>
  request<{ name: string; content: string; shipped: boolean; enabled: boolean }>('GET', `/api/skills/${q(name)}`);
export const saveSkill = (name: string, content: string) => request('PUT', `/api/skills/${q(name)}`, { content });
export const enableSkill = (name: string, enabled: boolean) =>
  request('PUT', `/api/skills/${q(name)}/enabled`, { enabled });

// ---- Tâches (workspace/tasks.json, format Harness) ----

export type TaskStatus = 'todo' | 'doing' | 'done';

export interface Task {
  id: string;
  title: string;
  notes: string;
  status: TaskStatus;
  project_id: string | null;
  due: string | null;
  order: number;
  created_at: number;
  updated_at: number;
  done_at?: number;
  created_by?: string;
}

export interface TaskProject {
  id: string;
  name: string;
}

export interface Board {
  version: number;
  projects: TaskProject[];
  tasks: Task[];
}

export type TaskPatch = Partial<Pick<Task, 'title' | 'notes' | 'status' | 'project_id' | 'due' | 'order'>>;

export const board = () => request<Board>('GET', '/api/tasks');
export const createTask = (t: { title: string; notes?: string; status?: TaskStatus; project_id?: string | null; due?: string | null }) =>
  request<Task>('POST', '/api/tasks', t);
export const patchTask = (id: string, p: TaskPatch) => request<Task>('PATCH', `/api/tasks/${encodeURIComponent(id)}`, p);
export const deleteTask = (id: string) => request('DELETE', `/api/tasks/${encodeURIComponent(id)}`);
export const createTaskProject = (name: string) => request<TaskProject>('POST', '/api/tasks/projects', { name });
export const deleteTaskProject = (id: string) => request('DELETE', `/api/tasks/projects/${encodeURIComponent(id)}`);

// ---- Crons ----

export type CronKind = 'prompt' | 'shell';
export type CronRunStatus = 'running' | 'ok' | 'failed' | 'skipped';

export interface CronBody {
  name: string;
  topic_id: number;
  kind: CronKind;
  body: string;
  schedule: string;
  tz?: string;
  timeout_s?: number;
  enabled?: boolean;
  /** `null` = preset par défaut. */
  preset_id?: number | null;
}

export interface Cron extends Required<CronBody> {
  id: number;
  topic: string | null;
  session_id: string | null;
  next_run: number | null;
  running: boolean;
  last: { status: CronRunStatus; started_at: number; ended_at: number | null } | null;
}

export interface CronRun {
  id: number;
  trigger: 'schedule' | 'manual';
  status: CronRunStatus;
  started_at: number;
  ended_at: number | null;
  output: string;
  session_id: string | null;
}

export const crons = () => request<Cron[]>('GET', '/api/crons');
export const createCron = (b: CronBody) => request<{ id: number }>('POST', '/api/crons', b);
export const updateCron = (id: number, b: CronBody) => request('PUT', `/api/crons/${id}`, b);
export const deleteCron = (id: number) => request('DELETE', `/api/crons/${id}`);
export const runCron = (id: number) => request('POST', `/api/crons/${id}/run`, {});
export const cronRuns = (id: number) => request<CronRun[]>('GET', `/api/crons/${id}/runs`);
export const cronPreview = (schedule: string, tz?: string) =>
  request<{ next: number[] }>('POST', '/api/crons/preview', { schedule, tz });

// ---- Agents ----

export type AgentTrigger = 'schedule' | 'manual' | 'webhook' | 'feed' | 'agent' | 'session';

export interface FeedFilter {
  /** Vide = toutes les sources. */
  sources: string[];
  min_level: FeedLevel;
  contains: string;
}

export interface AgentBody {
  name: string;
  topic_id: number;
  preset_id: number | null;
  role: string;
  /** `null` = pas d'horaire. */
  schedule: string | null;
  tz: string;
  /** `null` = pas de départ par le fil. */
  feed_filter: FeedFilter | null;
  approval: 'auto' | 'prudent';
  timeout_s: number;
  budget_tokens: number;
  budget_gpu_min: number;
  notify: boolean;
  enabled: boolean;
}

export interface Agent extends AgentBody {
  id: number;
  topic: string | null;
  next_run: number | null;
  running: boolean;
  today: { tokens: number; gpu_s: number };
  webhooks: { id: number; name: string }[];
  last: { status: CronRunStatus; trigger: AgentTrigger; started_at: number; ended_at: number | null; session_id: string | null } | null;
}

export interface AgentRun {
  id: number;
  trigger: AgentTrigger;
  context: string;
  status: CronRunStatus;
  started_at: number;
  ended_at: number | null;
  output: string;
  session_id: string | null;
  tokens: number;
  gpu_s: number;
  depth: number;
}

export const agents = () => request<Agent[]>('GET', '/api/agents');
export const createAgent = (b: AgentBody) => request<{ id: number }>('POST', '/api/agents', b);
export const updateAgent = (id: number, b: AgentBody) => request('PUT', `/api/agents/${id}`, b);
export const deleteAgent = (id: number) => request('DELETE', `/api/agents/${id}`);
export const runAgent = (id: number, context = '') =>
  request<{ run_id: number }>('POST', `/api/agents/${id}/run`, { context });
export const stopAgent = (id: number) => request('POST', `/api/agents/${id}/stop`, {});
export const agentRuns = (id: number) => request<AgentRun[]>('GET', `/api/agents/${id}/runs`);

// ---- Page Fichiers (workspace) ----

export interface WsEntry extends TreeEntry {
  managed: boolean;
  readonly: boolean;
}

export interface DiskUsage {
  total_bytes: number;
  free_bytes: number;
  used_pct: number;
  alert_pct: number;
  alert: boolean;
  workspace_bytes: number;
  sessions_bytes: number;
}

const F = '/api/files';
export const wsTree = (path: string) =>
  request<{ path: string; entries: WsEntry[]; truncated: boolean; readonly: boolean }>('GET', `${F}/tree?path=${q(path)}`);
export const wsText = (path: string) =>
  request<{ text: string; truncated: boolean; binary: boolean; version: number; size: number; readonly: boolean }>(
    'GET',
    `${F}/text?path=${q(path)}`,
  );
/** `version` absente : création (refusée si le fichier existe). */
export const wsSave = (path: string, text: string, version?: number) =>
  request<{ version: number }>('PUT', `${F}/text`, { path, text, version });
export const wsMkdir = (path: string) => request('POST', `${F}/mkdir`, { path });
export const wsMove = (from: string, to: string) => request('POST', `${F}/move`, { from, to });
export const wsDelete = (paths: string[]) => request<{ deleted: number }>('POST', `${F}/delete`, { paths });
export const wsRawUrl = (path: string) => `${F}/raw?path=${q(path)}`;
export const wsDownloadUrl = (path: string) => `${wsRawUrl(path)}&download=1`;
export const wsZipUrl = (paths: string[]) => `${F}/zip?paths=${q(paths.join('\n'))}`;
export const diskUsage = () => request<DiskUsage>('GET', `${F}/usage`);

export async function wsUpload(path: string, file: Blob, overwrite = false): Promise<void> {
  const res = await fetch(`${F}/upload?path=${q(path)}${overwrite ? '&overwrite=true' : ''}`, {
    method: 'POST',
    credentials: 'same-origin',
    headers: { 'Content-Type': 'application/octet-stream' },
    body: file,
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new ApiError(res.status, data.error ?? 'internal', null, data.detail ?? null);
}

// ---- Page Git ----

export interface GitRepo {
  topic_id: number;
  project: string;
  topic: string;
  path: string;
  head: string | null;
  origin: string | null;
}

export interface GitBranch {
  name: string;
  sha: string;
  current: boolean;
  upstream: string | null;
  upstream_gone: boolean;
  ahead: number;
  behind: number;
}

export interface GitRefName {
  name: string;
  kind: 'branch' | 'remote' | 'tag';
}

export interface GitCommit {
  sha: string;
  parents: string[];
  author: string;
  email: string;
  time: number;
  subject: string;
  refs: GitRefName[];
}

export interface GitRepoDetail {
  head: string | null;
  head_sha: string | null;
  detached: boolean;
  branches: GitBranch[];
  remotes: { name: string; sha: string }[];
  commits: GitCommit[];
  truncated: boolean;
  origin: string | null;
  forge: 'github' | 'gitea' | null;
}

export interface GitCommitDetail {
  sha: string;
  parents: string[];
  author: string;
  email: string;
  time: number;
  message: string;
  files: { path: string; status: string; additions: number; deletions: number; binary: boolean }[];
  diff: string;
  diff_truncated: boolean;
}

export interface PullRequest {
  number: number;
  title: string;
  state: 'open' | 'closed' | 'merged';
  draft: boolean;
  author: string;
  created_at: string;
  updated_at: string;
  head: string;
  base: string;
  body: string;
  url: string;
  mergeable: boolean | null;
}

export interface PullDetail extends PullRequest {
  diff: string;
  diff_truncated: boolean;
  comments: { author: string; body: string; created_at: string; url: string }[];
}

const G = (id: number) => `/api/git/repos/${id}`;
export const gitRepos = () => request<{ repos: GitRepo[] }>('GET', '/api/git/repos').then((r) => r.repos);
export const gitRepo = (id: number, limit = 300) => request<GitRepoDetail>('GET', `${G(id)}?limit=${limit}`);
export const gitCommit = (id: number, sha: string) => request<GitCommitDetail>('GET', `${G(id)}/commits/${sha}`);
export const gitFetch = (id: number) => request('POST', `${G(id)}/fetch`);
export const gitPull = (id: number) => request('POST', `${G(id)}/pull`);
export const gitPush = (id: number, branch?: string) => request('POST', `${G(id)}/push`, { branch });
export const gitCheckout = (id: number, branch: string) => request('POST', `${G(id)}/checkout`, { branch });
export const gitMerge = (id: number, branch: string) => request('POST', `${G(id)}/merge`, { branch });
export const gitBranch = (id: number, branch: string, sha?: string) => request('POST', `${G(id)}/branches`, { branch, sha });
export const gitPulls = (id: number, state: 'open' | 'closed' | 'all') =>
  request<{ pulls: PullRequest[] }>('GET', `${G(id)}/pulls?state=${state}`).then((r) => r.pulls);
export const gitPullDetail = (id: number, n: number) => request<PullDetail>('GET', `${G(id)}/pulls/${n}`);
export const gitComment = (id: number, n: number, body: string) => request('POST', `${G(id)}/pulls/${n}/comments`, { body });
export const gitMergePull = (id: number, n: number, method: 'merge' | 'squash' | 'rebase') =>
  request('POST', `${G(id)}/pulls/${n}/merge`, { method });
export const gitCreatePull = (id: number, title: string, body: string, head: string, base?: string) =>
  request<PullRequest>('POST', `${G(id)}/pulls`, { title, body, head, base });

export interface SessionGit {
  git: boolean;
  branch?: string | null;
  origin?: string | null;
  ahead?: number | null;
  forge?: 'github' | 'gitea' | null;
}
export type SideTab = 'files' | 'terminal' | 'git' | 'preview' | 'browser';

export type PreviewStatus = { running: boolean; ports: number[]; error?: string | null; enabled: boolean };
export const previewStatus = (id: string) => request<PreviewStatus>('GET', `${S(id)}/preview`);
export const previewOpen = (id: string, port: number) =>
  request<{ url: string; origin: string }>('POST', `${S(id)}/preview`, { port });

export type BrowserStatus = { enabled: boolean; ok?: boolean; version?: string; error?: string };
export const browserStatus = () => request<BrowserStatus>('GET', '/api/browser');

export const sessionGit = (id: string) => request<SessionGit>('GET', `${S(id)}/git`);
export const sessionCommit = (id: string, message: string) => request<{ sha: string }>('POST', `${S(id)}/git/commit`, { message });
export const sessionPush = (id: string) => request<{ branch: string }>('POST', `${S(id)}/git/push`);
export const sessionPullRequest = (id: string, title: string, body: string) =>
  request<PullRequest>('POST', `${S(id)}/git/pull-request`, { title, body });

// ---- Fil de notifications, push, webhooks ----

export type FeedLevel = 'info' | 'success' | 'warning' | 'error';

export interface FeedItem {
  id: number;
  at: number;
  source: string;
  level: FeedLevel;
  title: string;
  body: string;
  link: string | null;
  read: boolean;
}

export const feed = (o: { before?: number; limit?: number; unread?: boolean } = {}) => {
  const p = new URLSearchParams();
  if (o.before !== undefined) p.set('before', String(o.before));
  if (o.limit !== undefined) p.set('limit', String(o.limit));
  if (o.unread) p.set('unread', 'true');
  return request<{ items: FeedItem[]; unread: number }>('GET', `/api/feed?${p}`);
};
export const feedRead = (b: { ids?: number[]; all?: boolean }) => request<{ unread: number }>('POST', '/api/feed/read', b);
export const feedDelete = (id: number) => request<{ unread: number }>('DELETE', `/api/feed/${id}`);
export const feedClear = () => request<{ unread: number }>('DELETE', '/api/feed');

export type PushMode = 'important' | 'all' | 'off';
export interface PushStatus {
  public_key: string;
  mode: PushMode;
  subscriptions: { id: number; endpoint: string; host: string | null; agent: string; created_at: number; last_ok: number | null }[];
}
export const pushStatus = () => request<PushStatus>('GET', '/api/push');
export const pushSubscribe = (sub: PushSubscriptionJSON) => request('POST', '/api/push/subscribe', sub);
export const pushUnsubscribe = (b: { endpoint?: string; id?: number }) => request('POST', '/api/push/unsubscribe', b);
export const pushMode = (mode: PushMode) => request('PUT', '/api/push/mode', { mode });
export const pushTest = () => request<{ sent: number; failed: number; errors: string[] }>('POST', '/api/push/test');

export interface Webhook {
  id: number;
  name: string;
  level: FeedLevel;
  push: boolean;
  enabled: boolean;
  created_at: number;
  last_at: number | null;
  count: number;
  /** Agent lancé à chaque appel. */
  agent_id: number | null;
}
export const webhooks = () => request<Webhook[]>('GET', '/api/webhooks');
export const createWebhook = (b: { name: string; level: FeedLevel; push: boolean; agent_id?: number | null }) =>
  request<{ id: number; path: string }>('POST', '/api/webhooks', b);
export const patchWebhook = (id: number, b: Partial<Pick<Webhook, 'name' | 'level' | 'push' | 'enabled' | 'agent_id'>>) =>
  request('PATCH', `/api/webhooks/${id}`, b);
export const deleteWebhook = (id: number) => request('DELETE', `/api/webhooks/${id}`);
