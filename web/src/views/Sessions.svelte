<script lang="ts">
  import type { SideTab } from '../lib/api';
  import { sandboxStatus, type SandboxStatus } from '../lib/api';
  import Tree from './sessions/Tree.svelte';
  import Thread from './sessions/Thread.svelte';
  import Side from './sessions/Side.svelte';

  let { sub }: { sub: string } = $props();
  // Nombre de rafraîchissements de l'arbre demandés (titres, sessions en cours).
  let version = $state(0);
  let status = $state<SandboxStatus | null>(null);

  sandboxStatus()
    .then((s) => (status = s))
    .catch(() => {});

  const open = (id: string) => (location.hash = `sessions/${id}`);
  // Panneau latéral (Fichiers, Terminal) : fermé par défaut.
  let side = $state<SideTab | null>(null);
  let tab = $state<SideTab>('files');
  const panel = (t: SideTab) => {
    if (side === t) side = null;
    else side = tab = t;
  };
</script>

<div class="sessions" class:with-side={!!side && !!sub}>
  <Tree selected={sub} {version} {open} />
  <div class="center">
    {#if status && !status.available}
      <p class="banner">
        Docker injoignable ou dossier /data introuvable : les sessions ne peuvent pas démarrer.
        {status.docker.error ?? status.host_data.error ?? ''} <a href="#settings/sessions">Détails</a>
      </p>
    {/if}
    {#if sub}
      {#key sub}
        <Thread id={sub} changed={() => version++} {panel} />
      {/key}
    {:else}
      <p class="muted pick">Choisissez une session à gauche, ou créez-en une avec « + session » à côté d’un topic.</p>
    {/if}
  </div>
  {#if side && sub}
    {#key sub}
      <div class="panel"><Side sessionId={sub} bind:tab close={() => (side = null)} /></div>
    {/key}
  {/if}
</div>

<style>
  .sessions {
    display: grid;
    grid-template-columns: 260px 1fr;
    gap: var(--s6);
    height: calc(100vh - 2 * var(--s6));
    min-height: 0;
  }
  .sessions.with-side { grid-template-columns: 260px 1fr 45%; }
  .panel { min-height: 0; min-width: 0; }
  @media (max-width: 1200px) {
    .sessions.with-side { grid-template-columns: 260px 1fr; }
    .panel { position: fixed; top: var(--s4); right: var(--s4); bottom: var(--s4); width: min(720px, 92vw); z-index: 10; }
  }
  .center { display: flex; flex-direction: column; gap: var(--s3); min-height: 0; }
  .banner { border: 1px solid var(--error); color: var(--error); border-radius: var(--radius); padding: var(--s2) var(--s3); margin: 0; }
  .pick { margin-top: var(--s8); text-align: center; }
  @media (max-width: 900px) {
    .sessions { grid-template-columns: 1fr; height: auto; }
  }
</style>
