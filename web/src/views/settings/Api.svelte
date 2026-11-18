<script lang="ts">
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
    if (!confirm(`Révoquer la clé « ${k.name} » ? Les programmes qui l’utilisent seront refusés.`)) return;
    try {
      await deleteApiKey(k.id);
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

  const date = (t: number | null) => (t ? new Date(t * 1000).toLocaleString('fr-FR') : 'jamais');
  let envLine = $derived(`export ${config?.env_key ?? 'ENGINEX_API_KEY'}=${created?.key ?? '<clé>'}`);

  load();
</script>

{#if error}<p class="error">{error}</p>{/if}

<section class="card">
  <h3>Clés d’API</h3>
  <p class="small muted">
    Codex et les autres clients OpenAI s’identifient avec une clé sur <code>{config?.base_url ?? '/v1'}</code>.
    La valeur n’est montrée qu’à la création ; une clé perdue se remplace par une nouvelle.
  </p>
  <form class="row wrap" onsubmit={create}>
    <input bind:value={name} placeholder="Nom (ex. codex-portable)" maxlength="50" required />
    <button type="submit" disabled={!name.trim()}>Nouvelle clé</button>
  </form>
  {#if created}
    <div class="created">
      <p class="small">Clé « {created.name} » : copiez-la maintenant, elle ne sera plus affichée.</p>
      <div class="row wrap">
        <code class="secret">{created.key}</code>
        <button class="secondary" onclick={() => created && copy(created.key, 'key')}>{copied === 'key' ? 'Copiée' : 'Copier'}</button>
        <button class="link" onclick={() => (created = null)}>Masquer</button>
      </div>
    </div>
  {/if}
  {#if keys.length}
    <table>
      <thead><tr><th>Nom</th><th>Créée le</th><th>Dernière utilisation</th><th></th></tr></thead>
      <tbody>
        {#each keys as k (k.id)}
          <tr>
            <td>{k.name}</td>
            <td class="small">{date(k.created_at)}</td>
            <td class="small">{date(k.last_used_at)}</td>
            <td class="actions"><button class="link" onclick={() => revoke(k)}>Révoquer</button></td>
          </tr>
        {/each}
      </tbody>
    </table>
  {:else}
    <p class="muted small">Aucune clé.</p>
  {/if}
</section>

<section class="card">
  <h3>Configuration de Codex</h3>
  {#if config}
    <p class="small muted">
      À mettre dans <code>~/.codex/config.toml</code>, pour le preset actif. La clé se lit dans la variable
      <code>{config.env_key}</code>.
    </p>
    <pre class="toml">{config.toml}</pre>
    <div class="actions">
      <button class="secondary" onclick={() => config && copy(config.toml, 'toml')}>{copied === 'toml' ? 'Copié' : 'Copier le config.toml'}</button>
    </div>
    <pre class="toml">{envLine}</pre>
    <div class="actions">
      <button class="secondary" onclick={() => copy(envLine, 'env')}>{copied === 'env' ? 'Copié' : 'Copier la ligne export'}</button>
    </div>
  {:else if configError}
    <p class="muted">{configError}</p>
  {/if}
</section>

<style>
  .created {
    margin-top: var(--s3);
    padding: var(--s3);
    border: 1px solid var(--accent);
    border-radius: 6px;
  }
  .secret {
    word-break: break-all;
    user-select: all;
  }
  .toml {
    white-space: pre-wrap;
    user-select: all;
  }
</style>
