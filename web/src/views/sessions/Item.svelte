<script lang="ts">
  import { fileUrl, type ItemData } from '../../lib/api';
  import Markdown from './Markdown.svelte';

  let { item, sessionId }: { item: ItemData; sessionId: string } = $props();

  const joined = (v: unknown) => (Array.isArray(v) ? v.join('\n\n') : typeof v === 'string' ? v : '');
  let reasoning = $derived(
    item.type === 'reasoning' ? (joined(item.summary) || joined(item.content) || (item._live ?? '')) : '',
  );
  const duration = (ms: number | null | undefined) =>
    ms == null ? '' : ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`;
  const kindLabel = (k: any) =>
    k?.type === 'add' ? 'ajouté' : k?.type === 'delete' ? 'supprimé' : k?.move_path ? `déplacé vers ${k.move_path}` : 'modifié';
  const running = (s: unknown) => s === 'inProgress';
</script>

{#if item.type === 'userMessage'}
  <div class="user">
    {#if item.text}<div class="text">{item.text}</div>{/if}
    {#if item.note}<p class="note small muted" title={item.note}>Note d’EngineX jointe : fichiers remis à un état antérieur.</p>{/if}
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
    <details class="tool">
      <summary class="muted">Raisonnement</summary>
      <div class="pre muted">{reasoning}</div>
    </details>
  {/if}
{:else if item.type === 'commandExecution'}
  <details class="tool" open={running(item.status)}>
    <summary>
      <code class="cmd">{item.command}</code>
      <span class="meta">
        {#if running(item.status)}en cours…{:else}
          <span class:error={item.exitCode !== 0 && item.exitCode != null}>code {item.exitCode ?? '?'}</span>
          {duration(item.durationMs)}
        {/if}
        {#if item.status === 'declined'}<span class="error">refusée</span>{/if}
      </span>
    </summary>
    {#if item.aggregatedOutput}<pre class="out">{item.aggregatedOutput}</pre>{:else}<p class="muted small">Aucune sortie.</p>{/if}
  </details>
{:else if item.type === 'fileChange'}
  <details class="tool">
    <summary>
      Fichiers : {(item.changes ?? []).map((c: any) => c.path).join(', ') || '…'}
      {#if item.status === 'declined'}<span class="error">refusé</span>{:else if item.status === 'failed'}<span class="error">échec</span>{/if}
    </summary>
    {#each item.changes ?? [] as c (c.path)}
      <p class="small"><code>{c.path}</code> <span class="muted">{kindLabel(c.kind)}</span></p>
      {#if c.diff}<pre class="out diff">{c.diff}</pre>{/if}
    {/each}
  </details>
{:else if item.type === 'mcpToolCall'}
  <details class="tool">
    <summary>Outil <code>{item.server}.{item.tool}</code> {#if item.error}<span class="error">échec</span>{/if}</summary>
    <pre class="out">{JSON.stringify({ arguments: item.arguments, result: item.result, error: item.error }, null, 2)}</pre>
  </details>
{:else if item.type === 'imageView' && item.path}
  <div class="tool">
    <p class="small muted">Image vue : <code>{item.path}</code></p>
    <img class="view" src={fileUrl(sessionId, item.path)} alt={item.path} />
  </div>
{:else if item.type === 'plan'}
  <div class="tool"><Markdown text={item.text ?? ''} /></div>
{:else if item.type === 'notice'}
  <p class="notice small">{item.text}</p>
{:else if item.type === 'contextCompaction'}
  <p class="small muted">Contexte résumé pour rester dans la fenêtre du modèle.</p>
{:else}
  <details class="tool">
    <summary class="muted">{item.type ?? 'élément'}</summary>
    <pre class="out">{JSON.stringify(item, null, 2)}</pre>
  </details>
{/if}

<style>
  .user {
    align-self: flex-end;
    max-width: 80%;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: var(--s2) var(--s3);
  }
  .text { white-space: pre-wrap; }
  .note { margin: var(--s1) 0 0; }
  .notice { border: 1px solid var(--line); border-radius: var(--radius); padding: var(--s2) var(--s3); margin: 0; white-space: pre-wrap; }
  .thumbs { display: flex; gap: var(--s2); flex-wrap: wrap; margin-top: var(--s2); }
  .thumbs img { max-height: 120px; max-width: 200px; border-radius: 4px; display: block; }
  .agent { max-width: 100%; }
  .tool {
    border-left: 2px solid var(--line);
    padding-left: var(--s3);
    font-size: 14px;
  }
  summary { cursor: pointer; display: flex; gap: var(--s2); align-items: baseline; flex-wrap: wrap; }
  .cmd { white-space: pre-wrap; word-break: break-all; }
  .meta { color: var(--muted); font-size: 12px; display: flex; gap: var(--s2); }
  .out {
    max-height: 360px;
    overflow: auto;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: var(--s2);
    font-size: 12px;
    white-space: pre-wrap;
    margin: var(--s2) 0 0;
  }
  .pre { white-space: pre-wrap; font-size: 13px; margin-top: var(--s1); }
  .view { max-width: 100%; max-height: 400px; border-radius: 4px; }
</style>
