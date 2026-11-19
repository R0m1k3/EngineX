<script lang="ts">
  import { fileUrl, type ItemData } from '../../lib/api';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import SquareTerminal from '@lucide/svelte/icons/square-terminal';
  import FilePen from '@lucide/svelte/icons/file-pen';
  import Plug from '@lucide/svelte/icons/plug';
  import Lightbulb from '@lucide/svelte/icons/lightbulb';
  import Check from '@lucide/svelte/icons/check';
  import X from '@lucide/svelte/icons/x';
  import Ban from '@lucide/svelte/icons/ban';
  import LoaderCircle from '@lucide/svelte/icons/loader-circle';
  import ImageIcon from '@lucide/svelte/icons/image';
  import ListChecks from '@lucide/svelte/icons/list-checks';
  import Info from '@lucide/svelte/icons/info';
  import Code from '@lucide/svelte/icons/code';
  import Icon from '../../lib/ui/Icon.svelte';
  import Markdown from './Markdown.svelte';

  let { item, sessionId }: { item: ItemData; sessionId: string } = $props();

  const joined = (v: unknown) => (Array.isArray(v) ? v.join('\n\n') : typeof v === 'string' ? v : '');
  let reasoning = $derived(
    item.type === 'reasoning' ? (joined(item.summary) || joined(item.content) || (item._live ?? '')) : '',
  );
  const words = (t: string) => (t.trim() ? t.trim().split(/\s+/).length : 0);
  const duration = (ms: number | null | undefined) =>
    ms == null ? '' : ms < 1000 ? `${ms} ms` : `${(ms / 1000).toLocaleString('fr-FR', { maximumFractionDigits: 1 })} s`;
  const kindLabel = (k: any) =>
    k?.type === 'add' ? 'ajouté' : k?.type === 'delete' ? 'supprimé' : k?.move_path ? `déplacé vers ${k.move_path}` : 'modifié';
  const running = (s: unknown) => s === 'inProgress';
  // Texte de la réponse d'un outil MCP (`content[].text`).
  const toolText = (it: ItemData) =>
    Array.isArray(it.result?.content)
      ? it.result.content.map((c: { text?: string }) => c.text ?? '').join('\n').trim()
      : '';
  // Lignes d'un diff colorées par leur premier caractère.
  const diffLines = (d: string) =>
    d.split('\n').map((l) => ({
      l,
      c: l.startsWith('+++') || l.startsWith('---') || l.startsWith('@@') ? 'h' : l[0] === '+' ? 'add' : l[0] === '-' ? 'del' : '',
    }));
  const failedCmd = $derived(item.type === 'commandExecution' && item.exitCode != null && item.exitCode !== 0);
  const mcpFailed = $derived(item.type === 'mcpToolCall' && (!!item.error || !!item.result?.isError));
</script>

