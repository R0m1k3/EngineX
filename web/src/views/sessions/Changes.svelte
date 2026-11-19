<script lang="ts">
  import { turnDiff, undoFile, message, ApiError, type Changes } from '../../lib/api';

  let { sessionId, turnId, changes, canUndo }: {
    sessionId: string;
    turnId: string;
    changes: Changes;
    canUndo: boolean;
  } = $props();

  let open = $state<Record<string, { diff: string; truncated: boolean; binary: boolean } | 'loading' | string>>({});
  let error = $state('');
  let done = $state<Record<string, boolean>>({});

  const STATUS: Record<string, string> = { A: 'ajouté', M: 'modifié', D: 'supprimé', T: 'type changé' };

  async function toggle(path: string) {
    if (open[path]) {
      delete open[path];
      return;
    }
    open[path] = 'loading';
    try {
      open[path] = await turnDiff(sessionId, turnId, path);
    } catch (e) {
      open[path] = message(e);
    }
  }

  async function undo(path: string) {
    error = '';
    try {
      await undoFile(sessionId, turnId, path);
      done[path] = true;
    } catch (e) {
      if (e instanceof ApiError && e.code === 'changed_since') {
        if (!confirm(`${path} a changé depuis ce tour. Annuler quand même ?`)) return;
        try {
          await undoFile(sessionId, turnId, path, true);
          done[path] = true;
        } catch (e2) {
          error = message(e2);
        }
      } else {
        error = message(e);
      }
    }
  }

  // Lignes du diff colorées par leur premier caractère.
  const lines = (d: string) =>
    d.split('\n').map((l) => ({ l, c: l.startsWith('+++') || l.startsWith('---') ? 'h' : l[0] === '+' ? 'add' : l[0] === '-' ? 'del' : l.startsWith('@@') ? 'h' : '' }));
</script>

<details class="changes">
  <summary class="small">
    Modifications : {changes.files.length}{changes.truncated ? '+' : ''} fichier{changes.files.length > 1 ? 's' : ''}
  </summary>
  {#each changes.files as f (f.path)}
    {@const d = open[f.path]}
    <div class="file">
      <div class="line small">
        <button class="link path" onclick={() => toggle(f.path)} title="Voir le diff"><code>{f.path}</code></button>
        <span class="muted">{STATUS[f.status] ?? f.status}</span>
        {#if f.binary}<span class="muted">binaire</span>
        {:else}<span class="ok">+{f.added}</span><span class="error">−{f.deleted}</span>{/if}
        <span class="grow"></span>
        {#if done[f.path]}<span class="muted">annulé</span>
        {:else if canUndo}<button class="link" onclick={() => undo(f.path)}>Annuler ce fichier</button>{/if}
      </div>
      {#if d === 'loading'}<p class="muted small">Chargement…</p>
      {:else if typeof d === 'string'}<p class="error small">{d}</p>
      {:else if d}
        {#if d.binary}<p class="muted small">Fichier binaire : pas de diff.</p>
        {:else}
          <pre class="diff">{#each lines(d.diff) as { l, c }}<span class={c}>{l}</span>{'\n'}{/each}</pre>
          {#if d.truncated}<p class="muted small">Diff tronqué (512 Kio).</p>{/if}
        {/if}
      {/if}
    </div>
  {/each}
  {#if changes.truncated}<p class="muted small">Liste limitée à 500 fichiers.</p>{/if}
  {#if error}<p class="error small">{error}</p>{/if}
</details>

<style>
  .changes { border-left: 2px solid var(--accent); padding-left: var(--s3); }
  summary { cursor: pointer; }
  .file { margin-top: var(--s1); }
  .line { display: flex; gap: var(--s2); align-items: baseline; flex-wrap: wrap; }
  .path { padding: 0; }
  .diff {
    max-height: 420px; overflow: auto; background: var(--bg); border: 1px solid var(--line);
    border-radius: var(--radius); padding: var(--s2); font-size: var(--fs-sm); margin: var(--s1) 0;
  }
  .add { color: var(--accent); }
  .del { color: var(--error); }
  .h { color: var(--muted); }
</style>
