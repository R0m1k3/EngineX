<script lang="ts">
  import { skills, skill, saveSkill, enableSkill, message, type Skill } from '../../lib/api';

  let list = $state<Skill[]>([]);
  let editing = $state<{ name: string; content: string; isNew: boolean } | null>(null);
  let error = $state('');
  let info = $state('');

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
    info = '';
    try {
      const s = await skill(name);
      editing = { name, content: s.content, isNew: false };
    } catch (e) {
      error = message(e);
    }
  }

  function create() {
    const n = prompt('Nom de la skill (minuscules, chiffres et tirets)', '');
    if (!n?.trim()) return;
    const name = n.trim();
    editing = { name, content: TEMPLATE(name), isNew: true };
    info = '';
  }

  async function save() {
    if (!editing) return;
    error = '';
    try {
      await saveSkill(editing.name, editing.content);
      info = `Skill ${editing.name} enregistrée : elle vaut pour les sandboxes démarrées ensuite.`;
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
      info = 'Changement pris en compte au prochain démarrage de sandbox.';
      await load();
    } catch (e) {
      error = message(e);
    }
  }

  load();
</script>

{#if error}<p class="error">{error}</p>{/if}

<section class="card">
  <h3>Skills</h3>
  <p class="small muted">
    Une skill est un fichier <code>SKILL.md</code> que Codex lit quand la tâche s’y prête (son nom et sa description lui
    sont annoncés). Elles sont copiées dans chaque sandbox à son démarrage : un changement vaut pour les sandboxes
    démarrées ensuite.
  </p>
  <table>
    <tbody>
      {#each list as s (s.name)}
        <tr>
          <td class="name"><code>{s.name}</code>{#if s.shipped}<br /><span class="muted small">livrée</span>{/if}</td>
          <td class="small">{#if s.valid}{s.description}{:else}<span class="error">SKILL.md invalide : non installée</span>{/if}</td>
          <td class="act">
            <div class="row-act">
              <label class="check small"><input type="checkbox" checked={s.enabled} onchange={() => toggle(s)} /> active</label>
              <button class="link" onclick={() => edit(s.name)}>Modifier</button>
            </div>
          </td>
        </tr>
      {/each}
    </tbody>
  </table>
  <div class="actions"><button onclick={create}>Nouvelle skill</button>{#if info}<span class="ok small">{info}</span>{/if}</div>
</section>

{#if editing}
  <section class="card">
    <h3>{editing.isNew ? 'Nouvelle skill' : 'Skill'} <code>{editing.name}</code></h3>
    <p class="small muted">En tête, le bloc <code>---</code> avec <code>name: {editing.name}</code> et une
      <code>description</code> sur une ligne. 64 Kio au plus.</p>
    <textarea bind:value={editing.content} rows="20" spellcheck="false"></textarea>
    <div class="actions">
      <button onclick={save}>Enregistrer</button>
      <button class="link" onclick={() => (editing = null)}>Annuler</button>
    </div>
  </section>
{/if}

<style>
  textarea { width: 100%; font-family: var(--mono, monospace); font-size: 13px; }
  td.name { white-space: nowrap; vertical-align: top; }
  td.act { white-space: nowrap; vertical-align: top; }
  .row-act { display: flex; gap: var(--s3); align-items: center; justify-content: flex-end; }
  .check { display: inline-flex; flex-direction: row; gap: var(--s1); align-items: center; }
</style>
