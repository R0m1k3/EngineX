<script lang="ts">
  import { login, message } from '../lib/api';

  let { done }: { done: () => void } = $props();
  let password = $state('');
  let error = $state('');
  let busy = $state(false);

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    error = '';
    busy = true;
    try {
      await login(password);
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
    <label>Mot de passe <input type="password" bind:value={password} autocomplete="current-password" required /></label>
    {#if error}<p class="error">{error}</p>{/if}
    <button type="submit" disabled={busy}>Se connecter</button>
  </form>
</main>
