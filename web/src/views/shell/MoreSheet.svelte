<script lang="ts">
  // « Plus » du téléphone : les autres pages, l'apparence et le compte.
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import KeyRound from '@lucide/svelte/icons/key-round';
  import LogOut from '@lucide/svelte/icons/log-out';
  import Search from '@lucide/svelte/icons/search';
  import Sheet from '../../lib/ui/Sheet.svelte';
  import Icon from '../../lib/ui/Icon.svelte';
  import Segmented from '../../lib/ui/Segmented.svelte';
  import { ui, setTheme, setSize } from '../../lib/ui/theme.svelte';
  import { PAGES, PHONE_TABS } from './pages';

  let {
    open = $bindable(false),
    page,
    onPalette,
    onPassword,
    onLogout,
  }: { open?: boolean; page: string; onPalette: () => void; onPassword: () => void; onLogout: () => void } = $props();

  const rest = PAGES.filter((p) => !PHONE_TABS.includes(p.id));
  const then = (fn: () => void) => () => {
    open = false;
    fn();
  };
</script>

<Sheet bind:open title="Plus">
  <div class="rows">
    {#each rest as p (p.id)}
      <a href="#{p.id}" class:on={page === p.id} onclick={() => (open = false)}>
        <Icon icon={p.icon} size={22} /><span class="grow">{p.label}</span><Icon icon={ChevronRight} size={18} class="chev" />
      </a>
    {/each}
    <button type="button" onclick={then(onPalette)}><Icon icon={Search} size={22} /><span class="grow">Commandes</span></button>
  </div>
  <section>
    <h3>Apparence</h3>
    <div class="line"><span>Thème</span><Segmented label="Thème" size="lg" value={ui.theme} onchange={setTheme} options={[{ value: 'system', label: 'Système' }, { value: 'light', label: 'Clair' }, { value: 'dark', label: 'Sombre' }]} /></div>
    <div class="line"><span>Taille du texte</span><Segmented label="Taille du texte" size="lg" value={ui.size} onchange={setSize} options={[{ value: 's', label: 'S' }, { value: 'm', label: 'M' }, { value: 'l', label: 'L' }]} /></div>
  </section>
  <div class="rows">
    <button type="button" onclick={then(onPassword)}><Icon icon={KeyRound} size={22} /><span class="grow">Changer le mot de passe</span></button>
    <button type="button" class="danger" onclick={then(onLogout)}><Icon icon={LogOut} size={22} /><span class="grow">Se déconnecter</span></button>
  </div>
</Sheet>

<style>
  .rows { display: flex; flex-direction: column; border-bottom: 1px solid var(--line); }
  .rows a, .rows button {
    display: flex;
    align-items: center;
    gap: var(--s3);
    min-height: 56px;
    padding: 0 var(--s4);
    border: 0;
    border-top: 1px solid var(--line);
    border-radius: 0;
    background: none;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-md);
    font-weight: 400;
    text-align: left;
    text-decoration: none;
    justify-content: flex-start;
    filter: none;
  }
  .rows > :first-child { border-top: 0; }
  .rows a:active, .rows button:active { background: var(--raised); }
  .rows .on { background: var(--accent-soft); box-shadow: inset 2px 0 var(--accent); }
  .rows :global(.chev) { color: var(--faint); }
  .rows .danger { color: var(--danger); }
  .grow { flex: 1; }
  section { padding: var(--s4); border-bottom: 1px solid var(--line); display: flex; flex-direction: column; gap: var(--s3); }
  h3 { margin: 0; font-size: var(--fs-md); }
  .line { display: flex; align-items: center; justify-content: space-between; gap: var(--s3); flex-wrap: wrap; color: var(--muted); font-size: var(--fs-sm); }
</style>
