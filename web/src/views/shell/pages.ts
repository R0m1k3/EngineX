// Pages de la coquille : routes `#<id>`, libellés, icônes, groupes de la barre d'activité.
import LayoutDashboard from '@lucide/svelte/icons/layout-dashboard';
import MessageSquare from '@lucide/svelte/icons/message-square';
import SquareCheck from '@lucide/svelte/icons/square-check';
import Bot from '@lucide/svelte/icons/bot';
import Clock from '@lucide/svelte/icons/clock';
import Folder from '@lucide/svelte/icons/folder';
import GitBranch from '@lucide/svelte/icons/git-branch';
import Box from '@lucide/svelte/icons/box';
import Cpu from '@lucide/svelte/icons/cpu';
import BookOpen from '@lucide/svelte/icons/book-open';
import Settings from '@lucide/svelte/icons/settings';
import Bell from '@lucide/svelte/icons/bell';
import type { IconType } from '../../lib/ui/types';

export interface PageDef {
  id: string;
  label: string;
  /** Libellé court (barre d'onglets du téléphone). */
  short?: string;
  icon: IconType;
  /** Groupe de la barre d'activité (séparés par un filet) ; 0 = hors barre. */
  group: number;
}

export const PAGES: PageDef[] = [
  { id: 'dashboard', label: 'Tableau de bord', short: 'Accueil', icon: LayoutDashboard, group: 1 },
  { id: 'sessions', label: 'Sessions', icon: MessageSquare, group: 1 },
  { id: 'tasks', label: 'Tâches', icon: SquareCheck, group: 1 },
  { id: 'agents', label: 'Agents', icon: Bot, group: 1 },
  { id: 'crons', label: 'Crons', icon: Clock, group: 1 },
  { id: 'files', label: 'Fichiers', icon: Folder, group: 2 },
  { id: 'git', label: 'Git', icon: GitBranch, group: 2 },
  { id: 'models', label: 'Modèles', icon: Box, group: 3 },
  { id: 'system', label: 'Système', icon: Cpu, group: 3 },
  { id: 'memory', label: 'Mémoire', icon: BookOpen, group: 4 },
  { id: 'feed', label: 'Notifications', icon: Bell, group: 0 },
  { id: 'settings', label: 'Paramètres', icon: Settings, group: 0 },
];

export const page = (id: string) => PAGES.find((p) => p.id === id);

/** Onglets du téléphone ; le reste passe dans « Plus ». */
export const PHONE_TABS = ['sessions', 'dashboard', 'tasks', 'feed'];
