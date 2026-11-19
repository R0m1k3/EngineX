<script lang="ts">
  import {
    sandboxStatus, sessionSettings, saveSessionSettings, message, type SandboxStatus,
  } from '../../lib/api';

  let status = $state<SandboxStatus | null>(null);
  let idle = $state(10);
  let max = $state(3);
  let verifyOn = $state(true);
  let fixes = $state(3);
  let appCheck = $state(true);
  let memoryAuto = $state(true);
  let gitName = $state('');
  let gitEmail = $state('');
  let diskPct = $state(90);
  let error = $state('');
  let saved = $state(false);

  async function load() {
    try {
      const s = await sessionSettings();
      idle = s.idle_minutes;
      max = s.max;
      verifyOn = s.verify_enabled ?? true;
      fixes = s.verify_max_fixes ?? 3;
      appCheck = s.verify_app_check ?? true;
      memoryAuto = s.memory_auto ?? true;
      gitName = s.git_name ?? '';
      gitEmail = s.git_email ?? '';
      diskPct = s.disk_alert_pct ?? 90;
      status = await sandboxStatus();
      error = '';
    } catch (e) {
      error = message(e);
    }
  }

  async function save(e: SubmitEvent) {
    e.preventDefault();
    error = '';
    saved = false;
    try {
      const s = await saveSessionSettings({
        idle_minutes: idle,
        max,
        verify_enabled: verifyOn,
        verify_max_fixes: fixes,
        verify_app_check: appCheck,
        memory_auto: memoryAuto,
        git_name: gitName.trim() || undefined,
        git_email: gitEmail.trim() || undefined,
        disk_alert_pct: diskPct,
      });
      idle = s.idle_minutes;
      max = s.max;
      saved = true;
    } catch (err) {
      error = message(err);
    }
  }

  const date = (t: number | null) => (t ? new Date(t * 1000).toLocaleString('fr-FR') : 'jamais');
  load();
</script>

{#if error}<p class="error">{error}</p>{/if}

<section class="card">
  <h3>Sandboxes</h3>
  <p class="small muted">
    Chaque session a sa sandbox Docker, où tourne Codex. Elle démarre au premier message et s’arrête après le délai
    ci-dessous sans tour en cours. Le code et l’historique sont gardés.
  </p>
  <form class="fields" onsubmit={save}>
    <label>Arrêt après inactivité (minutes, 5 à 1440)
      <input class="num" type="number" min="5" max="1440" bind:value={idle} required />
    </label>
    <label>Sandboxes actives en même temps (1 à 10)
      <input class="num" type="number" min="1" max="10" bind:value={max} required />
    </label>
    <label class="check"><input type="checkbox" bind:checked={verifyOn} /> Vérification après chaque tour qui change des fichiers</label>
    <label>Corrections automatiques au plus (0 à 10)
      <input class="num" type="number" min="0" max="10" bind:value={fixes} required disabled={!verifyOn} />
    </label>
    <label class="check"><input type="checkbox" bind:checked={appCheck} /> Essai de l’app au navigateur après une modification de l’interface
      <span class="small muted">(si l’agent ne l’a pas ouverte lui-même : un tour de plus, un seul par message ; demande le navigateur de la stack)</span></label>
    <label class="check"><input type="checkbox" bind:checked={memoryAuto} /> Mettre à jour la mémoire en fin de session
      <span class="small muted">(un tour de Codex avant l’arrêt pour inactivité, s’il y a eu un message depuis le précédent)</span></label>
    <label>Nom git des commits d’EngineX
      <input bind:value={gitName} placeholder="EngineX" maxlength="100" />
    </label>
    <label>E-mail git
      <input type="email" bind:value={gitEmail} placeholder="enginex@localhost" maxlength="200" />
    </label>
    <label>Alerte disque de /data au-delà de (%, 50 à 99)
      <input class="num" type="number" min="50" max="99" bind:value={diskPct} required />
      <span class="small muted">Bandeau sur les pages Fichiers et Système ; les fichiers des sessions sont gardés jusqu’à leur suppression.</span>
    </label>
    <div class="actions"><button type="submit">Enregistrer</button>{#if saved}<span class="ok small">Enregistré.</span>{/if}</div>
  </form>
</section>

{#if status}
  <section class="card">
    <h3>État</h3>
    <table>
      <tbody>
        <tr><th>Mode</th><td>{status.backend === 'docker' ? 'Docker' : 'Local, sans isolation (développement)'}</td></tr>
        {#if status.backend === 'docker'}
          <tr>
            <th>Docker</th>
            <td>
              {#if status.docker.reachable}<span class="ok">joignable</span>{:else}<span class="error">injoignable</span>{/if}
              {#if status.docker.error}<div class="small muted">{status.docker.error}</div>{/if}
            </td>
          </tr>
          <tr>
            <th>Dossier /data sur l’hôte</th>
            <td>
              {#if status.host_data.path}<code>{status.host_data.path}</code>{:else}<span class="error">introuvable</span>{/if}
              {#if status.host_data.error}<div class="small muted">{status.host_data.error}</div>{/if}
            </td>
          </tr>
          <tr>
            <th>Image devbox</th>
            <td>
              <code>{status.image.name}</code>
              {#if status.image.present}<span class="ok">présente</span>{:else}<span class="muted">pas encore téléchargée (au premier message)</span>{/if}
              <div class="small muted">Dernière mise à jour : {date(status.image.last_pull_at)}</div>
              {#if status.image.last_pull_error}<div class="small error">Échec de la mise à jour : {status.image.last_pull_error}</div>{/if}
            </td>
          </tr>
        {/if}
        <tr><th>Sandboxes actives</th><td>{status.live}</td></tr>
      </tbody>
    </table>
  </section>
{/if}
