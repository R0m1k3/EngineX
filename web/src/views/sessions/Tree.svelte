<script lang="ts">
  import {
    projects as loadProjects, sessions as loadSessions, createProject, createTopic, deleteProject, deleteTopic,
    createSession, patchSession, deleteSession, message, type Project, type Session,
  } from '../../lib/api';
  import Plus from '@lucide/svelte/icons/plus';
  import Search from '@lucide/svelte/icons/search';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import Ellipsis from '@lucide/svelte/icons/ellipsis';
  import GitBranch from '@lucide/svelte/icons/git-branch';
  import Trash from '@lucide/svelte/icons/trash-2';
  import FolderPlus from '@lucide/svelte/icons/folder-plus';
  import Pencil from '@lucide/svelte/icons/pencil';
  import Archive from '@lucide/svelte/icons/archive';
  import ArchiveRestore from '@lucide/svelte/icons/archive-arrow-up';
  import Button from '../../lib/ui/Button.svelte';
  import IconButton from '../../lib/ui/IconButton.svelte';
  import Input from '../../lib/ui/Input.svelte';
  import Icon from '../../lib/ui/Icon.svelte';
  import StatusDot from '../../lib/ui/StatusDot.svelte';
  import Skeleton from '../../lib/ui/Skeleton.svelte';
  import { tooltip } from '../../lib/ui/tooltip';
  import { confirmDialog, promptDialog } from '../../lib/ui/dialog.svelte';
  import { toast } from '../../lib/ui/toast.svelte';
  import Menu from '../../lib/ui/Menu.svelte';
  import { screen } from './phone.svelte';

  let { selected, version, open, onNew, projects = $bindable([]) }: {
    selected: string;
    version: number;
    open: (id: string) => void;
    /** Ouvre « Nouvelle session » (topic proposé). */
    onNew: (topic?: number | null) => void;
    projects?: Project[];
  } = $props();

  let byTopic = $state<Record<number, Session[]>>({});
  let closed = $state<Record<number, boolean>>({});
  let search = $state('');
  let archived = $state(false);
  let found = $state<Session[] | null>(null);
  let error = $state('');
  let loaded = $state(false);

  async function load() {
    try {
      projects = await loadProjects();
      const topics = projects.flatMap((p) => p.topics);
      const lists = await Promise.all(topics.map((t) => loadSessions({ topic: t.id, archived })));
      byTopic = Object.fromEntries(topics.map((t, i) => [t.id, lists[i]]));
      found = search.trim() ? await loadSessions({ q: search.trim(), archived }) : null;
      error = '';
    } catch (e) {
      error = message(e);
    } finally {
      loaded = true;
    }
  }

  $effect(() => {
    void version;
    void archived;
    load();
  });

  let timer: ReturnType<typeof setTimeout> | undefined;
  function onSearch() {
    clearTimeout(timer);
    timer = setTimeout(load, 250);
  }

  async function act(fn: () => Promise<unknown>) {
    try {
      await fn();
      await load();
    } catch (e) {
      error = message(e);
    }
  }

  // Topic de la session ouverte : proposé par « Nouvelle session ».
  const currentTopic = $derived(
    Number(Object.entries(byTopic).find(([, l]) => l.some((s) => s.id === selected))?.[0]) || null,
  );
  const where = (topicId: number) => {
    for (const p of projects) {
      const t = p.topics.find((x) => x.id === topicId);
      if (t) return `${p.name} / ${t.name}`;
    }
    return '';
  };

  async function newProject() {
    const name = await promptDialog('Nom du projet', { title: 'Nouveau projet', confirm: 'Créer le projet' });
    if (name) act(() => createProject(name));
  }
  async function newTopic(p: Project) {
    const name = await promptDialog('Nom du topic', { title: `Nouveau topic dans « ${p.name} »`, confirm: 'Créer le topic' });
    if (name) act(() => createTopic(p.id, name));
  }
  let creating = $state<number | null>(null);
  async function newSession(topic: number, copy = false) {
    if (
      copy &&
      !(await confirmDialog(
        'La session travaillera sur sa propre branche. Les modifications non commitées du topic ne sont pas copiées.',
        { title: 'Créer une copie séparée du dépôt git ?', confirm: 'Créer la copie' },
      ))
    )
      return;
    creating = copy ? topic : null;
    try {
      const s = await createSession(topic, 'auto', copy);
      await load();
      open(s.id);
    } catch (e) {
      error = message(e);
    } finally {
      creating = null;
    }
  }
  async function rename(s: Session) {
    const title = await promptDialog('Titre', { title: 'Renommer la session', value: s.title, confirm: 'Renommer' });
    if (title) act(() => patchSession(s.id, { title }));
  }
  async function remove(s: Session) {
    const msg = s.copy_branch
      ? 'La copie et sa branche seront supprimées ; ce qui n’a pas été ramené dans le topic est perdu.'
      : 'Son historique est supprimé. Les fichiers du topic sont gardés.';
    if (await confirmDialog(msg, { title: `Supprimer « ${s.title} » ?`, confirm: 'Supprimer', danger: true }))
      act(() => deleteSession(s.id));
  }
  async function removeTopic(name: string, id: number) {
    if (
      await confirmDialog('Il doit être vide (ni session ni fichier). Sa mémoire et ses instructions seront supprimées avec lui.', {
        title: `Supprimer le topic « ${name} » ?`,
        confirm: 'Supprimer',
        danger: true,
      })
    )
      act(() => deleteTopic(id));
  }
  async function removeProject(p: Project) {
    if (
      await confirmDialog('Ses topics doivent être vides. La mémoire et les instructions du projet et de ses topics seront supprimées.', {
        title: `Supprimer le projet « ${p.name} » ?`,
        confirm: 'Supprimer',
        danger: true,
      })
    )
      act(() => deleteProject(p.id));
  }

  // Un seul menu ⋯ ouvert à la fois, ancré à la ligne choisie.
  let menuOpen = $state(false);
  let menuAnchor = $state<HTMLElement>();
  let menuFor = $state<Session | null>(null);
  function menu(e: MouseEvent, s: Session) {
    menuAnchor = e.currentTarget as HTMLElement;
    menuFor = s;
    menuOpen = true;
  }
  const menuItems = $derived(
    menuFor
      ? [
          { label: 'Renommer', icon: Pencil, onclick: () => rename(menuFor!) },
          {
            label: menuFor.archived ? 'Désarchiver' : 'Archiver',
            icon: menuFor.archived ? ArchiveRestore : Archive,
            onclick: () => {
              const s = menuFor!;
              act(() => patchSession(s.id, { archived: !s.archived })).then(() =>
                toast(s.archived ? 'Session désarchivée.' : 'Session archivée.', 'ok'),
              );
            },
          },
          { label: 'Supprimer', icon: Trash, danger: true, onclick: () => remove(menuFor!) },
        ]
      : [],
  );
