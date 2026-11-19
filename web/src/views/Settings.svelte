<script lang="ts">
  import Cpu from '@lucide/svelte/icons/cpu';
  import MessageSquare from '@lucide/svelte/icons/message-square';
  import Sparkles from '@lucide/svelte/icons/sparkles';
  import KeyRound from '@lucide/svelte/icons/key-round';
  import Plug from '@lucide/svelte/icons/plug';
  import Palette from '@lucide/svelte/icons/palette';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import ArrowLeft from '@lucide/svelte/icons/arrow-left';
  import Icon from '../lib/ui/Icon.svelte';
  import Engine from './settings/Engine.svelte';
  import Api from './settings/Api.svelte';
  import Vault from './settings/Vault.svelte';
  import Sessions from './settings/Sessions.svelte';
  import Skills from './settings/Skills.svelte';
  import Appearance from './settings/Appearance.svelte';

  let { sub }: { sub: string } = $props();
  const tabs = [
    { id: 'engine', label: 'Moteur', text: 'llama.cpp, versions, mise à jour', icon: Cpu },
    { id: 'sessions', label: 'Sessions', text: 'Sandboxes, vérification, git', icon: MessageSquare },
    { id: 'skills', label: 'Skills', text: 'Fichiers SKILL.md donnés à Codex', icon: Sparkles },
    { id: 'vault', label: 'Clés', text: 'Secrets des sandboxes', icon: KeyRound },
    { id: 'api', label: 'API', text: 'Clés d’API et configuration de Codex', icon: Plug },
    { id: 'appearance', label: 'Apparence', text: 'Thème et taille du texte', icon: Palette },
  ];
  // Bureau : #settings seul ouvre Moteur. Téléphone : #settings seul montre la liste.
  let tab = $derived(tabs.some((t) => t.id === sub) ? sub : 'engine');
  let current = $derived(tabs.find((t) => t.id === tab)!);
</script>

<div class="settings" class:at-list={sub === ''}>
  <nav class="side" aria-label="Paramètres">
    <h1>Paramètres</h1>
    <ul>
      {#each tabs as t (t.id)}
        <li>
          <a href="#settings/{t.id}" class:on={tab === t.id} aria-current={tab === t.id ? 'page' : undefined}>
            <Icon icon={t.icon} size={18} />
            <span class="lbl"><span>{t.label}</span><small>{t.text}</small></span>
            <span class="chev"><Icon icon={ChevronRight} size={18} /></span>
          </a>
        </li>
      {/each}
    </ul>
  </nav>

  <section class="content" aria-labelledby="settings-title">
    <a class="back" href="#settings"><Icon icon={ArrowLeft} size={18} />Paramètres</a>
    <header class="ch">
      <h2 id="settings-title">{current.label}</h2>
    </header>
    {#if tab === 'api'}<Api />{:else if tab === 'vault'}<Vault />{:else if tab === 'sessions'}<Sessions />{:else if tab === 'skills'}<Skills />{:else if tab === 'appearance'}<Appearance />{:else}<Engine />{/if}
  </section>
</div>

<style>
  .settings {
    margin: calc(-1 * var(--s6));
    min-height: calc(100dvh - var(--status-h));
    display: grid;
    grid-template-columns: 260px minmax(0, 1fr);
    background: var(--bg);
  }
  .side { background: var(--panel); border-right: 1px solid var(--line); }
  .side h1 { margin: 0; padding: var(--s5) var(--s5) var(--s3); font-size: var(--fs-lg); font-weight: 600; }
  ul { list-style: none; margin: 0; padding: 0 0 var(--s4); position: sticky; top: 0; }
  .side a {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--s3);
    min-height: 40px;
    padding: 0 var(--s5);
    color: var(--muted);
    text-decoration: none;
  }
  @media (hover: hover) { .side a:hover { background: var(--raised); color: var(--text); } }
  .side a.on { background: var(--accent-soft); color: var(--text); }
  .side a.on::before { content: ''; position: absolute; left: 0; top: 0; bottom: 0; width: 2px; background: var(--accent); }
  .side a.on :global(.ui-icon) { color: var(--accent); }
  .lbl { display: flex; flex-direction: column; flex: 1; min-width: 0; }
  .lbl small { display: none; }
  .chev { display: none; }
  .content { min-width: 0; }
  .ch { padding: var(--s5) var(--s6) var(--s4); border-bottom: 1px solid var(--line); }
  .ch h2 { margin: 0; font-size: var(--fs-lg); font-weight: 600; }
  .back { display: none; }

  @media (max-width: 767px) {
    .settings { margin: calc(-1 * var(--s4)) calc(-1 * (var(--s4) + var(--safe-right))) calc(-1 * var(--s4)) calc(-1 * (var(--s4) + var(--safe-left))); min-height: 0; display: block; }
    .side { background: var(--bg); border-right: 0; }
    .side h1 { padding: var(--s5) var(--s4) var(--s3); font-size: var(--fs-xl); font-weight: 400; }
    ul { border-top: 1px solid var(--line); }
    .side a { min-height: 60px; padding: var(--s2) var(--s3) var(--s2) var(--s4); border-bottom: 1px solid var(--line); color: var(--text); }
    .side a.on { background: none; }
    .side a.on::before { display: none; }
    .side a.on :global(.ui-icon) { color: inherit; }
    .lbl small { display: block; color: var(--muted); font-size: var(--fs-sm); }
    .chev { display: flex; color: var(--faint); }
    .settings:not(.at-list) .side { display: none; }
    .settings.at-list .content { display: none; }
    .back {
      display: flex;
      align-items: center;
      gap: var(--s2);
      height: var(--touch);
      padding: 0 var(--s4);
      color: var(--muted);
      text-decoration: none;
      border-bottom: 1px solid var(--line);
    }
    .ch { padding: var(--s4); }
    .ch h2 { font-size: var(--fs-xl); font-weight: 400; }
  }
</style>
