<script lang="ts">
  import { turnDiff, undoFile, message, ApiError, type Changes } from '../../lib/api';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import FileDiff from '@lucide/svelte/icons/file-diff';
  import Undo2 from '@lucide/svelte/icons/undo-2';
  import Icon from '../../lib/ui/Icon.svelte';
  import IconButton from '../../lib/ui/IconButton.svelte';
  import Skeleton from '../../lib/ui/Skeleton.svelte';
  import { confirmDialog } from '../../lib/ui/dialog.svelte';
  import { toast } from '../../lib/ui/toast.svelte';
  import { screen } from './phone.svelte';

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
      toast(`${path} : modification annulée.`, 'ok');
    } catch (e) {
      if (e instanceof ApiError && e.code === 'changed_since') {
        if (
          !(await confirmDialog(`${path} a changé depuis ce tour : ces changements plus récents seront perdus.`, {
            title: 'Annuler quand même ?',
            confirm: 'Annuler le fichier',
            cancel: 'Garder',
            danger: true,
          }))
        )
          return;
        try {
          await undoFile(sessionId, turnId, path, true);
          done[path] = true;
          toast(`${path} : modification annulée.`, 'ok');
        } catch (e2) {
          error = message(e2);
        }
      } else {
        error = message(e);
      }
    }
  }

  // Lignes du diff colorées par leur premier caractère.
  const total = $derived(
    changes.files.reduce((a, f) => ({ add: a.add + (f.added ?? 0), del: a.del + (f.deleted ?? 0) }), { add: 0, del: 0 }),
  );
  const lines = (d: string) =>
    d.split('\n').map((l) => ({ l, c: l.startsWith('+++') || l.startsWith('---') ? 'h' : l[0] === '+' ? 'add' : l[0] === '-' ? 'del' : l.startsWith('@@') ? 'h' : '' }));
</script>

<details class="changes">
  <summary>
    <Icon icon={ChevronRight} size={16} class="chev" /><Icon icon={FileDiff} size={16} />
    <span class="grow">{changes.files.length}{changes.truncated ? '+' : ''} fichier{changes.files.length > 1 ? 's' : ''} modifié{changes.files.length > 1 ? 's' : ''}</span>
    <span class="num plus">+{total.add}</span><span class="num minus">−{total.del}</span>
  </summary>
  <ul>
    {#each changes.files as f (f.path)}
      {@const d = open[f.path]}
      <li class:on={!!d}>
        <div class="line">
          <span class="letter s-{f.status}" title={STATUS[f.status] ?? f.status}>{f.status}</span>
          <button class="path" onclick={() => toggle(f.path)} aria-expanded={!!d} title={f.path}><span>{f.path}</span></button>
          {#if f.binary}<span class="faint">binaire</span>
          {:else}<span class="num plus">+{f.added}</span><span class="num minus">−{f.deleted}</span>{/if}
          {#if done[f.path]}<span class="faint">annulé</span>
          {:else if canUndo}<IconButton icon={Undo2} label="Annuler ce fichier" size={screen.phone ? 'lg' : 'sm'} onclick={() => undo(f.path)} />{/if}
        </div>
        {#if d === 'loading'}<div class="pad"><Skeleton lines={3} /></div>
        {:else if typeof d === 'string'}<p class="error small pad">{d}</p>
        {:else if d}
          {#if d.binary}<p class="faint small pad">Fichier binaire : pas de diff.</p>
          {:else}
            <pre class="diff">{#each lines(d.diff) as { l, c }}<span class={c}>{l}</span>{'\n'}{/each}</pre>
            {#if d.truncated}<p class="faint small pad">Diff tronqué (512 Kio).</p>{/if}
          {/if}
        {/if}
      </li>
    {/each}
  </ul>
  {#if changes.truncated}<p class="faint small pad">Liste limitée à 500 fichiers.</p>{/if}
  {#if error}<p class="error small pad">{error}</p>{/if}
</details>

<style>
  .changes { border: 1px solid var(--line); border-radius: var(--r); overflow: hidden; font-size: var(--fs-sm); }
  summary { list-style: none; display: flex; align-items: center; gap: var(--s2); min-height: 40px; padding: 0 var(--s3); background: var(--panel); color: var(--muted); cursor: pointer; }
  summary::-webkit-details-marker { display: none; }
  summary:hover { color: var(--text); }
  summary :global(.chev) { color: var(--faint); transition: transform 0.12s; flex: none; }
  .changes[open] > summary :global(.chev) { transform: rotate(90deg); }
  ul { list-style: none; margin: 0; padding: var(--s1) 0; border-top: 1px solid var(--line); }
  li.on { background: var(--panel); }
  .line { display: flex; align-items: center; gap: var(--s2); min-height: 32px; padding: 0 var(--s2) 0 var(--s3); }
  .letter { font-family: var(--mono); font-size: var(--fs-count); width: 16px; text-align: center; flex: none; color: var(--muted); }
  .s-A { color: var(--add-fg); }
  .s-M, .s-T { color: var(--warn); }
  .s-D { color: var(--del-fg); }
  .path { justify-content: flex-start; flex: 1; min-width: 0; height: 28px; min-height: 0; padding: 0; border: 0; background: none; color: var(--text); font: inherit; text-align: left; cursor: pointer; filter: none; overflow: hidden; }
  /* Chemin tronqué par la gauche : la fin (nom du fichier) reste visible. */
  .path span { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; direction: rtl; text-align: left; }
  .path:hover span { text-decoration: underline; text-underline-offset: 2px; }
  .plus { color: var(--add-fg); }
  .minus { color: var(--del-fg); }
  .pad { margin: 0; padding: var(--s2) var(--s3); }
  .diff { margin: 0; max-height: 420px; overflow: auto; padding: var(--s1) 0; background: var(--bg); border-top: 1px solid var(--line); border-bottom: 1px solid var(--line); line-height: 1.6; }
  .diff span { display: inline-block; min-width: 100%; padding: 0 var(--s3); }
  .add { background: var(--add-bg); color: var(--add-fg); }
  .del { background: var(--del-bg); color: var(--del-fg); }
  .h { color: var(--faint); }
  @media (max-width: 767px) {
    summary, .line { min-height: var(--touch); }
  }
</style>
