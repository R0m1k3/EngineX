<script lang="ts">
  // Menu flottant ancré à un élément (compte, ⋯). Fermé par Échap, clic dehors, choix ou redimensionnement.
  // `side` : right (à droite du bouton, barre d'activité), top (au-dessus, aligné à droite),
  // bottom-end (sous le bouton, aligné à droite ; au-dessus s'il n'y a pas la place).
  // La position est toujours gardée dans l'écran.
  import type { MenuItem } from './types';
  import Icon from './Icon.svelte';

  let {
    open = $bindable(false),
    anchor,
    items,
    side = 'right',
    label,
  }: { open?: boolean; anchor: HTMLElement | undefined; items: MenuItem[]; side?: 'right' | 'top' | 'bottom-end'; label: string } = $props();

  let el: HTMLDivElement | undefined = $state();
  let pos = $state({ x: 0, y: 0 });

  const M = 8;
  const clamp = (v: number, size: number, room: number) => Math.max(M, Math.min(v, room - size - M));

  $effect(() => {
    if (!open || !anchor || !el) return;
    const r = anchor.getBoundingClientRect();
    const w = el.offsetWidth;
    const h = el.offsetHeight;
    let x: number;
    let y: number;
    if (side === 'right') {
      x = r.right + M;
      y = r.bottom - h;
    } else if (side === 'top') {
      x = r.right - w;
      y = r.top - h - M;
    } else {
      x = r.right - w;
      y = r.bottom + 4 + h <= innerHeight - M ? r.bottom + 4 : r.top - h - 4;
    }
    pos = { x: clamp(x, w, innerWidth), y: clamp(y, h, innerHeight) };
    el.querySelector<HTMLElement>('[role=menuitem]:not(:disabled)')?.focus();
    const away = (e: PointerEvent) => {
      if (!el?.contains(e.target as Node) && !anchor?.contains(e.target as Node)) open = false;
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        open = false;
        anchor?.focus();
      }
    };
    const close = () => (open = false);
    document.addEventListener('pointerdown', away, true);
    document.addEventListener('keydown', key);
    window.addEventListener('resize', close);
    return () => {
      document.removeEventListener('pointerdown', away, true);
      document.removeEventListener('keydown', key);
      window.removeEventListener('resize', close);
    };
  });

  function move(e: KeyboardEvent) {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    e.preventDefault();
    const all = [...(el?.querySelectorAll<HTMLElement>('[role=menuitem]:not(:disabled)') ?? [])];
    const i = all.indexOf(document.activeElement as HTMLElement);
    all[(i + (e.key === 'ArrowDown' ? 1 : all.length - 1)) % all.length]?.focus();
  }
  function choose(it: MenuItem) {
    open = false;
    it.onclick?.();
  }
</script>

{#if open}
  <div bind:this={el} class="menu" role="menu" aria-label={label} tabindex="-1" style:transform="translate({pos.x}px, {pos.y}px)" onkeydown={move}>
    {#each items as it (it.label)}
      {#if it.href}
        <a role="menuitem" href={it.href} class:danger={it.danger} onclick={() => (open = false)}>{#if it.icon}<Icon icon={it.icon} size={16} />{/if}{it.label}</a>
      {:else}
        <button role="menuitem" type="button" class:danger={it.danger} disabled={it.disabled} onclick={() => choose(it)}>{#if it.icon}<Icon icon={it.icon} size={16} />{/if}{it.label}</button>
      {/if}
    {/each}
  </div>
{/if}

<style>
  .menu {
    position: fixed;
    left: 0;
    top: 0;
    z-index: 90;
    min-width: 220px;
    max-width: calc(100vw - 16px);
    padding: var(--s1);
    background: var(--panel);
    border: 1px solid var(--line-strong);
    border-radius: var(--r);
    box-shadow: var(--shadow);
    display: flex;
    flex-direction: column;
  }
  a, button {
    display: flex;
    align-items: center;
    justify-content: flex-start;
    gap: var(--s2);
    height: 36px;
    min-height: 0;
    padding: 0 var(--s3);
    border: 0;
    border-radius: var(--r);
    background: none;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-sm);
    font-weight: 400;
    text-align: left;
    text-decoration: none;
    white-space: nowrap;
    cursor: pointer;
    filter: none;
  }
  a:hover, button:hover:not(:disabled), a:focus-visible, button:focus-visible { background: var(--raised); outline: none; }
  button:disabled { color: var(--faint); opacity: 1; cursor: default; }
  .menu :global(.ui-icon) { color: var(--faint); flex: none; }
  .danger, .danger :global(.ui-icon) { color: var(--danger); }
  @media (max-width: 767px) {
    a, button { height: var(--touch); font-size: var(--fs-md); }
  }
</style>
