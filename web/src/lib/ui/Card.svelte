<script lang="ts">
  // Panneau à filets (pas de carte flottante) : titre avec icône, lien « plus » ou actions à droite.
  // Dans une grille `.ui-tiles` (filets = fond), `span` = nombre de colonnes.
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';
  import type { IconType } from './types';

  let {
    title,
    icon,
    more,
    moreHref,
    span = 1,
    flush = false,
    actions,
    children,
  }: {
    title?: string;
    icon?: IconType;
    more?: string;
    moreHref?: string;
    span?: 1 | 2 | 3;
    /** Sans marge intérieure (listes bord à bord). */
    flush?: boolean;
    actions?: Snippet;
    children: Snippet;
  } = $props();
</script>

<section class="card-ui" class:flush style:grid-column={span > 1 ? `span ${span}` : undefined}>
  {#if title}
    <header>
      <h2>{#if icon}<Icon {icon} class="ic" />{/if}<span>{title}</span></h2>
      {#if actions}<div class="acts">{@render actions()}</div>{/if}
      {#if more && moreHref}<a class="more" href={moreHref}>{more}</a>{/if}
    </header>
  {/if}
  {@render children()}
</section>

<style>
  .card-ui { background: var(--bg); padding: var(--s5); min-width: 0; display: flex; flex-direction: column; gap: var(--s3); }
  .flush { padding: 0; }
  .flush header { padding: var(--s4) var(--s5) 0; }
  header { display: flex; align-items: center; gap: var(--s2); min-height: 24px; }
  h2 { display: flex; align-items: center; gap: var(--s2); margin: 0; font-size: var(--fs-md); font-weight: 600; flex: 1; min-width: 0; }
  h2 :global(.ic) { color: var(--faint); }
  .acts { display: flex; gap: var(--s2); align-items: center; }
  .more { font-size: var(--fs-sm); color: var(--muted); text-decoration: none; }
  .more:hover { color: var(--accent); }
  @media (max-width: 767px) {
    .card-ui { grid-column: auto !important; padding: var(--s4); }
  }
</style>
