<script lang="ts">
  import { sandboxStatus, type SandboxStatus } from '../lib/api';
  import Tree from './sessions/Tree.svelte';
  import Thread from './sessions/Thread.svelte';

  let { sub }: { sub: string } = $props();
  // Nombre de rafraîchissements de l'arbre demandés (titres, sessions en cours).
  let version = $state(0);
  let status = $state<SandboxStatus | null>(null);

  sandboxStatus()
    .then((s) => (status = s))
    .catch(() => {});

  const open = (id: string) => (location.hash = `sessions/${id}`);
</script>

<div class="sessions">
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
        <Thread id={sub} changed={() => version++} />
      {/key}
    {:else}
      <p class="muted pick">Choisissez une session à gauche, ou créez-en une avec « + session » à côté d’un topic.</p>
    {/if}
  </div>
</div>

<style>
  .sessions {
    display: grid;
    grid-template-columns: 260px 1fr;
    gap: var(--s6);
    height: calc(100vh - 2 * var(--s6));
    min-height: 0;
  }
  .center { display: flex; flex-direction: column; gap: var(--s3); min-height: 0; }
  .banner { border: 1px solid var(--error); color: var(--error); border-radius: var(--radius); padding: var(--s2) var(--s3); margin: 0; }
  .pick { margin-top: var(--s8); text-align: center; }
  @media (max-width: 900px) {
    .sessions { grid-template-columns: 1fr; height: auto; }
  }
</style>
