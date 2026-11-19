<script lang="ts">
  // Menu flottant ancré à un élément (compte, ⋯). Fermé par Échap, clic dehors ou choix.
  import type { MenuItem } from './types';
  import Icon from './Icon.svelte';

  let {
    open = $bindable(false),
    anchor,
    items,
    side = 'right',
    label,
  }: { open?: boolean; anchor: HTMLElement | undefined; items: MenuItem[]; side?: 'right' | 'top'; label: string } = $props();

  let el: HTMLDivElement | undefined = $state();
  let pos = $state({ x: 0, y: 0 });

  $effect(() => {
    if (!open || !anchor || !el) return;
    const r = anchor.getBoundingClientRect();
    const w = el.offsetWidth;
    const h = el.offsetHeight;
    pos =
      side === 'right'
        ? { x: r.right + 8, y: Math.max(8, Math.min(r.bottom - h, innerHeight - h - 8)) }
        : { x: Math.max(8, Math.min(r.right - w, innerWidth - w - 8)), y: r.top - h - 8 };
    el.querySelector<HTMLElement>('[role=menuitem]')?.focus();
    const away = (e: PointerEvent) => {
      if (!el?.contains(e.target as Node) && !anchor?.contains(e.target as Node)) open = false;
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        open = false;
        anchor?.focus();
      }
    };
    document.addEventListener('pointerdown', away, true);
    document.addEventListener('keydown', key);
    return () => {
      document.removeEventListener('pointerdown', away, true);
      document.removeEventListener('keydown', key);
    };
  });

  function move(e: KeyboardEvent) {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    e.preventDefault();
    const all = [...(el?.querySelectorAll<HTMLElement>('[role=menuitem]') ?? [])];
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
        <button role="menuitem" type="button" class:danger={it.danger} onclick={() => choose(it)}>{#if it.icon}<Icon icon={it.icon} size={16} />{/if}{it.label}</button>
      {/if}
    {/each}
  </div>
{/if}

<style>
  .menu {
    position: fixed;
    left: 0;
    top: 0;
    z-index: 80;
    min-width: 220px;
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
    cursor: pointer;
    filter: none;
    justify-content: flex-start;
  }
  a:hover, button:hover, a:focus-visible, button:focus-visible { background: var(--raised); outline: none; }
  .danger { color: var(--danger); }
</style>
