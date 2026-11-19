<script lang="ts">
  import Dialog from '../../lib/ui/Dialog.svelte';
  import Button from '../../lib/ui/Button.svelte';

  let { close, choose }: { close: () => void; choose: (conversation: boolean) => void } = $props();
  let open = $state(true);
  function pick(conversation: boolean) {
    open = false;
    choose(conversation);
  }
</script>

<Dialog bind:open title="Revenir avant ce message" width="520px" onclose={close}>
  <div class="txt">
    <p>
      Les fichiers du dossier de travail reprennent leur état d’avant ce message (les fichiers ajoutés depuis sont
      supprimés, les fichiers ignorés ne bougent pas). « Rétablir » annule ce retour.
    </p>
    <p class="muted">Avec la conversation, ce message et les suivants disparaissent du fil ; son texte revient dans la zone de saisie.</p>
  </div>
  {#snippet actions()}
    <Button variant="ghost" onclick={() => { open = false; close(); }}>Annuler</Button>
    <Button onclick={() => pick(false)}>Fichiers seulement</Button>
    <Button variant="primary" onclick={() => pick(true)}>Fichiers et conversation</Button>
  {/snippet}
</Dialog>

<style>
  .txt { display: flex; flex-direction: column; gap: var(--s3); font-size: var(--fs-sm); }
  .txt p { margin: 0; }
</style>
