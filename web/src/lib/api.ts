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
    default:
      return `Erreur : ${e.code}`;
  }
}
