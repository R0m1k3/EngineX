<script lang="ts">
  // Barre d'onglets du téléphone : Sessions, Accueil, Tâches, Notifications, Plus.
  import Ellipsis from '@lucide/svelte/icons/ellipsis';
  import Icon from '../../lib/ui/Icon.svelte';
  import { PHONE_TABS, page as pageDef } from './pages';

  let { page, unread, onMore }: { page: string; unread: number; onMore: () => void } = $props();
  const tabs = PHONE_TABS.map((id) => pageDef(id)!);
  const inMore = $derived(!PHONE_TABS.includes(page));
</script>

<nav class="tabbar" aria-label="Pages">
  {#each tabs as t (t.id)}
    <a href="#{t.id}" class:on={page === t.id} aria-current={page === t.id ? 'page' : undefined}>
      <Icon icon={t.icon} size={22} />
      {#if t.id === 'feed' && unread}<span class="count" aria-label="{unread} non lues">{unread > 99 ? '99+' : unread}</span>{/if}
      <span>{t.short ?? t.label}</span>
    </a>
  {/each}
  <button type="button" class:on={inMore} onclick={onMore} aria-haspopup="dialog"><Icon icon={Ellipsis} size={22} /><span>Plus</span></button>
</nav>

<style>
  .tabbar {
    display: flex;
    justify-content: space-around;
    padding: 0 var(--safe-right) var(--safe-bottom) var(--safe-left);
    background: var(--panel);
    border-top: 1px solid var(--line);
  }
  a, button {
    position: relative;
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 2px;
    min-height: var(--tabbar-h);
    min-width: 56px;
    padding: 0 2px;
    border: 0;
    border-radius: 0;
    background: none;
    color: var(--muted);
    font: inherit;
    font-size: var(--fs-xs);
    font-weight: 400;
    text-decoration: none;
    filter: none;
  }
  .on { color: var(--accent); font-weight: 600; }
  .count {
    position: absolute;
    top: 4px;
    left: calc(50% + 6px);
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    border-radius: 9px;
    background: var(--accent-fill);
    color: var(--accent-ink);
    font-size: var(--fs-count);
    font-weight: 600;
    line-height: 18px;
    text-align: center;
  }
</style>
