<script lang="ts">
  import { authState, setUnauthorizedHandler, type AuthState } from './lib/api';
  import Setup from './views/Setup.svelte';
  import Login from './views/Login.svelte';
  import Shell from './views/Shell.svelte';

  let auth = $state<AuthState | null>(null);
  let failed = $state(false);

  async function refresh() {
    try {
      auth = await authState();
      failed = false;
    } catch {
      failed = true;
    }
  }

  setUnauthorizedHandler(() => {
    auth = { setup_required: false, authenticated: false };
  });
  refresh();
</script>

{#if failed}
  <main class="center"><p class="error">Serveur injoignable. <button onclick={refresh}>Réessayer</button></p></main>
{:else if auth === null}
  <main class="center"><p class="muted">Chargement…</p></main>
{:else if auth.setup_required}
  <Setup done={refresh} />
{:else if !auth.authenticated}
  <Login done={refresh} />
{:else}
  <Shell loggedOut={refresh} />
{/if}
