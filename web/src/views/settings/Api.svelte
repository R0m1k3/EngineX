<script lang="ts">
  import { stamp } from '../../lib/ui/format';
  import Plus from '@lucide/svelte/icons/plus';
  import Copy from '@lucide/svelte/icons/copy';
  import Check from '@lucide/svelte/icons/check';
  import Button from '../../lib/ui/Button.svelte';
  import Input from '../../lib/ui/Input.svelte';
  import { confirmDialog } from '../../lib/ui/dialog.svelte';
  import { toast } from '../../lib/ui/toast.svelte';
  import Section from './Section.svelte';
  import Note from './Note.svelte';
  import {
    apiKeys, createApiKey, deleteApiKey, codexConfig, message, ApiError,
    type ApiKey, type CodexConfig,
  } from '../../lib/api';

  let keys = $state<ApiKey[]>([]);
  let config = $state<CodexConfig | null>(null);
  let configError = $state('');
  let error = $state('');
  let name = $state('');
  // Valeur de la clé créée : montrée une seule fois.
  let created = $state<{ name: string; key: string } | null>(null);
  let copied = $state('');

  async function load() {
    try {
      keys = await apiKeys();
      error = '';
    } catch (e) {
      error = message(e);
    }
    try {
      config = await codexConfig();
      configError = '';
    } catch (e) {
      config = null;
      configError = e instanceof ApiError && e.code === 'no_active_preset'
        ? 'Aucun preset actif : activez-en un dans Modèles pour obtenir la configuration.'
        : message(e);
    }
  }

  async function create(e: SubmitEvent) {
    e.preventDefault();
    error = '';
    try {
      const r = await createApiKey(name.trim());
      created = { name: r.name, key: r.key };
      name = '';
      await load();
    } catch (err) {
      error = message(err);
    }
  }

  async function revoke(k: ApiKey) {
    const ok = await confirmDialog(`Révoquer la clé « ${k.name} » ? Les programmes qui l’utilisent seront refusés.`, {
      title: 'Révoquer la clé',
      confirm: 'Révoquer',
      danger: true,
    });
    if (!ok) return;
    try {
      await deleteApiKey(k.id);
      toast(`Clé « ${k.name} » révoquée.`, 'ok');
      await load();
    } catch (err) {
      error = message(err);
    }
  }

  async function copy(text: string, what: string) {
    try {
      await navigator.clipboard.writeText(text);
      copied = what;
      setTimeout(() => (copied = ''), 2000);
    } catch {
      copied = '';
      error = 'Copie refusée par le navigateur : sélectionnez le texte à la main.';
    }
  }

  const date = (t: number | null) => (t ? stamp(t) : 'jamais');
  let envLine = $derived(`export ${config?.env_key ?? 'ENGINEX_API_KEY'}=${created?.key ?? '<clé>'}`);

  load();
</script>

{#if error}<div class="err"><Note tone="danger">{error}</Note></div>{/if}

<Section title="Clés d’API" text="EX et les autres clients OpenAI s’identifient avec une clé. Sa valeur n’est montrée qu’à la création.">
  <p class="hint">Adresse : <code>{config?.base_url ?? '/v1'}</code>. Une clé perdue se remplace par une nouvelle.</p>
  <form class="newkey" onsubmit={create}>
    <Input label="Nom de la clé" bind:value={name} placeholder="ex-portable" maxlength={50} required />
    <Button type="submit" variant="primary" icon={Plus} disabled={!name.trim()}>Nouvelle clé</Button>
  </form>
  {#if created}
    <div class="created" role="status">
      <p>Clé « {created.name} » : copiez-la maintenant, elle ne sera plus affichée.</p>
      <code class="secret">{created.key}</code>
      <div class="acts">
        <Button icon={copied === 'key' ? Check : Copy} onclick={() => created && copy(created.key, 'key')}>{copied === 'key' ? 'Copiée' : 'Copier'}</Button>
        <Button variant="ghost" onclick={() => (created = null)}>Masquer</Button>
      </div>
    </div>
  {/if}
  {#if keys.length}
    <ul class="rows" aria-label="Clés d’API">
      {#each keys as k (k.id)}
        <li>
          <div class="rmain">
            <span class="rname">{k.name}</span>
            <span class="rmeta">Créée le {date(k.created_at)}, dernière utilisation : {date(k.last_used_at)}</span>
          </div>
          <Button variant="danger" size="sm" onclick={() => revoke(k)}>Révoquer</Button>
        </li>
      {/each}
    </ul>
  {:else}
    <p class="hint">Aucune clé.</p>
  {/if}
</Section>

<Section title="Configuration d’un client externe" text="Pour utiliser EngineX depuis un autre client OpenAI, avec le preset actif.">
  {#if config}
    {@const cfg = config}
    <p class="hint">À mettre dans <code>~/.codex/config.toml</code>. La clé se lit dans la variable <code>{cfg.env_key}</code>.</p>
    <div class="snip">
      <div class="snip-h">
        <span class="mono">config.toml</span>
        <Button size="sm" variant="ghost" icon={copied === 'toml' ? Check : Copy} onclick={() => copy(cfg.toml, 'toml')}>{copied === 'toml' ? 'Copié' : 'Copier'}</Button>
      </div>
      <pre>{cfg.toml}</pre>
    </div>
    <div class="snip">
      <div class="snip-h">
        <span class="mono">shell</span>
        <Button size="sm" variant="ghost" icon={copied === 'env' ? Check : Copy} onclick={() => copy(envLine, 'env')}>{copied === 'env' ? 'Copié' : 'Copier'}</Button>
      </div>
      <pre>{envLine}</pre>
    </div>
  {:else if configError}
    <p class="hint">{configError}</p>
  {/if}
</Section>

<style>
  .err { padding: var(--s4) var(--s6) 0; }
  .hint { margin: 0; color: var(--muted); font-size: var(--fs-sm); }
  .newkey { display: flex; gap: var(--s2); align-items: flex-end; max-width: 520px; }
  .newkey > :global(.f) { flex: 1; }
  .created {
    display: flex;
    flex-direction: column;
    gap: var(--s2);
    padding: var(--s3);
    border: 1px solid var(--accent);
    border-radius: var(--r);
    background: var(--accent-soft);
    font-size: var(--fs-sm);
  }
  .created p { margin: 0; }
  .secret { word-break: break-all; user-select: all; color: var(--text); }
  .acts { display: flex; gap: var(--s2); }
  .rows { list-style: none; margin: 0; padding: 0; border-top: 1px solid var(--line); }
  .rows li { display: flex; align-items: center; gap: var(--s3); padding: var(--s3) 0; border-bottom: 1px solid var(--line); }
  .rmain { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .rname { font-weight: 600; }
  .rmeta { color: var(--muted); font-size: var(--fs-sm); }
  .snip { border: 1px solid var(--line); border-radius: var(--r); overflow: hidden; }
  .snip-h { display: flex; align-items: center; justify-content: space-between; height: 36px; padding: 0 var(--s1) 0 var(--s3); background: var(--panel); border-bottom: 1px solid var(--line); color: var(--muted); }
  .snip pre { margin: 0; padding: var(--s3); overflow-x: auto; white-space: pre-wrap; overflow-wrap: anywhere; user-select: all; line-height: 1.6; }
  @media (max-width: 767px) {
    .err { padding: var(--s4) var(--s4) 0; }
    .newkey { flex-direction: column; align-items: stretch; }
    .newkey :global(.box), .newkey :global(.btn), .acts :global(.btn), .rows :global(.btn) { height: var(--touch); }
  }
</style>
