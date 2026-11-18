<script lang="ts">
  import { message, setup } from '../lib/api';

  let { done }: { done: () => void } = $props();
  let code = $state('');
  let password = $state('');
  let confirm = $state('');
  let error = $state('');
  let busy = $state(false);

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    error = '';
    if (password !== confirm) {
      error = 'Les deux mots de passe diffèrent.';
      return;
    }
    busy = true;
    try {
      await setup(code, password);
      done();
    } catch (err) {
      error = message(err);
    } finally {
      busy = false;
    }
  }
</script>

<main class="center">
  <form class="card narrow" onsubmit={submit}>
    <h1>EngineX</h1>
    <p class="muted">Première connexion : choisissez le mot de passe. Le code de configuration est dans les logs du conteneur (<code>docker logs enginex</code>).</p>
    <label>Code de configuration <input bind:value={code} autocomplete="off" required /></label>
    <label>Mot de passe (12 caractères minimum) <input type="password" bind:value={password} autocomplete="new-password" minlength="12" required /></label>
    <label>Confirmation <input type="password" bind:value={confirm} autocomplete="new-password" required /></label>
    {#if error}<p class="error">{error}</p>{/if}
    <button type="submit" disabled={busy}>Créer le mot de passe</button>
  </form>
</main>
