<script lang="ts">
  // Palette de commandes (Ctrl+K) : pages, sessions récentes, nouvelle session dans un topic, thème, taille du texte.
  // Mêmes appels que l'arbre des sessions (GET /api/sessions, /api/projects, POST /api/sessions).
  import MessageSquare from '@lucide/svelte/icons/message-square';
  import MessageSquarePlus from '@lucide/svelte/icons/message-square-plus';
  import Palette from '@lucide/svelte/icons/palette';
  import ALargeSmall from '@lucide/svelte/icons/a-large-small';
  import Search from '@lucide/svelte/icons/search';
  import Icon from '../../lib/ui/Icon.svelte';
  import Kbd from '../../lib/ui/Kbd.svelte';
  import { toast } from '../../lib/ui/toast.svelte';
  import { setTheme, setSize, THEME_LABEL, SIZE_LABEL, type ThemeChoice, type TextSize } from '../../lib/ui/theme.svelte';
  import type { IconType } from '../../lib/ui/types';
  import { sessions, projects, createSession, message, type Session, type Project } from '../../lib/api';
  import { PAGES } from './pages';

  let { open = $bindable(false) }: { open?: boolean } = $props();

  interface Cmd {
    id: string;
    group: string;
    label: string;
    hint?: string;
    icon: IconType;
    run: () => void | Promise<void>;
  }

  let el: HTMLDialogElement | undefined = $state();
  let input: HTMLInputElement | undefined = $state();
  let q = $state('');
  let sel = $state(0);
  let recent = $state<Session[]>([]);
  let projs = $state<Project[]>([]);

  const go = (hash: string) => () => {
    location.hash = hash;
  };

  async function load() {
    const [s, p] = await Promise.allSettled([sessions({}), projects()]);
    if (s.status === 'fulfilled') recent = [...s.value].sort((a, z) => z.updated_at - a.updated_at).slice(0, 8);
    if (p.status === 'fulfilled') projs = p.value;
  }

  const topicName = (id: number) => {
    for (const p of projs) for (const t of p.topics) if (t.id === id) return `${p.name} / ${t.name}`;
    return '';
  };

  const cmds = $derived.by<Cmd[]>(() => [
    ...PAGES.map((p) => ({ id: `p-${p.id}`, group: 'Aller à', label: p.label, icon: p.icon, run: go(p.id) })),
    ...recent.map((s) => ({
      id: `s-${s.id}`,
      group: 'Sessions récentes',
      label: s.title,
      hint: topicName(s.topic_id),
      icon: MessageSquare,
      run: go(`sessions/${s.id}`),
    })),
    ...projs.flatMap((p) =>
      p.topics.map((t) => ({
        id: `n-${t.id}`,
        group: 'Nouvelle session',
        label: `Nouvelle session dans ${p.name} / ${t.name}`,
        icon: MessageSquarePlus,
        run: async () => {
          try {
            const s = await createSession(t.id, 'auto', false);
            location.hash = `sessions/${s.id}`;
          } catch (e) {
            toast(message(e), 'error');
          }
        },
      })),
    ),
    ...(['system', 'light', 'dark'] as ThemeChoice[]).map((t) => ({
      id: `t-${t}`,
      group: 'Apparence',
      label: `Thème : ${THEME_LABEL[t]}`,
      icon: Palette,
      run: () => setTheme(t),
    })),
    ...(['s', 'm', 'l'] as TextSize[]).map((s) => ({
      id: `z-${s}`,
      group: 'Apparence',
      label: `Taille du texte : ${SIZE_LABEL[s]}`,
      icon: ALargeSmall,
      run: () => setSize(s),
    })),
  ]);

  const fold = (s: string) => s.normalize('NFD').replace(/[\u0300-\u036f]/g, '').toLowerCase();
  const shown = $derived.by(() => {
    const words = fold(q).split(/\s+/).filter(Boolean);
    if (!words.length) return cmds.filter((c) => c.group !== 'Nouvelle session' || projs.length <= 4);
    return cmds.filter((c) => {
      const hay = fold(`${c.label} ${c.hint ?? ''} ${c.group}`);
      return words.every((w) => hay.includes(w));
    });
  });

  $effect(() => {
    if (!el) return;
    if (open && !el.open) {
      q = '';
      sel = 0;
      el.showModal();
      input?.focus();
      load();
    } else if (!open && el.open) el.close();
  });
  $effect(() => {
    void q;
    sel = 0;
  });

  function run(c: Cmd | undefined) {
    if (!c) return;
    open = false;
    c.run();
  }

  function key(e: KeyboardEvent) {
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const n = shown.length;
      if (!n) return;
      sel = (sel + (e.key === 'ArrowDown' ? 1 : n - 1)) % n;
      el?.querySelector(`[data-i="${sel}"]`)?.scrollIntoView({ block: 'nearest' });
    } else if (e.key === 'Enter') {
      e.preventDefault();
      run(shown[sel]);
    }
  }
