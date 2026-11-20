<script lang="ts">
  // Barre d'activité du bureau (52 px) : pages groupées, palette, compte, paramètres.
  import Search from '@lucide/svelte/icons/search';
  import CircleUser from '@lucide/svelte/icons/circle-user';
  import KeyRound from '@lucide/svelte/icons/key-round';
  import LogOut from '@lucide/svelte/icons/log-out';
  import PanelLeftOpen from '@lucide/svelte/icons/panel-left-open';
  import PanelLeftClose from '@lucide/svelte/icons/panel-left-close';
  import Icon from '../../lib/ui/Icon.svelte';
  import Menu from '../../lib/ui/Menu.svelte';
  import { tooltip } from '../../lib/ui/tooltip';
  import Logo from './Logo.svelte';
  import { PAGES, page as pageDef } from './pages';

  let {
    page,
    onPalette,
    onPassword,
    onLogout,
  }: { page: string; onPalette: () => void; onPassword: () => void; onLogout: () => void } = $props();

  const groups = [1, 2, 3, 4].map((g) => PAGES.filter((p) => p.group === g));
  const settings = pageDef('settings')!;
  let wide = $state(typeof localStorage !== 'undefined' && localStorage.getItem('enginex.sidebar') === 'wide');
  $effect(() => {
    document.documentElement.style.setProperty('--act-w', wide ? '208px' : '52px');
    localStorage.setItem('enginex.sidebar', wide ? 'wide' : 'narrow');
  });
  const tip = (text: string) => (wide ? null : { text, side: 'right' as const });
  let account = $state(false);
  let accountBtn: HTMLButtonElement | undefined = $state();
</script>

<nav class="activity" class:wide aria-label="Pages">
  <a class="logo" href="#dashboard" aria-label="EngineX, tableau de bord"><Logo size={24} /></a>
  <button class="act" onclick={onPalette} use:tooltip={wide ? null : { text: 'Commandes', side: 'right', kbd: 'Ctrl K' }} aria-label="Commandes (Ctrl+K)">
    <Icon icon={Search} size={20} />{#if wide}<span class="lb">Commandes</span><kbd>Ctrl K</kbd>{/if}
  </button>
  {#each groups as g, i (i)}
    <hr />
    {#each g as p (p.id)}
      <a class="act" class:on={page === p.id} href="#{p.id}" aria-current={page === p.id ? 'page' : undefined} aria-label={p.label} use:tooltip={tip(p.label)}>
        <Icon icon={p.icon} size={22} />{#if wide}<span class="lb">{p.label}</span>{/if}
      </a>
    {/each}
  {/each}
  <span class="end"></span>
  <button
    bind:this={accountBtn}
    class="act"
    class:on={account}
    aria-label="Compte"
    aria-haspopup="menu"
    aria-expanded={account}
    use:tooltip={tip('Compte')}
    onclick={() => (account = !account)}
  >
    <Icon icon={CircleUser} size={22} />{#if wide}<span class="lb">Compte</span>{/if}
  </button>
  <a class="act" class:on={page === settings.id} href="#settings" aria-current={page === 'settings' ? 'page' : undefined} aria-label={settings.label} use:tooltip={tip(settings.label)}>
    <Icon icon={settings.icon} size={22} />{#if wide}<span class="lb">{settings.label}</span>{/if}
  </a>
  <button class="act" aria-label={wide ? 'Replier la barre latérale' : 'Déplier la barre latérale'} aria-expanded={wide}
    use:tooltip={tip('Déplier la barre latérale')} onclick={() => (wide = !wide)}>
    <Icon icon={wide ? PanelLeftClose : PanelLeftOpen} size={22} />{#if wide}<span class="lb">Replier</span>{/if}
  </button>
</nav>

<Menu
  bind:open={account}
  anchor={accountBtn}
  label="Compte"
  items={[
    { label: 'Changer le mot de passe', icon: KeyRound, onclick: onPassword },
    { label: 'Se déconnecter', icon: LogOut, onclick: onLogout },
  ]}
/>

<style>
  .activity {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    width: var(--act-w);
    padding: var(--s2) 0;
    background: var(--panel);
    border-right: 1px solid var(--line);
    overflow-y: auto;
    scrollbar-width: none;
  }
  .logo { display: grid; place-items: center; width: 40px; height: 36px; margin-bottom: 2px; color: var(--accent); border-radius: var(--r); }
  .act {
    position: relative;
    display: grid;
    place-items: center;
    flex: none;
    width: 40px;
    height: 40px;
    min-height: 0;
    padding: 0;
    border: 0;
    border-radius: var(--r);
    background: transparent;
    color: var(--faint);
    cursor: pointer;
    filter: none;
  }
  .act:hover { color: var(--text); background: var(--raised); }
  .act.on { color: var(--text); }
  .act.on::before { content: ''; position: absolute; left: -6px; top: 8px; bottom: 8px; width: 2px; border-radius: 1px; background: var(--accent); }
  .wide { align-items: stretch; padding-left: var(--s2); padding-right: var(--s2); }
  .wide .act, .wide .logo { width: auto; justify-content: start; grid-auto-flow: column; grid-template-columns: 24px 1fr auto; gap: var(--s3); padding: 0 var(--s3); text-align: left; }
  .wide .act { place-items: center start; font: inherit; font-size: var(--fs-sm); }
  .wide .act.on::before { left: -8px; }
  .lb { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; justify-self: start; }
  kbd { font: inherit; font-size: var(--fs-xs); color: var(--faint); }
  .wide hr { width: auto; }
  hr { width: 24px; margin: 6px 0; border: 0; border-top: 1px solid var(--line-strong); flex: none; }
  .end { margin-top: auto; }
</style>
