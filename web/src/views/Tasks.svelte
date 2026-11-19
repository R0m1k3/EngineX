<script lang="ts">
  import { onDestroy } from 'svelte';
  import {
    board, createTask, patchTask, deleteTask, createTaskProject, deleteTaskProject, message,
    type Board, type Task, type TaskStatus,
  } from '../lib/api';

  const columns: { id: TaskStatus; label: string }[] = [
    { id: 'todo', label: 'À faire' },
    { id: 'doing', label: 'En cours' },
    { id: 'done', label: 'Fait' },
  ];

  let data = $state<Board | null>(null);
  let error = $state('');
  let filter = $state(''); // '' = tous, '-' = sans projet, sinon id
  let newTitle = $state('');
  let newProject = $state('');
  let editing = $state<Task | null>(null);
  let draft = $state({ title: '', notes: '', due: '', project_id: '' });
  let dragged = $state<string | null>(null);
  let over = $state<string | null>(null);

  const today = () => new Date().toISOString().slice(0, 10);
  const projectName = (id: string | null) => data?.projects.find((p) => p.id === id)?.name ?? '';
  const who = (b?: string) => (b?.startsWith('session:') ? 'IA' : '');

  let visible = $derived(
    (data?.tasks ?? []).filter((t) => !filter || (filter === '-' ? !t.project_id : t.project_id === filter)),
  );
  const inColumn = (s: TaskStatus) => visible.filter((t) => t.status === s).sort((a, b) => a.order - b.order);

  async function load() {
    try {
      data = await board();
      error = '';
      if (filter && filter !== '-' && !data.projects.some((p) => p.id === filter)) filter = '';
    } catch (e) {
      error = message(e);
    }
  }
  load();
  // L'IA ajoute des tâches pendant les sessions : rafraîchir tant qu'on n'édite pas.
  const timer = setInterval(() => {
    if (!editing && !dragged && document.visibilityState === 'visible') load();
  }, 5000);
  onDestroy(() => clearInterval(timer));

  async function act(f: () => Promise<unknown>) {
    error = '';
    try {
      await f();
    } catch (e) {
      error = message(e);
    }
    await load();
  }

  function add(e: SubmitEvent) {
    e.preventDefault();
    const title = newTitle.trim();
    if (!title) return;
    newTitle = '';
    act(() => createTask({ title, project_id: filter && filter !== '-' ? filter : null }));
  }

  function addProject(e: SubmitEvent) {
    e.preventDefault();
    const name = newProject.trim();
    if (!name) return;
    newProject = '';
    act(async () => {
      const p = await createTaskProject(name);
      filter = p.id;
    });
  }

  function removeProject() {
    const name = projectName(filter);
    if (!confirm(`Supprimer le projet « ${name} » ? Ses tâches restent, sans projet.`)) return;
    act(() => deleteTaskProject(filter));
    filter = '';
  }

  function open(t: Task) {
    editing = t;
    draft = { title: t.title, notes: t.notes, due: t.due ?? '', project_id: t.project_id ?? '' };
  }

  function save(e: SubmitEvent) {
    e.preventDefault();
    const t = editing;
    if (!t) return;
    editing = null;
    act(() =>
      patchTask(t.id, {
        title: draft.title,
        notes: draft.notes,
        due: draft.due || null,
        project_id: draft.project_id || null,
      }),
    );
  }

  function remove() {
    const t = editing;
    if (!t || !confirm(`Supprimer « ${t.title} » ?`)) return;
    editing = null;
    act(() => deleteTask(t.id));
  }

  /** Dépôt : avant la carte `before` (ou en bas de la colonne). */
  function drop(status: TaskStatus, before: Task | null) {
    const id = dragged;
    dragged = null;
    over = null;
    if (!id || id === before?.id) return;
    const col = inColumn(status).filter((t) => t.id !== id);
    let order: number;
    if (!before) {
      order = (col.at(-1)?.order ?? 0) + 1;
    } else {
      const i = col.findIndex((t) => t.id === before.id);
      const prev = i > 0 ? col[i - 1].order : before.order - 2;
      order = (prev + before.order) / 2;
    }
    const t = data?.tasks.find((x) => x.id === id);
    if (t) {
      t.status = status;
      t.order = order;
    }
    act(() => patchTask(id, { status, order }));
  }

  function move(t: Task, status: TaskStatus) {
    act(() => patchTask(t.id, { status }));
  }
</script>

