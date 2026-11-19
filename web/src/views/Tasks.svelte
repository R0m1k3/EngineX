<script lang="ts">
  import { onDestroy } from 'svelte';
  import {
    board, createTask, patchTask, deleteTask, createTaskProject, deleteTaskProject, message,
    type Board, type Task, type TaskStatus,
  } from '../lib/api';
  import { Button, Input, Select, Textarea, Dialog, Tabs, EmptyState, Skeleton, confirmDialog, toast, tooltip } from '../lib/ui';
  import Page from './ops/Page.svelte';
  import Plus from '@lucide/svelte/icons/plus';
  import FolderPlus from '@lucide/svelte/icons/folder-plus';
  import Trash2 from '@lucide/svelte/icons/trash-2';
  import CalendarDays from '@lucide/svelte/icons/calendar-days';
  import TextIcon from '@lucide/svelte/icons/text';
  import Bot from '@lucide/svelte/icons/bot';
  import SquareCheck from '@lucide/svelte/icons/square-check';

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
  let projectOpen = $state(false);
  let editing = $state<Task | null>(null);
  let editOpen = $state(false);
  let draft = $state({ title: '', notes: '', due: '', project_id: '' });
  let dragged = $state<string | null>(null);
  let over = $state<string | null>(null);
  /** Colonne montrée au téléphone. */
  let phoneCol = $state<string>('todo');

  const today = () => new Date().toISOString().slice(0, 10);
  const projectName = (id: string | null) => data?.projects.find((p) => p.id === id)?.name ?? '';
  const who = (b?: string) => (b?.startsWith('session:') ? 'IA' : '');
  const dueText = (d: string) => {
    if (d === today()) return 'aujourd’hui';
    return new Date(`${d}T12:00`).toLocaleDateString('fr-FR', { day: 'numeric', month: 'short' });
  };

  let visible = $derived(
    (data?.tasks ?? []).filter((t) => !filter || (filter === '-' ? !t.project_id : t.project_id === filter)),
  );
  const inColumn = (s: TaskStatus) => visible.filter((t) => t.status === s).sort((a, b) => a.order - b.order);
  const tabItems = $derived(columns.map((c) => ({ id: c.id, label: c.label, count: inColumn(c.id).length })));

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
      toast(error, 'error');
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
    projectOpen = false;
    act(async () => {
      const p = await createTaskProject(name);
      filter = p.id;
      toast(`Projet « ${name} » créé`, 'ok');
    });
  }

  async function removeProject() {
    const name = projectName(filter);
    const id = filter;
    if (!(await confirmDialog(`Supprimer le projet « ${name} » ? Ses tâches restent, sans projet.`, { title: 'Supprimer le projet', confirm: 'Supprimer', danger: true }))) return;
    act(() => deleteTaskProject(id));
    filter = '';
  }

  function open(t: Task) {
    editing = t;
    draft = { title: t.title, notes: t.notes, due: t.due ?? '', project_id: t.project_id ?? '' };
    editOpen = true;
  }
  function closeEdit() {
    editOpen = false;
    editing = null;
  }

  function save(e: SubmitEvent) {
    e.preventDefault();
    const t = editing;
    if (!t) return;
    closeEdit();
    act(() =>
      patchTask(t.id, {
        title: draft.title,
        notes: draft.notes,
        due: draft.due || null,
        project_id: draft.project_id || null,
      }),
    );
  }

  async function remove() {
    const t = editing;
    if (!t || !(await confirmDialog(`Supprimer « ${t.title} » ?`, { title: 'Supprimer la tâche', confirm: 'Supprimer', danger: true }))) return;
    closeEdit();
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

  const filterOptions = $derived([
    { value: '', label: 'Tous les projets' },
    { value: '-', label: 'Sans projet' },
    ...(data?.projects ?? []).map((p) => ({ value: p.id, label: p.name })),
  ]);
  const total = $derived(visible.filter((t) => t.status !== 'done').length);
</script>

<Page title="Tâches">
  {#snippet meta()}{#if data}<span class="num">{total} ouverte{total > 1 ? 's' : ''}</span>{/if}{/snippet}
  {#snippet actions()}
    <Select bind:value={filter} options={filterOptions} aria-label="Projet" />
    {#if filter && filter !== '-'}
      <Button variant="ghost" icon={Trash2} label="Supprimer ce projet" onclick={removeProject} />
    {/if}
    <Button variant="ghost" icon={FolderPlus} onclick={() => (projectOpen = true)}>Nouveau projet</Button>
  {/snippet}

  <form class="add" onsubmit={add}>
    <Input class="grow-in" icon={Plus} placeholder="Nouvelle tâche, Entrée pour l’ajouter{filter && filter !== '-' ? ` à ${projectName(filter)}` : ''}" bind:value={newTitle} maxlength={200} aria-label="Nouvelle tâche" />
    <Button type="submit" variant="primary" disabled={!newTitle.trim()}>Ajouter</Button>
  </form>
  {#if error}<p class="err-line" role="alert">{error}</p>{/if}

  {#if data}
    <div class="phone-tabs"><Tabs items={tabItems} bind:active={phoneCol} label="Colonnes" /></div>
    <div class="board">
      {#each columns as c}
        {@const list = inColumn(c.id)}
        <div
          class="column"
          class:target={over === c.id}
          class:shown={phoneCol === c.id}
          role="list"
          aria-label={c.label}
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
          <h2 class="col-h"><span>{c.label}</span><span class="n num">{list.length}</span></h2>
          <div class="cards">
            {#each list as t (t.id)}
              {@const late = !!t.due && t.status !== 'done' && t.due < today()}
              <div
                class="task"
                class:late
                class:done={t.status === 'done'}
                class:dragging={dragged === t.id}
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
                {#if (t.project_id && !filter) || t.due || t.notes || who(t.created_by)}
                  <div class="meta">
                    {#if t.project_id && !filter}<span class="tag">{projectName(t.project_id)}</span>{/if}
                    {#if t.due}<span class="due" class:late><CalendarDays size={14} />{late ? `en retard, ${dueText(t.due)}` : dueText(t.due)}</span>{/if}
                    {#if t.notes}<span class="ic" use:tooltip={t.notes.length > 200 ? `${t.notes.slice(0, 200)}…` : t.notes} aria-label="Notes"><TextIcon size={14} /></span>{/if}
                    {#if who(t.created_by)}<span class="ic" use:tooltip={'Ajoutée par une session'} aria-label="Ajoutée par l’IA"><Bot size={14} /></span>{/if}
                  </div>
                {/if}
                <div class="moves">
                  {#each columns.filter((x) => x.id !== c.id) as x}
                    <button class="mv" onclick={() => move(t, x.id)} use:tooltip={`Passer en « ${x.label} »`}>{x.label}</button>
                  {/each}
                </div>
              </div>
            {:else}
              {#if c.id === 'todo' && !data.tasks.length}
                <EmptyState icon={SquareCheck} title="Aucune tâche" text="Ajoute une tâche ci-dessus. Les sessions peuvent aussi en créer avec les outils tasks_*." />
              {:else}
              <p class="none">{c.id === 'todo' ? 'Rien à faire.' : c.id === 'doing' ? 'Rien en cours.' : 'Rien de fini.'}</p>
              {/if}
            {/each}
          </div>
        </div>
      {/each}
    </div>
  {:else if !error}
    <div class="pad"><Skeleton lines={4} /></div>
  {/if}
</Page>

<Dialog bind:open={editOpen} title="Tâche" width="560px" onclose={() => (editing = null)}>
  {#if editing}
    <form id="task-form" class="form" onsubmit={save}>
      <Input label="Titre" bind:value={draft.title} maxlength={200} required />
      <Textarea label="Notes" rows={5} bind:value={draft.notes} />
      <div class="two">
        <Input label="Échéance" type="date" bind:value={draft.due} />
        <Select label="Projet" bind:value={draft.project_id} options={[{ value: '', label: 'Aucun' }, ...(data?.projects ?? []).map((p) => ({ value: p.id, label: p.name }))]} />
      </div>
      <p class="created">
        Créée le {new Date(editing.created_at * 1000).toLocaleString('fr-FR', { dateStyle: 'long', timeStyle: 'short' })}{#if editing.created_by?.startsWith('session:')}
          par une <a href="#sessions/{editing.created_by.slice(8)}">session</a>{/if}.
      </p>
    </form>
  {/if}
  {#snippet actions()}
    <Button variant="danger" icon={Trash2} onclick={remove}>Supprimer</Button>
    <span class="spacer"></span>
    <Button onclick={closeEdit}>Annuler</Button>
    <Button variant="primary" type="submit" form="task-form">Enregistrer</Button>
  {/snippet}
</Dialog>

<Dialog bind:open={projectOpen} title="Nouveau projet" width="420px">
  <form id="proj-form" class="form" onsubmit={addProject}>
    <Input label="Nom" hint="Les tâches peuvent être rangées par projet et filtrées." bind:value={newProject} maxlength={80} required />
  </form>
  {#snippet actions()}
    <Button onclick={() => (projectOpen = false)}>Annuler</Button>
    <Button variant="primary" type="submit" form="proj-form" disabled={!newProject.trim()}>Créer le projet</Button>
  {/snippet}
</Dialog>

<style>
  .add { display: flex; gap: var(--s2); align-items: flex-end; padding: var(--s4) var(--s5); border-bottom: 1px solid var(--line); }
  .add > :global(:first-child) { flex: 1; min-width: 0; max-width: 720px; }
  .err-line { margin: 0; padding: var(--s2) var(--s5); background: var(--danger-soft); color: var(--danger); font-size: var(--fs-sm); border-bottom: 1px solid var(--line); }
  .pad { padding: var(--s5); }

  .phone-tabs { display: none; }
  .board { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 1px; background: var(--line); flex: 1; min-height: calc(100dvh - var(--status-h) - 128px); border-bottom: 1px solid var(--line); }
  .column { background: var(--panel); display: flex; flex-direction: column; min-height: 320px; min-width: 0; box-shadow: inset 0 0 0 2px transparent; }
  .column.target { box-shadow: inset 0 0 0 2px var(--accent); background: var(--accent-soft); }
  .col-h { display: flex; align-items: center; gap: var(--s2); margin: 0; height: 44px; padding: 0 var(--s4); font-size: var(--fs-md); font-weight: 600; }
  .col-h .n { color: var(--muted); font-weight: 400; font-size: var(--fs-sm); }
  .cards { display: flex; flex-direction: column; gap: var(--s2); padding: 0 var(--s3) var(--s4); }
  .none { margin: 0; padding: var(--s2) var(--s1); color: var(--faint); font-size: var(--fs-sm); }

  .task { position: relative; background: var(--bg); border: 1px solid var(--line); border-radius: var(--r); padding: var(--s3); cursor: grab; display: flex; flex-direction: column; gap: var(--s2); }
  .task:hover { border-color: var(--line-strong); }
  .task.late { border-left: 2px solid var(--danger); }
  .task.dragging { opacity: 0.5; }
  .title { all: unset; cursor: pointer; display: block; overflow-wrap: anywhere; font-size: var(--fs-md); line-height: 1.4; color: var(--text); }
  .title:hover { color: var(--accent); }
  .title:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; border-radius: var(--r-sm); }
  .done .title { color: var(--muted); text-decoration: line-through; text-decoration-color: var(--faint); }
  .meta { display: flex; gap: var(--s2); align-items: center; font-size: var(--fs-sm); color: var(--muted); flex-wrap: wrap; }
  .tag { padding: 0 var(--s2); height: 22px; display: inline-flex; align-items: center; border-radius: var(--r); background: var(--raised); color: var(--muted); font-size: var(--fs-xs); }
  .due { display: inline-flex; align-items: center; gap: 4px; font-variant-numeric: tabular-nums; }
  .due.late { color: var(--danger); }
  .ic { display: inline-flex; color: var(--faint); }
  .moves { display: flex; gap: var(--s1); flex-wrap: wrap; }
  .mv { all: unset; cursor: pointer; font-size: var(--fs-sm); color: var(--muted); padding: 2px var(--s2); border: 1px solid var(--line); border-radius: var(--r); }
  .mv:hover { color: var(--text); border-color: var(--line-strong); background: var(--raised); }
  .mv:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
  @media (hover: hover) and (min-width: 768px) {
    .moves { position: absolute; right: var(--s2); bottom: var(--s2); opacity: 0; background: var(--bg); padding-left: var(--s2); }
    .task:hover .moves, .task:focus-within .moves { opacity: 1; }
  }

  .form { display: flex; flex-direction: column; gap: var(--s4); }
  .two { display: grid; grid-template-columns: 180px 1fr; gap: var(--s3); }
  .created { margin: 0; color: var(--muted); font-size: var(--fs-sm); }
  .spacer { flex: 1; }

  @media (max-width: 767px) {
    .add { padding: var(--s3) var(--s4); }
    .add :global(.box) { height: var(--touch); }
    .add :global(.btn) { height: var(--touch); }
    .phone-tabs { display: block; padding: 0 var(--s2); border-bottom: 1px solid var(--line); }
    .phone-tabs :global(.tabs-ui) { border-bottom: 0; }
    .phone-tabs :global([role='tab']) { height: var(--touch); flex: 1; justify-content: center; }
    .board { grid-template-columns: minmax(0, 1fr); background: none; border: 0; min-height: 0; }
    .column { display: none; background: var(--bg); min-height: 0; }
    .column.shown { display: flex; }
    .col-h { display: none; }
    .cards { padding: var(--s3) var(--s4) var(--s4); }
    .task { cursor: default; padding: var(--s3) var(--s4); }
    .task { padding-bottom: 0; }
    .moves { gap: 0; flex-wrap: nowrap; margin: 0 calc(-1 * var(--s4)); border-top: 1px solid var(--line); }
    .mv { border: 0; border-radius: 0; padding: 0 var(--s3); min-height: var(--touch); display: inline-flex; align-items: center; justify-content: center; flex: 1; box-sizing: border-box; }
    .mv + .mv { border-left: 1px solid var(--line); }
    .two { grid-template-columns: 1fr; }
    .spacer { display: none; }
  }
</style>
