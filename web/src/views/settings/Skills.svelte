<script lang="ts">
  import Plus from '@lucide/svelte/icons/plus';
  import Pencil from '@lucide/svelte/icons/pencil';
  import Button from '../../lib/ui/Button.svelte';
  import Input from '../../lib/ui/Input.svelte';
  import Switch from '../../lib/ui/Switch.svelte';
  import Badge from '../../lib/ui/Badge.svelte';
  import Dialog from '../../lib/ui/Dialog.svelte';
  import { toast } from '../../lib/ui/toast.svelte';
  import Section from './Section.svelte';
  import Note from './Note.svelte';
  import { skills, skill, saveSkill, enableSkill, message, type Skill } from '../../lib/api';

  let list = $state<Skill[]>([]);
  let editing = $state<{ name: string; content: string; isNew: boolean } | null>(null);
  let error = $state('');

  const TEMPLATE = (n: string) => `---\nname: ${n}\ndescription: Quand et pourquoi utiliser cette skill.\n---\n\n# ${n}\n\n`;

  async function load() {
    try {
      list = await skills();
    } catch (e) {
      error = message(e);
    }
  }

  async function edit(name: string) {
    error = '';
    try {
      const s = await skill(name);
      editing = { name, content: s.content, isNew: false };
    } catch (e) {
      error = message(e);
    }
  }

  // Nom demandé dans une modale (remplace prompt()).
  let askOpen = $state(false);
  let newName = $state('');
  function create() {
    newName = '';
    askOpen = true;
  }
  function submitNew(e: SubmitEvent) {
    e.preventDefault();
    const name = newName.trim();
    askOpen = false;
    if (!name) return;
    editing = { name, content: TEMPLATE(name), isNew: true };
  }

  async function save() {
    if (!editing) return;
    error = '';
    try {
      await saveSkill(editing.name, editing.content);
      toast(`Skill ${editing.name} enregistrée : elle vaut pour les sandboxes démarrées ensuite.`, 'ok');
      editing = null;
      await load();
    } catch (e) {
      error = message(e);
    }
  }

  async function toggle(s: Skill) {
    error = '';
    try {
      await enableSkill(s.name, !s.enabled);
      toast('Changement pris en compte au prochain démarrage de sandbox.', 'ok');
      await load();
    } catch (e) {
      error = message(e);
    }
  }

  load();
</script>

{#if error}<div class="err"><Note tone="danger">{error}</Note></div>{/if}

<Section title="Skills" text="Un fichier SKILL.md que Codex lit quand la tâche s’y prête. Copiées dans chaque sandbox à son démarrage.">
  {#snippet aside()}
    <div class="new"><Button variant="primary" icon={Plus} onclick={create}>Nouvelle skill</Button></div>
  {/snippet}
  <p class="hint">Codex reçoit le nom et la description de chaque skill active. Un changement vaut pour les sandboxes démarrées ensuite.</p>
  <ul class="rows" aria-label="Skills">
    {#each list as s (s.name)}
      <li>
        <div class="rmain">
          <span class="rname"><span class="mono">{s.name}</span>{#if s.shipped}<Badge>livrée</Badge>{/if}</span>
          {#if s.valid}<span class="rdesc">{s.description}</span>{:else}<span class="rdesc bad">SKILL.md invalide : non installée</span>{/if}
        </div>
        <div class="ract">
          <Switch checked={s.enabled} onchange={() => toggle(s)} label="Active" />
          <Button variant="ghost" icon={Pencil} label="Modifier {s.name}" onclick={() => edit(s.name)} />
        </div>
      </li>
    {/each}
  </ul>
</Section>

{#if editing}
  <Section title={editing.isNew ? 'Nouvelle skill' : 'Modifier la skill'} text="En tête, le bloc --- avec le nom et une description sur une ligne. 64 Kio au plus.">
    <p class="ename mono">{editing.name}/SKILL.md</p>
    <textarea class="code" bind:value={editing.content} rows="22" spellcheck="false" aria-label="Contenu de SKILL.md"></textarea>
    <div class="acts">
      <Button variant="primary" onclick={save}>Enregistrer</Button>
      <Button variant="ghost" onclick={() => (editing = null)}>Annuler</Button>
    </div>
  </Section>
{/if}

<Dialog bind:open={askOpen} title="Nouvelle skill" width="420px">
  <form id="skill-form" onsubmit={submitNew}>
    <Input label="Nom de la skill" hint="Minuscules, chiffres et tirets" class="mono-in" bind:value={newName} required />
  </form>
  {#snippet actions()}
    <Button onclick={() => (askOpen = false)}>Annuler</Button>
    <Button variant="primary" type="submit" form="skill-form" disabled={!newName.trim()}>Créer</Button>
  {/snippet}
</Dialog>

<style>
  .err { padding: var(--s4) var(--s6) 0; }
  .hint { margin: 0; color: var(--muted); font-size: var(--fs-sm); }
  .new { margin-top: var(--s4); }
  .rows { list-style: none; margin: 0; padding: 0; border-top: 1px solid var(--line); }
  .rows li { display: flex; align-items: center; gap: var(--s4); padding: var(--s3) 0; border-bottom: 1px solid var(--line); }
  .rmain { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: var(--s1); }
  .rname { display: flex; align-items: center; gap: var(--s2); font-weight: 600; }
  .rdesc { color: var(--muted); font-size: var(--fs-sm); line-height: 1.5; }
  .bad { color: var(--danger); }
  .ract { display: flex; align-items: center; gap: var(--s2); flex: none; }
  .ename { margin: 0; color: var(--muted); }
  .code {
    width: 100%;
    box-sizing: border-box;
    padding: var(--s3);
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--line-strong);
    border-radius: var(--r);
    font-family: var(--mono);
    font-size: var(--fs-code);
    line-height: 1.6;
    resize: vertical;
    tab-size: 2;
  }
  .code:focus-visible { outline: 1px solid var(--accent); border-color: var(--accent); }
  .acts { display: flex; gap: var(--s2); }
  :global(#skill-form .mono-in input) { font-family: var(--mono); }
  @media (max-width: 767px) {
    .err { padding: var(--s4) var(--s4) 0; }
    .rows li { flex-direction: column; align-items: stretch; gap: var(--s2); }
    .ract { justify-content: space-between; }
    .ract :global(.btn), .acts :global(.btn), .new :global(.btn) { height: var(--touch); min-width: var(--touch); }
    .ract :global(.sw) { height: var(--touch); }
  }
</style>
