// Libellés et formats communs de la page Tester.
import type { BenchFamily, BenchVerdict } from '../../lib/api';
import type { Tone } from '../../lib/ui';
import { dec1 } from '../../lib/ui/format';

export const FAMILY: Record<BenchFamily, { label: string; hint: string }> = {
  visual: {
    label: 'Visuels',
    hint: 'La page est ouverte dans le navigateur d’EngineX (ordinateur et téléphone) et utilisée pour de vrai. La machine contrôle le fonctionnement ; vous notez le visuel.',
  },
  code: {
    label: 'Code',
    hint: 'Contrôlés à 100 % par la machine, avec des tests cachés que le modèle ne voit jamais.',
  },
  tools: {
    label: 'Outils',
    hint: 'La machine contrôle le résultat ET la façon de travailler (outils utilisés, vérifications faites), lue dans le journal de la session.',
  },
};

export const VERDICT: Record<BenchVerdict, { label: string; tone: Tone }> = {
  passed: { label: 'Réussi', tone: 'ok' },
  partial: { label: 'Partiel', tone: 'warn' },
  failed: { label: 'Échoué', tone: 'danger' },
};

export const STATUS: Record<string, string> = {
  pending: 'en attente', running: 'en cours', done: 'terminé', failed: 'échec', stopped: 'arrêté',
};

/** Note visuelle : le sens de chaque niveau, écrit sur le bouton. */
export const VISUAL_SCALE: { value: number; label: string; hint: string }[] = [
  { value: 1, label: 'Cassé', hint: 'illisible, éléments cassés ou qui se chevauchent' },
  { value: 2, label: 'Brut', hint: 'fonctionne mais sans mise en forme réelle' },
  { value: 3, label: 'Correct', hint: 'propre et lisible, sans plus' },
  { value: 4, label: 'Soigné', hint: 'agréable, cohérent, bien espacé' },
  { value: 5, label: 'Pro', hint: 'on pourrait le mettre en ligne tel quel' },
];

export const dur = (s: number) => (s >= 60 ? `${Math.floor(s / 60)} min ${String(s % 60).padStart(2, '0')} s` : `${s} s`);
export const pct = (v: number | null | undefined) => (v == null ? '—' : `${Math.round(v)} %`);
export const pts = (v: number | null | undefined) => (v == null ? '—' : dec1(Math.round(v * 10) / 10).replace(/,0$/, ''));
export const visual = (v: number | null | undefined) => (v == null ? '—' : `${dec1(v).replace(/,0$/, '')} / 5`);
export const visualLabel = (v: number | null | undefined) => VISUAL_SCALE.find((x) => x.value === Math.round(v ?? 0))?.label ?? '';
