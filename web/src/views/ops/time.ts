// Dates relatives et absolues des vues du lot « opérations » (format français).
const DAY = 86400;

const startOfDay = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime() / 1000;
const hm = (d: Date) => d.toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit' });

/** Passé : « à l'instant », « 4 min », « 3 h », « hier », « lun. 17 nov. ». */
export function ago(t: number, now = Date.now() / 1000): string {
  const s = Math.max(0, now - t);
  if (s < 60) return 'à l’instant';
  if (s < 3600) return `${Math.round(s / 60)} min`;
  const today = startOfDay(new Date(now * 1000));
  if (t >= today) return `${Math.floor(s / 3600)} h`;
  if (t >= today - DAY) return 'hier';
  return new Date(t * 1000).toLocaleDateString('fr-FR', { weekday: 'short', day: 'numeric', month: 'short' });
}

/** Futur : « 02:00, dans 9 h », « 07:30, demain », « lun. 09:00 ». */
export function ahead(t: number | null, now = Date.now() / 1000): string {
  if (!t) return '';
  const d = new Date(t * 1000);
  const s = t - now;
  const today = startOfDay(new Date(now * 1000));
  if (s < 60) return `${hm(d)}, imminent`;
  if (s < 3600) return `${hm(d)}, dans ${Math.round(s / 60)} min`;
  if (t < today + DAY) return `${hm(d)}, dans ${Math.round(s / 3600)} h`;
  if (t < today + 2 * DAY) return `${hm(d)}, demain`;
  return `${d.toLocaleDateString('fr-FR', { weekday: 'short' })} ${hm(d)}`;
}

/** Date et heure courtes : « 19/11/2026 14:32 ». */
export const stamp = (t: number | null) =>
  t ? new Date(t * 1000).toLocaleString('fr-FR', { dateStyle: 'short', timeStyle: 'short' }) : '';

/** Durée : « 42 s », « 3 min 10 s », « 1 h 05 ». */
export function span(sec: number): string {
  if (sec < 60) return `${sec} s`;
  if (sec < 3600) return `${Math.floor(sec / 60)} min${sec % 60 ? ` ${sec % 60} s` : ''}`;
  return `${Math.floor(sec / 3600)} h ${String(Math.floor((sec % 3600) / 60)).padStart(2, '0')}`;
}

const DAYS = ['dimanche', 'lundi', 'mardi', 'mercredi', 'jeudi', 'vendredi', 'samedi'];

/** Horaire lisible pour les formes courantes (`every:N`, « M H * * J ») ; sinon l'expression telle quelle. */
export function scheduleText(s: string): string {
  const e = /^every:(\d+)$/.exec(s);
  if (e) {
    const n = Number(e[1]);
    if (n === 60) return 'toutes les heures';
    if (n % 60 === 0) return `toutes les ${n / 60} h`;
    return `toutes les ${n} min`;
  }
  const m = /^(\d{1,2}) (\d{1,2}) \* \* (\*|[0-7]|1-5)$/.exec(s.trim());
  if (!m) return s;
  const at = `${m[2].padStart(2, '0')}:${m[1].padStart(2, '0')}`;
  if (m[3] === '*') return `chaque jour à ${at}`;
  if (m[3] === '1-5') return `en semaine à ${at}`;
  return `le ${DAYS[Number(m[3]) % 7]} à ${at}`;
}

/** Passé avec « il y a » quand c'est une durée : « il y a 8 min », « hier ». */
export function since(t: number): string {
  const a = ago(t);
  return /^\d/.test(a) ? `il y a ${a}` : a;
}