<section class="tasks">
  <div class="row head">
    <h1>Tâches</h1>
    <select bind:value={filter} aria-label="Projet">
      <option value="">Tous les projets</option>
      <option value="-">Sans projet</option>
      {#each data?.projects ?? [] as p}<option value={p.id}>{p.name}</option>{/each}
    </select>
    {#if filter && filter !== '-'}
      <button class="link" onclick={removeProject}>Supprimer ce projet</button>
    {/if}
    <form class="row" onsubmit={addProject}>
      <input placeholder="Nouveau projet" bind:value={newProject} maxlength="80" />
      <button type="submit" disabled={!newProject.trim()}>Créer</button>
    </form>
  </div>
  {#if error}<p class="error">{error}</p>{/if}

  <form class="row add" onsubmit={add}>
    <input class="grow" placeholder="Nouvelle tâche (Entrée pour ajouter)" bind:value={newTitle} maxlength="200" />
    <button type="submit" disabled={!newTitle.trim()}>Ajouter</button>
  </form>

  {#if data}
    <div class="board">
      {#each columns as c}
        {@const list = inColumn(c.id)}
        <div
          class="column"
          class:target={over === c.id}
          role="list"
          ondragover={(e) => {
            e.preventDefault();
            over = c.id;
          }}
          ondragleave={() => (over = null)}
          ondrop={(e) => {
            e.preventDefault();
            drop(c.id, null);
          }}
        >
          <h2>{c.label} <span class="muted">{list.length}</span></h2>
          {#each list as t (t.id)}
            <div
              class="task"
              class:late={t.due && t.status !== 'done' && t.due < today()}
              role="listitem"
              draggable="true"
              ondragstart={(e) => {
                dragged = t.id;
                e.dataTransfer?.setData('text/plain', t.id);
              }}
              ondragend={() => {
                dragged = null;
                over = null;
              }}
              ondragover={(e) => e.preventDefault()}
              ondrop={(e) => {
                e.preventDefault();
                e.stopPropagation();
                drop(c.id, t);
              }}
            >
              <button class="title" onclick={() => open(t)}>{t.title}</button>
              <div class="meta">
                {#if t.project_id && !filter}<span class="badge">{projectName(t.project_id)}</span>{/if}
                {#if t.due}<span class:error={t.status !== 'done' && t.due < today()}>{t.due}</span>{/if}
                {#if t.notes}<span class="muted" title={t.notes}>notes</span>{/if}
                {#if who(t.created_by)}<span class="muted" title={t.created_by}>{who(t.created_by)}</span>{/if}
                <span class="grow"></span>
                {#each columns.filter((x) => x.id !== c.id) as x}
                  <button class="link small" onclick={() => move(t, x.id)} title="Passer en « {x.label} »">{x.label}</button>
                {/each}
              </div>
            </div>
          {:else}
            <p class="muted small">Rien ici.</p>
          {/each}
        </div>
      {/each}
    </div>
  {:else if !error}
    <p class="muted">Chargement…</p>
  {/if}
</section>

{#if editing}
  <div class="overlay" role="presentation" onclick={(e) => e.target === e.currentTarget && (editing = null)}>
    <form class="card narrow" onsubmit={save}>
      <h2>Tâche</h2>
      <label>Titre <input bind:value={draft.title} maxlength="200" required /></label>
      <label>Notes <textarea rows="5" bind:value={draft.notes}></textarea></label>
      <div class="row">
        <label>Échéance <input type="date" bind:value={draft.due} /></label>
        <label class="grow">Projet
          <select bind:value={draft.project_id}>
            <option value="">Aucun</option>
            {#each data?.projects ?? [] as p}<option value={p.id}>{p.name}</option>{/each}
          </select>
        </label>
      </div>
      <p class="muted small">
        Créée le {new Date(editing.created_at * 1000).toLocaleString('fr-FR')}
        {#if editing.created_by?.startsWith('session:')}
          par une <a href="#sessions/{editing.created_by.slice(8)}">session</a>{/if}.
      </p>
      <div class="row">
        <button type="submit">Enregistrer</button>
        <button type="button" class="link" onclick={() => (editing = null)}>Annuler</button>
        <span class="grow"></span>
        <button type="button" class="link error" onclick={remove}>Supprimer</button>
      </div>
    </form>
  </div>
{/if}

<style>
  .head { align-items: center; flex-wrap: wrap; gap: var(--s3); }
  .head h1 { margin: 0; margin-right: var(--s3); }
  .add { margin: var(--s4) 0; }
  .board { display: grid; grid-template-columns: repeat(3, minmax(220px, 1fr)); gap: var(--s4); align-items: start; }
  .column { background: var(--panel); border: 1px solid var(--line); border-radius: var(--radius); padding: var(--s3); min-height: 120px; }
  .column.target { border-color: var(--accent); }
  .column h2 { font-size: 15px; }
  .task { border: 1px solid var(--line); border-radius: var(--radius); padding: var(--s2) var(--s3); margin-bottom: var(--s2); background: var(--bg); cursor: grab; }
  .task.late { border-left: 3px solid var(--error); }
  .title { all: unset; cursor: pointer; display: block; overflow-wrap: anywhere; }
  .title:hover { color: var(--accent); }
  .meta { display: flex; gap: var(--s2); align-items: center; font-size: 12px; margin-top: var(--s1); flex-wrap: wrap; }
  .overlay { position: fixed; inset: 0; background: rgb(0 0 0 / 0.5); display: grid; place-items: center; z-index: 10; }
  .overlay form { width: min(520px, 92vw); }
  textarea { width: 100%; }
  @media (max-width: 800px) { .board { grid-template-columns: 1fr; } }
</style>