{#snippet chev()}<Icon icon={ChevronRight} size={16} class="chev" />{/snippet}

{#if item.type === 'userMessage'}
  <div class="user">
    {#if item.text}<div class="text">{item.text}</div>{/if}
    {#if item.note}<p class="note" title={item.note}>Note d’EngineX jointe : fichiers remis à un état antérieur.</p>{/if}
    {#if item.images?.length}
      <div class="thumbs">
        {#each item.images as img (img.name)}
          <a href={img.url} target="_blank" rel="noopener"><img src={img.url} alt="Pièce jointe" /></a>
        {/each}
      </div>
    {/if}
  </div>
{:else if item.type === 'agentMessage'}
  <div class="agent"><Markdown text={item.text ?? ''} /></div>
{:else if item.type === 'reasoning'}
  {#if reasoning}
    <details class="fold reason">
      <summary>{@render chev()}<Icon icon={Lightbulb} size={16} />Réflexion <span class="faint num">{words(reasoning).toLocaleString('fr-FR')} mots</span></summary>
      <div class="reason-body">{reasoning}</div>
    </details>
  {/if}
{:else if item.type === 'commandExecution'}
  <details class="fold tool toolrow" open={running(item.status)}>
    <summary>
      {@render chev()}<Icon icon={SquareTerminal} size={16} />
      <code class="cmd" title={item.command}>{item.command}</code>
      <span class="meta num">
        {#if running(item.status)}
          <Icon icon={LoaderCircle} size={16} class="spin" label="En cours" />
        {:else if item.status === 'declined'}
          <span class="bad"><Icon icon={Ban} size={16} />refusée</span>
        {:else}
          <span class={failedCmd ? 'bad' : 'good'}>
            <Icon icon={failedCmd ? X : Check} size={16} label={failedCmd ? 'Échec' : 'Réussi'} />code {item.exitCode ?? '?'}
          </span>
          {#if duration(item.durationMs)}<span>{duration(item.durationMs)}</span>{/if}
        {/if}
      </span>
    </summary>
    {#if item.aggregatedOutput}<pre class="out">{item.aggregatedOutput}</pre>{:else}<p class="none">Aucune sortie.</p>{/if}
  </details>
{:else if item.type === 'fileChange'}
  {@const changes = item.changes ?? []}
  <details class="fold tool toolrow">
    <summary>
      {@render chev()}<Icon icon={FilePen} size={16} />
      <code class="cmd">{changes.map((c: any) => c.path).join(', ') || '…'}</code>
      <span class="meta">
        {#if item.status === 'declined'}<span class="bad"><Icon icon={Ban} size={16} />refusé</span>
        {:else if item.status === 'failed'}<span class="bad"><Icon icon={X} size={16} />échec</span>
        {:else if running(item.status)}<Icon icon={LoaderCircle} size={16} class="spin" label="En cours" />
        {:else}<span>{changes.length > 1 ? `${changes.length} fichiers` : kindLabel(changes[0]?.kind)}</span>{/if}
      </span>
    </summary>
    {#each changes as c (c.path)}
      <div class="fh"><code class="ell">{c.path}</code><span class="faint">{kindLabel(c.kind)}</span></div>
      {#if c.diff}<pre class="out diff">{#each diffLines(c.diff) as { l, c: k }}<span class={k}>{l}</span>{'\n'}{/each}</pre>{/if}
    {/each}
  </details>
{:else if item.type === 'mcpToolCall'}
  <details class="fold tool toolrow">
    <summary>
      {@render chev()}<Icon icon={Plug} size={16} />
      <span class="who">{item.server === 'enginex' ? 'EngineX' : 'Outil'}</span>
      <code class="cmd">{item.server === 'enginex' ? item.tool : `${item.server}.${item.tool}`}</code>
      <span class="meta">
        {#if running(item.status)}<Icon icon={LoaderCircle} size={16} class="spin" label="En cours" />
        {:else if mcpFailed}<span class="bad"><Icon icon={X} size={16} />échec</span>
        {:else if item.result}<span class="good"><Icon icon={Check} size={16} label="Réussi" /></span>{/if}
        {#if duration(item.durationMs)}<span class="num">{duration(item.durationMs)}</span>{/if}
      </span>
    </summary>
    <pre class="out">{JSON.stringify(item.arguments, null, 2)}</pre>
    {#if toolText(item)}<pre class="out">{toolText(item)}</pre>
    {:else if item.result || item.error}<pre class="out">{JSON.stringify({ result: item.result, error: item.error }, null, 2)}</pre>{/if}
  </details>
{:else if item.type === 'imageView' && item.path}
  <figure class="seen">
    <figcaption><Icon icon={ImageIcon} size={16} />Image vue <code class="ell">{item.path}</code></figcaption>
    <img src={fileUrl(sessionId, item.path)} alt={item.path} />
  </figure>
{:else if item.type === 'plan'}
  <div class="plan">
    <div class="ph"><Icon icon={ListChecks} size={16} />Plan</div>
    <Markdown text={item.text ?? ''} />
  </div>
{:else if item.type === 'notice'}
  <p class="notice"><Icon icon={Info} size={16} /><span>{item.text}</span></p>
{:else if item.type === 'contextCompaction'}
  <p class="compact"><span>Contexte résumé pour rester dans la fenêtre du modèle</span></p>
{:else}
  <details class="fold tool toolrow">
    <summary>{@render chev()}<Icon icon={Code} size={16} /><span class="cmd">{item.type ?? 'élément'}</span></summary>
    <pre class="out">{JSON.stringify(item, null, 2)}</pre>
  </details>
{/if}

<style>
  .user {
    align-self: flex-end;
    max-width: 85%;
    background: var(--raised);
    border: 1px solid var(--line);
    border-radius: var(--r);
    padding: var(--s3) var(--s4);
  }
  .text { white-space: pre-wrap; overflow-wrap: anywhere; }
  .note { margin: var(--s2) 0 0; font-size: var(--fs-sm); color: var(--muted); }
  .thumbs { display: flex; gap: var(--s2); flex-wrap: wrap; margin-top: var(--s3); }
  .thumbs a { display: block; border: 1px solid var(--line-strong); border-radius: var(--r); overflow: hidden; background: var(--bg); }
  .thumbs img { max-height: 120px; max-width: 200px; display: block; }
  .agent { max-width: 100%; }

  /* Repliables : réflexion, appels d'outils */
  .fold > summary {
    list-style: none;
    display: flex;
    align-items: center;
    gap: var(--s2);
    min-height: 30px;
    cursor: pointer;
    font-size: var(--fs-sm);
    color: var(--muted);
  }
  .fold > summary::-webkit-details-marker { display: none; }
  .fold > summary:hover { color: var(--text); }
  .fold :global(.chev) { color: var(--faint); transition: transform 0.12s; flex: none; }
  .fold[open] > summary :global(.chev) { transform: rotate(90deg); }
  .fold > summary > :global(.ui-icon) { flex: none; }
  .reason { border-left: 2px solid var(--line-strong); padding-left: var(--s3); }
  .reason-body { font-size: var(--fs-sm); color: var(--muted); white-space: pre-wrap; padding: var(--s1) 0 var(--s2) var(--s6); max-width: 68ch; max-height: 420px; overflow: auto; }

  .tool { border: 1px solid var(--line); border-radius: var(--r); overflow: hidden; }
  .tool > summary { padding: 0 var(--s3); min-height: 40px; background: var(--panel); }
  .cmd { flex: 1; min-width: 0; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tool[open] .cmd { white-space: pre-wrap; word-break: break-all; padding: var(--s2) 0; }
  .who { color: var(--muted); flex: none; }
  .meta { display: inline-flex; align-items: center; gap: var(--s3); color: var(--muted); white-space: nowrap; flex: none; }
  .meta > span { display: inline-flex; align-items: center; gap: 4px; }
  .good { color: var(--accent); }
  .bad { color: var(--danger); }
  .meta :global(.spin) { color: var(--accent); animation: spin 1s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .out {
    margin: 0;
    max-height: 360px;
    overflow: auto;
    padding: var(--s3) var(--s4);
    background: var(--bg);
    border-top: 1px solid var(--line);
    color: var(--muted);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .none { margin: 0; padding: var(--s2) var(--s4); border-top: 1px solid var(--line); font-size: var(--fs-sm); color: var(--faint); }
  .fh { display: flex; align-items: center; gap: var(--s2); padding: 0 var(--s3); min-height: 34px; border-top: 1px solid var(--line); font-size: var(--fs-sm); background: var(--panel); }
  .fh code { flex: 1; }
  .diff { white-space: pre; overflow-wrap: normal; padding: var(--s2) 0; color: var(--text); }
  .diff span { display: inline-block; min-width: 100%; padding: 0 var(--s4); }
  .diff .add { background: var(--add-bg); color: var(--add-fg); }
  .diff .del { background: var(--del-bg); color: var(--del-fg); }
  .diff .h { color: var(--faint); }

  .seen { margin: 0; display: flex; flex-direction: column; gap: var(--s2); }
  figcaption { display: flex; align-items: center; gap: var(--s2); font-size: var(--fs-sm); color: var(--muted); }
  .seen img { max-width: 100%; max-height: 400px; border: 1px solid var(--line); border-radius: var(--r); align-self: flex-start; }
  .plan { border: 1px solid var(--line); border-radius: var(--r); padding: var(--s3) var(--s4); background: var(--panel); }
  .ph { display: flex; align-items: center; gap: var(--s2); font-size: var(--fs-sm); color: var(--muted); margin-bottom: var(--s2); }
  .notice { display: flex; gap: var(--s2); align-items: flex-start; margin: 0; padding: var(--s2) var(--s3); border: 1px solid var(--line-strong); border-radius: var(--r); background: var(--panel); font-size: var(--fs-sm); white-space: pre-wrap; }
  .notice :global(.ui-icon) { color: var(--faint); flex: none; margin-top: 2px; }
  .compact { display: flex; align-items: center; gap: var(--s3); margin: 0; font-size: var(--fs-xs); color: var(--faint); }
  .compact::before, .compact::after { content: ''; flex: 1; border-top: 1px dashed var(--line-strong); }
  @media (prefers-reduced-motion: reduce) {
    .meta :global(.spin) { animation: none; }
  }
</style>
