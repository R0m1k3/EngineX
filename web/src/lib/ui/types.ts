// Types partagés des composants de lib/ui.
import type { Component } from 'svelte';

export type IconType = Component<any>;
export type DotKind = 'ok' | 'warn' | 'off' | 'err';
export type Tone = 'neutral' | 'ok' | 'warn' | 'danger';
export type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger';
export type ButtonSize = 'sm' | 'md' | 'lg';

export interface TabItem {
  id: string;
  label: string;
  icon?: IconType;
  /** Compteur affiché à côté du nom. */
  count?: number;
  /** Lien `#…` : l'onglet est un <a> (routage par l'ancre). */
  href?: string;
}

/** Une ligne de jauge VRAM (une carte ou le CPU), en Mio. */
export interface GaugeRow {
  label: string;
  sub?: string;
  total_mib: number;
  weights_mib?: number;
  kv_mib?: number;
  compute_mib?: number;
  /** Occupation sans détail (nvidia-smi) : dessinée comme un seul segment. */
  used_mib?: number;
  /** Texte de droite à la place de « x / y Go » (« rien », « 0 couche »…). */
  note?: string;
}

export interface MenuItem {
  label: string;
  icon?: IconType;
  href?: string;
  onclick?: () => void;
  danger?: boolean;
  disabled?: boolean;
}
