<script lang="ts">
  import { onMount } from 'svelte';
  import { message, setup } from '../lib/api';
  import { Button, Icon, Input } from '../lib/ui';
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
  import Logo from './shell/Logo.svelte';

  let { done }: { done: () => void } = $props();
  let code = $state('');
  let password = $state('');
  let confirm = $state('');
  let error = $state('');
  let busy = $state(false);
  let ref = $state<HTMLInputElement>();

  onMount(() => ref?.focus());

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

<main class="gate">
  <div class="col">
    <div class="brand"><Logo size={32} /><span class="name">EngineX</span><span class="what">Atelier de code local</span></div>
    <form class="box" onsubmit={submit}>
      <div class="intro">
        <h1>Première connexion</h1>
        <p>Choisissez le mot de passe de l’interface. Le code de configuration est écrit dans les journaux du conteneur :</p>
        <pre class="cmd">docker logs enginex</pre>
      </div>
      <Input label="Code de configuration" size="lg" class="mono-in" bind:value={code} bind:ref autocomplete="off" spellcheck="false" required />
      <Input label="Mot de passe" hint="12 caractères au moins." type="password" size="lg" bind:value={password} autocomplete="new-password" minlength={12} required />
      <Input label="Confirmation" type="password" size="lg" bind:value={confirm} autocomplete="new-password" required />
      {#if error}<p class="err" role="alert"><Icon icon={TriangleAlert} size={16} /><span>{error}</span></p>{/if}
      <Button variant="primary" size="lg" type="submit" full busy={busy}>{busy ? 'Création…' : 'Créer le mot de passe'}</Button>
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
  .col { width: min(440px, 100%); display: flex; flex-direction: column; gap: var(--s6); }
  .brand { display: flex; align-items: center; gap: var(--s3); color: var(--accent); }
  .name { color: var(--text); font-size: var(--fs-xl); font-weight: 600; letter-spacing: -0.01em; }
  .what { margin-left: auto; color: var(--muted); font-size: var(--fs-sm); }
  .box { display: flex; flex-direction: column; gap: var(--s5); padding: var(--s6); background: var(--bg); border: 1px solid var(--line); border-radius: var(--r); }
  .box :global(.l) { color: var(--text); font-size: var(--fs-md); }
  .box :global(.hint), .box :global(.err) { font-size: var(--fs-sm); }
  .box :global(.mono-in input) { font-family: var(--mono); }
  .intro { display: flex; flex-direction: column; gap: var(--s2); padding-bottom: var(--s5); border-bottom: 1px solid var(--line); }
  h1 { margin: 0; font-size: var(--fs-lg); font-weight: 600; }
  .intro p { margin: 0; color: var(--muted); font-size: var(--fs-sm); line-height: 1.55; }
  .cmd { margin: 0; padding: var(--s2) var(--s3); background: var(--panel); border: 1px solid var(--line); border-radius: var(--r); color: var(--text); user-select: all; }
  .err { display: flex; align-items: flex-start; gap: var(--s2); margin: 0; padding: var(--s2) var(--s3); background: var(--danger-soft); color: var(--danger); border-radius: var(--r); font-size: var(--fs-sm); }
  .err :global(.ui-icon) { margin-top: 3px; }
  @media (max-width: 767px) {
    .gate { place-items: start center; padding-top: var(--s8); }
    .box { padding: var(--s5) var(--s4); }
  }
</style>
