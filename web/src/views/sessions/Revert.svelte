<script lang="ts">
  let { close, choose }: { close: () => void; choose: (conversation: boolean) => void } = $props();
  let dialog: HTMLDialogElement | undefined = $state();
  $effect(() => dialog?.showModal());
</script>

<dialog bind:this={dialog} onclose={close} class="card">
  <h3>Revenir avant ce message</h3>
  <p class="small muted">
    Les fichiers du dossier de travail reprennent leur état d’avant ce message (les fichiers ajoutés depuis sont
    supprimés, les fichiers ignorés ne bougent pas). « Rétablir » annule ce retour.
  </p>
  <div class="actions">
    <button onclick={() => { choose(true); dialog?.close(); }}>Fichiers et conversation</button>
    <button class="secondary" onclick={() => { choose(false); dialog?.close(); }}>Fichiers seulement</button>
    <button class="link" onclick={() => dialog?.close()}>Annuler</button>
  </div>
  <p class="small muted">Avec la conversation, ce message et les suivants disparaissent du fil ; son texte revient dans la zone de saisie.</p>
</dialog>

<style>
  dialog { color: var(--text); max-width: 520px; }
  dialog::backdrop { background: rgba(0, 0, 0, 0.5); }
  h3 { margin-top: 0; }
</style>
