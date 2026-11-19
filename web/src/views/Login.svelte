<script lang="ts">
  import { onMount } from 'svelte';
  import { login, message } from '../lib/api';
  import { Button, Input } from '../lib/ui';
  import Logo from './shell/Logo.svelte';

  let { done }: { done: () => void } = $props();
  let password = $state('');
  let error = $state('');
  let busy = $state(false);
  let ref = $state<HTMLInputElement>();

  onMount(() => ref?.focus());

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    error = '';
    busy = true;
    try {
      await login(password);
      done();
    } catch (err) {
      error = message(err);
      ref?.select();
    } finally {
      busy = false;
    }
  }
</script>

<main class="gate">
  <div class="col">
    <div class="brand"><Logo size={32} /><span class="name">EngineX</span><span class="what">Atelier de code local</span></div>
    <form class="box" onsubmit={submit}>
      <Input
        label="Mot de passe"
        type="password"
        size="lg"
        bind:value={password}
        bind:ref
        autocomplete="current-password"
        required
        error={error || undefined}
      />
      <Button variant="primary" size="lg" type="submit" full busy={busy}>{busy ? 'Connexion…' : 'Se connecter'}</Button>
    </form>
  </div>
</main>

<style>
  .gate {
    min-height: 100dvh;
    display: grid;
    place-items: center;
    padding: var(--s6) calc(var(--s4) + var(--safe-right)) var(--s6) calc(var(--s4) + var(--safe-left));
    background: var(--panel);
  }
  .col { width: min(380px, 100%); display: flex; flex-direction: column; gap: var(--s6); }
  .brand { display: flex; align-items: center; gap: var(--s3); color: var(--accent); }
  .name { color: var(--text); font-size: var(--fs-xl); font-weight: 600; letter-spacing: -0.01em; }
  .what { margin-left: auto; color: var(--muted); font-size: var(--fs-sm); }
  .box { display: flex; flex-direction: column; gap: var(--s5); padding: var(--s6); background: var(--bg); border: 1px solid var(--line); border-radius: var(--r); }
  .box :global(.l) { color: var(--text); font-size: var(--fs-md); }
  .box :global(.err) { font-size: var(--fs-sm); }
  @media (max-width: 767px) {
    .gate { place-items: start center; padding-top: max(15vh, var(--s8)); }
    .box { padding: var(--s5) var(--s4); }
  }
</style>
