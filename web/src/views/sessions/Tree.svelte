<script lang="ts">
  import {
    projects as loadProjects, sessions as loadSessions, createProject, createTopic, deleteProject, deleteTopic,
    createSession, patchSession, deleteSession, message, type Project, type Session,
  } from '../../lib/api';

  let { selected, version, open }: { selected: string; version: number; open: (id: string) => void } = $props();

  let projects = $state<Project[]>([]);
  let byTopic = $state<Record<number, Session[]>>({});
  let closed = $state<Record<number, boolean>>({});
  let search = $state('');
  let archived = $state(false);
  let found = $state<Session[] | null>(null);
  let error = $state('');

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

  const ask = (q: string, v = '') => prompt(q, v)?.trim();

  function newProject() {
    const name = ask('Nom du projet');
    if (name) act(() => createProject(name));
  }
  function newTopic(p: Project) {
    const name = ask(`Nouveau topic dans « ${p.name} »`);
    if (name) act(() => createTopic(p.id, name));
  }
  async function newSession(topic: number) {
    try {
      const s = await createSession(topic);
      await load();
      open(s.id);
    } catch (e) {
      error = message(e);
    }
  }
  function rename(s: Session) {
    const title = ask('Titre de la session', s.title);
    if (title) act(() => patchSession(s.id, { title }));
  }
  function remove(s: Session) {
    if (confirm(`Supprimer la session « ${s.title} » et son historique ? Les fichiers du topic sont gardés.`))
      act(() => deleteSession(s.id));
  }
  function removeTopic(name: string, id: number) {
    if (confirm(`Supprimer le topic « ${name} » ? Il doit être vide (ni session ni fichier).`)) act(() => deleteTopic(id));
  }
  function removeProject(p: Project) {
    if (confirm(`Supprimer le projet « ${p.name} » ? Ses topics doivent être vides.`)) act(() => deleteProject(p.id));
  }
</script>

{#snippet line(s: Session)}
  <li class:current={s.id === selected}>
    <a href="#sessions/{s.id}" class="sess" title={s.title}>
      {#if s.running}<span class="dot" title="Tour en cours"></span>{/if}{s.title}
    </a>
    <details class="menu">
      <summary aria-label="Actions">⋯</summary>
      <div class="pop">
        <button class="link" onclick={() => rename(s)}>Renommer</button>
        <button class="link" onclick={() => act(() => patchSession(s.id, { archived: !s.archived }))}>
          {s.archived ? 'Désarchiver' : 'Archiver'}
        </button>
        <button class="link danger" onclick={() => remove(s)}>Supprimer</button>
      </div>
    </details>
  </li>
{/snippet}

<aside class="tree">
  <input type="search" placeholder="Chercher dans les sessions" bind:value={search} oninput={onSearch} />
  <label class="check small"><input type="checkbox" bind:checked={archived} /> Archivées</label>
  {#if error}<p class="error small">{error}</p>{/if}

  {#if found}
    <ul class="list">
      {#each found as s (s.id)}{@render line(s)}{:else}<li class="muted small">Aucun résultat.</li>{/each}
    </ul>
  {:else}
    {#each projects as p (p.id)}
      <div class="project">
        <div class="row">
          <strong>{p.name}</strong>
          <span class="tools">
            <button class="link small" onclick={() => newTopic(p)} title="Nouveau topic">+ topic</button>
            <button class="link small" onclick={() => removeProject(p)} title="Supprimer le projet">×</button>
          </span>
        </div>
        {#each p.topics as t (t.id)}
          <div class="topic">
            <div class="row">
              <button class="link" onclick={() => (closed[t.id] = !closed[t.id])}>{closed[t.id] ? '▸' : '▾'} {t.name}</button>
              <span class="tools">
                <button class="link small" onclick={() => newSession(t.id)} title="Nouvelle session">+ session</button>
                <button class="link small" onclick={() => removeTopic(t.name, t.id)} title="Supprimer le topic">×</button>
              </span>
            </div>
            {#if !closed[t.id]}
              <ul class="list">
                {#each byTopic[t.id] ?? [] as s (s.id)}{@render line(s)}{:else}<li class="muted small">Aucune session.</li>{/each}
              </ul>
            {/if}
          </div>
        {/each}
      </div>
    {/each}
    <button class="secondary small" onclick={newProject}>Nouveau projet</button>
  {/if}
</aside>

<style>
  .tree { display: flex; flex-direction: column; gap: var(--s2); overflow: auto; min-height: 0; }
  .project { display: flex; flex-direction: column; gap: var(--s1); margin-top: var(--s2); }
  .topic { padding-left: var(--s2); }
  .tools { display: flex; gap: var(--s2); visibility: hidden; }
  .row:hover .tools { visibility: visible; }
  .list { list-style: none; margin: 0; padding-left: var(--s4); }
  .list li { display: flex; align-items: center; justify-content: space-between; gap: var(--s1); }
  .list li.current .sess { color: var(--accent); }
  .sess { color: var(--text); text-decoration: none; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 1; font-size: 14px; padding: 2px 0; }
  .dot { display: inline-block; width: 7px; height: 7px; border-radius: 50%; background: var(--accent); margin-right: var(--s1); vertical-align: middle; }
  .menu { position: relative; }
  .menu summary { list-style: none; cursor: pointer; color: var(--muted); padding: 0 var(--s1); }
  .menu summary::-webkit-details-marker { display: none; }
  .pop { position: absolute; right: 0; z-index: 2; background: var(--panel); border: 1px solid var(--line); border-radius: var(--radius); padding: var(--s2) var(--s3); display: flex; flex-direction: column; gap: var(--s1); min-width: 130px; }
  .danger { color: var(--error); }
  input[type='search'] { width: 100%; }
</style>