</script>

{#snippet line(s: Session, sub = false)}
  <li class="sess" class:on={s.id === selected}>
    <a href="#sessions/{s.id}" aria-current={s.id === selected ? 'page' : undefined}>
      <span class="t">
        <span class="ell">{s.title}</span>
        {#if sub}<span class="where ell">{where(s.topic_id)}</span>{/if}
      </span>
      {#if s.copy_branch}<span class="copy" use:tooltip={`Copie séparée : ${s.copy_branch}`}><Icon icon={GitBranch} size={14} />copie</span>{/if}
      {#if s.running}<StatusDot kind="ok" label="Tour en cours" />{/if}
    </a>
    <IconButton icon={Ellipsis} label="Actions de la session" size={screen.phone ? 'lg' : 'sm'} class="more" onclick={(e) => menu(e, s)} />
  </li>
{/snippet}

<aside class="tree" aria-label="Sessions">
  <div class="top">
    <Button variant="primary" icon={Plus} full size={screen.phone ? 'lg' : 'md'} onclick={() => onNew(currentTopic)}>Nouvelle session</Button>
    <Input type="search" icon={Search} placeholder="Chercher dans les sessions" aria-label="Chercher dans les sessions"
      size={screen.phone ? 'lg' : 'md'} bind:value={search} oninput={onSearch} />
    <label class="arch"><input type="checkbox" bind:checked={archived} /> Archivées</label>
  </div>
  {#if error}<p class="error small err" role="alert">{error}</p>{/if}

  <div class="list">
    {#if !loaded}
      <div class="sk"><Skeleton lines={6} /></div>
    {:else if found}
      <ul>
        {#each found as s (s.id)}{@render line(s, true)}{:else}<li class="none">Aucune session ne contient « {search.trim()} ».</li>{/each}
      </ul>
    {:else}
      {#each projects as p (p.id)}
        <div class="proj row-h">
          <button class="fold" aria-expanded={!closed[-p.id]} onclick={() => (closed[-p.id] = !closed[-p.id])}>
            <Icon icon={ChevronRight} size={16} class="chev" /><span class="ell">{p.name}</span>
          </button>
          <span class="tools">
            <IconButton icon={Plus} label="Nouveau topic" size="sm" onclick={() => newTopic(p)} />
            <IconButton icon={Trash} label="Supprimer le projet" size="sm" onclick={() => removeProject(p)} />
          </span>
        </div>
        {#if !closed[-p.id]}
          {#each p.topics as t (t.id)}
            {@const list = byTopic[t.id] ?? []}
            <div class="topic row-h">
              <button class="fold" aria-expanded={!closed[t.id]} onclick={() => (closed[t.id] = !closed[t.id])}>
                <Icon icon={ChevronRight} size={16} class="chev" /><span class="ell">{t.name}</span>
                {#if closed[t.id] && list.length}<span class="n num">{list.length}</span>{/if}
              </button>
              <span class="tools">
                <IconButton icon={Plus} label="Nouvelle session" size="sm" onclick={() => newSession(t.id)} />
                {#if t.git}
                  <IconButton icon={GitBranch} label={creating === t.id ? 'Création de la copie…' : 'Nouvelle session dans une copie séparée du dépôt git'}
                    size="sm" disabled={creating === t.id} onclick={() => newSession(t.id, true)} />
                {/if}
                <IconButton icon={Trash} label="Supprimer le topic" size="sm" onclick={() => removeTopic(t.name, t.id)} />
              </span>
            </div>
            {#if !closed[t.id]}
              <ul>
                {#each list as s (s.id)}{@render line(s)}{:else}<li class="none">{archived ? 'Aucune session archivée.' : 'Aucune session.'}</li>{/each}
              </ul>
            {/if}
          {/each}
        {/if}
      {/each}
      <div class="foot">
        <Button variant="ghost" icon={FolderPlus} size={screen.phone ? 'lg' : 'sm'} onclick={newProject}>Nouveau projet</Button>
      </div>
    {/if}
  </div>
</aside>

<Menu side="bottom-end" bind:open={menuOpen} anchor={menuAnchor} items={menuItems} label="Actions de la session" />

<style>
  .tree { display: flex; flex-direction: column; min-height: 0; height: 100%; background: var(--panel); }
  .top { display: flex; flex-direction: column; gap: var(--s2); padding: var(--s3); border-bottom: 1px solid var(--line); flex: none; }
  .arch { flex-direction: row; align-items: center; gap: var(--s2); align-self: flex-start; min-height: 28px; cursor: pointer; }
  .err { margin: var(--s2) var(--s3) 0; }
  .list { flex: 1; min-height: 0; overflow: auto; padding: var(--s2) 0 var(--s4); font-size: var(--fs-sm); }
  .sk { padding: var(--s2) var(--s3); }
  ul { list-style: none; margin: 0; padding: 0; }

  .row-h { display: flex; align-items: center; height: 32px; padding-right: var(--s2); }
  .fold {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    justify-content: flex-start;
    gap: var(--s2);
    height: 100%;
    min-height: 0;
    padding: 0;
    border: 0;
    border-radius: 0;
    background: none;
    color: inherit;
    font: inherit;
    font-weight: inherit;
    text-align: left;
    cursor: pointer;
    filter: none;
  }
  .fold :global(.chev) { color: var(--faint); flex: none; transition: transform 0.12s; }
  .fold[aria-expanded='true'] :global(.chev) { transform: rotate(90deg); }
  .proj { padding-left: var(--s3); margin-top: var(--s2); font-weight: 600; color: var(--text); }
  .proj:first-child { margin-top: 0; }
  .topic { padding-left: 28px; color: var(--muted); }
  .topic:hover, .proj:hover { color: var(--text); }
  .n { font-size: var(--fs-count); color: var(--muted); background: var(--raised); border-radius: var(--r-sm); padding: 0 6px; }
  .tools { display: flex; gap: 2px; opacity: 0; }
  .row-h:hover .tools, .row-h:focus-within .tools { opacity: 1; }

  .sess { display: flex; align-items: center; min-height: 32px; border-left: 2px solid transparent; padding-right: var(--s2); }
  .sess:hover { background: var(--raised); }
  .sess.on { background: var(--accent-soft); border-left-color: var(--accent); }
  .sess a { flex: 1; min-width: 0; display: flex; align-items: center; gap: var(--s2); padding: var(--s1) 0 var(--s1) 50px; color: var(--text); text-decoration: none; min-height: 32px; }
  .sess .t { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .where { color: var(--faint); }
  .copy { display: inline-flex; align-items: center; gap: 2px; color: var(--accent); flex: none; }
  .sess :global(.more) { opacity: 0; flex: none; }
  .sess:hover :global(.more), .sess:focus-within :global(.more), .sess.on :global(.more) { opacity: 1; }
  .none { padding: var(--s1) var(--s3) var(--s1) 52px; color: var(--faint); }
  .foot { padding: var(--s3) var(--s3) 0; }

  @media (max-width: 767px) {
    .tree { background: var(--bg); height: auto; }
    .top { padding: var(--s3) 0; border-bottom: 0; }
    .list { overflow: visible; font-size: var(--fs-md); padding-top: 0; }
    .row-h { height: var(--touch); padding-right: 0; }
    .proj { padding-left: 0; border-top: 1px solid var(--line); margin-top: var(--s3); padding-top: var(--s2); height: auto; min-height: var(--touch); }
    .proj:first-child { border-top: 0; margin-top: 0; }
    .topic { padding-left: var(--s4); }
    .tools { opacity: 1; }
    .tools :global(.ib) { width: var(--touch); height: var(--touch); }
    .sess { min-height: 56px; border-left-width: 0; border-bottom: 1px solid var(--line); padding-right: 0; }
    .sess:last-child { border-bottom: 0; }
    .sess:hover { background: none; }
    .sess.on { background: var(--accent-soft); }
    .sess a { padding-left: var(--s8); min-height: 56px; }
    .sess :global(.more) { opacity: 1; }
    .none { padding-left: var(--s8); min-height: var(--touch); display: flex; align-items: center; }
    .foot { padding: var(--s4) 0 0; }
  }
</style>