</script>

<dialog bind:this={el} class="pal" aria-label="Commandes" onclose={() => (open = false)} onclick={(e) => e.target === el && (open = false)}>
  {#if open}
    <div class="search">
      <Icon icon={Search} size={18} />
      <input
        bind:this={input}
        bind:value={q}
        onkeydown={key}
        placeholder="Aller à une page, une session, une commande"
        aria-label="Chercher une commande"
        role="combobox"
        aria-expanded="true"
        aria-controls="pal-list"
        aria-activedescendant={shown[sel] ? `pal-${shown[sel].id}` : undefined}
        autocomplete="off"
        spellcheck="false"
      />
      <Kbd>Échap</Kbd>
    </div>
    <ul id="pal-list" role="listbox" aria-label="Résultats">
      {#each shown as c, i (c.id)}
        {#if i === 0 || shown[i - 1].group !== c.group}<li class="grp" role="presentation">{c.group}</li>{/if}
        <li
          id="pal-{c.id}"
          role="option"
          aria-selected={i === sel}
          data-i={i}
          class:sel={i === sel}
          onclick={() => run(c)}
          onkeydown={() => {}}
          onmousemove={() => (sel = i)}
        >
          <Icon icon={c.icon} size={16} />
          <span class="ell lbl">{c.label}</span>
          {#if c.hint}<span class="hint ell">{c.hint}</span>{/if}
        </li>
      {:else}
        <li class="none" role="presentation">Rien ne correspond à « {q} ».</li>
      {/each}
    </ul>
    <div class="foot"><Kbd>↑</Kbd><Kbd>↓</Kbd> choisir <Kbd>Entrée</Kbd> ouvrir</div>
  {/if}
</dialog>

<style>
  .pal {
    width: min(620px, calc(100vw - 24px));
    margin: 12vh auto auto;
    padding: 0;
    border: 1px solid var(--line-strong);
    border-radius: var(--r);
    background: var(--panel);
    color: var(--text);
    box-shadow: var(--shadow);
    overflow: hidden;
  }
  .pal::backdrop { background: var(--backdrop); }
  .search { display: flex; align-items: center; gap: var(--s3); height: 52px; padding: 0 var(--s4); border-bottom: 1px solid var(--line); color: var(--faint); }
  .search input { flex: 1; min-width: 0; height: 100%; padding: 0; border: 0; background: none; outline: none; color: var(--text); font: inherit; font-size: var(--fs-md); }
  .search input:focus-visible { outline: none; }
  ul { list-style: none; margin: 0; padding: var(--s1) 0; max-height: min(420px, 60dvh); overflow: auto; }
  .grp { padding: var(--s2) var(--s4) var(--s1); font-size: var(--fs-xs); color: var(--faint); }
  [role='option'] { display: flex; align-items: center; gap: var(--s3); height: 40px; margin: 0 var(--s1); padding: 0 var(--s3); border-radius: var(--r); cursor: pointer; color: var(--muted); font-size: var(--fs-sm); }
  [role='option'] .lbl { color: var(--text); flex: 0 1 auto; }
  .hint { margin-left: auto; color: var(--faint); font-size: var(--fs-xs); flex: 0 1 auto; }
  .sel { background: var(--accent-soft); box-shadow: inset 2px 0 var(--accent); }
  .none { padding: var(--s4); color: var(--muted); font-size: var(--fs-sm); }
  .foot { display: flex; align-items: center; gap: var(--s2); padding: var(--s2) var(--s4); border-top: 1px solid var(--line); font-size: var(--fs-xs); color: var(--faint); }
  @media (max-width: 767px) {
    .pal { margin: calc(var(--safe-top) + 8px) auto auto; }
    [role='option'] { height: var(--touch); }
    .foot { display: none; }
  }
</style>
