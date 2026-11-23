<script lang="ts">
  import { stamp } from '../../lib/ui/format';
  import Button from '../../lib/ui/Button.svelte';
  import Input from '../../lib/ui/Input.svelte';
  import Switch from '../../lib/ui/Switch.svelte';
  import StatusDot from '../../lib/ui/StatusDot.svelte';
  import { toast } from '../../lib/ui/toast.svelte';
  import Section from './Section.svelte';
  import Note from './Note.svelte';
  import {
    sandboxStatus, sandboxPull, sessionSettings, saveSessionSettings, message, type SandboxStatus,
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
      toast('Réglages des sessions enregistrés.', 'ok');
    } catch (err) {
      error = message(err);
    }
  }

  let pulling = $state(false);
  async function pullNow() {
    pulling = true;
    try {
      status = await sandboxPull();
      if (status.image.last_pull_error) toast(`Échec de la mise à jour : ${status.image.last_pull_error}`, 'error');
      else toast('Image à jour : les prochaines sandboxes l’utilisent.', 'ok');
    } catch (e) {
      toast(message(e), 'error');
    } finally {
      pulling = false;
    }
  }

  const date = (t: number | null) => (t ? stamp(t) : 'jamais');
  load();
</script>

{#if error}<div class="err"><Note tone="danger">{error}</Note></div>{/if}

<form onsubmit={save}>
  <Section title="Sandboxes" text="Chaque session a sa sandbox Docker, où tourne EX. Elle démarre au premier message ; le code et l’historique sont gardés.">
    <div class="g2">
      <Input label="Arrêt après inactivité" unit="min" hint="5 à 1440, sans tour en cours" type="number" min="5" max="1440" bind:value={idle} required />
      <Input label="Sandboxes actives en même temps" hint="1 à 10" type="number" min="1" max="10" bind:value={max} required />
    </div>
  </Section>

  <Section title="Vérification" text="Tests lancés par EngineX après chaque tour qui change des fichiers ; un échec est renvoyé à EX.">
    <Switch bind:checked={verifyOn}>Vérifier après chaque tour qui change des fichiers</Switch>
    <div class="g2">
      <Input label="Corrections automatiques au plus" hint="0 à 10" type="number" min="0" max="10" bind:value={fixes} required disabled={!verifyOn} />
    </div>
    <Switch bind:checked={appCheck}>Essai de l’app au navigateur après une modification de l’interface</Switch>
    <p class="small muted">Si l’agent ne l’a pas ouverte lui-même : un tour de plus, un seul par message ; demande le navigateur de la stack.</p>
  </Section>

  <Section title="Mémoire" text="Un tour d’EX avant l’arrêt pour inactivité, s’il y a eu un message depuis le précédent.">
    <Switch bind:checked={memoryAuto}>Mettre à jour la mémoire en fin de session</Switch>
  </Section>

  <Section title="Git" text="Auteur des commits faits par EngineX dans les dépôts des topics.">
    <div class="g2">
      <Input label="Nom git des commits" bind:value={gitName} placeholder="EngineX" maxlength={100} />
      <Input label="E-mail git" type="email" bind:value={gitEmail} placeholder="enginex@localhost" maxlength={200} />
    </div>
  </Section>

  <Section title="Disque" text="Bandeau sur les pages Fichiers et Système ; les fichiers des sessions sont gardés jusqu’à leur suppression.">
    <div class="g2">
      <Input label="Alerte disque de /data au-delà de" unit="%" hint="50 à 99" type="number" min="50" max="99" bind:value={diskPct} required />
    </div>
  </Section>

  <div class="savebar"><Button type="submit" variant="primary">Enregistrer</Button></div>
</form>

{#if status}
  <Section title="État" text="Ce que voit EngineX de Docker en ce moment.">
    <dl class="kv">
      <dt>Mode</dt>
      <dd>{status.backend === 'docker' ? 'Docker' : 'Local, sans isolation (développement)'}</dd>
      {#if status.backend === 'docker'}
        <dt>Docker</dt>
        <dd>
          <span class="st"><StatusDot kind={status.docker.reachable ? 'ok' : 'err'} />{status.docker.reachable ? 'joignable' : 'injoignable'}</span>
          {#if status.docker.error}<div class="sub mono">{status.docker.error}</div>{/if}
        </dd>
        <dt>Dossier /data sur l’hôte</dt>
        <dd>
          {#if status.host_data.path}<code>{status.host_data.path}</code>{:else}<span class="bad">introuvable</span>{/if}
          {#if status.host_data.error}<div class="sub mono">{status.host_data.error}</div>{/if}
        </dd>
        <dt>Image devbox</dt>
        <dd>
          <code>{status.image.name}</code>
          <span class="st">
            <StatusDot kind={status.image.present ? 'ok' : 'off'} />
            {status.image.present ? 'présente' : 'pas encore téléchargée (au premier message)'}
          </span>
          {#if status.image.codex}<div class="sub">Version d’EX : <span class="mono">{status.image.codex}</span></div>{/if}
          <div class="sub">Dernière mise à jour : {date(status.image.last_pull_at)} · vérifiée toutes les 6 h, EX testé et republié chaque jour</div>
          <Button size="sm" variant="secondary" disabled={pulling} onclick={pullNow}>{pulling ? 'Mise à jour…' : 'Mettre à jour maintenant'}</Button>
          {#if status.image.last_pull_error}<div class="sub bad">Échec de la mise à jour : {status.image.last_pull_error}</div>{/if}
        </dd>
      {/if}
      <dt>Sandboxes actives</dt>
      <dd class="num">{status.live}</dd>
    </dl>
  </Section>
{/if}

<style>
  .err { padding: var(--s4) var(--s6) 0; }
  .g2 { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: var(--s4); }
  .savebar {
    position: sticky;
    bottom: 0;
    z-index: 1;
    display: flex;
    padding: var(--s3) var(--s6) var(--s3) calc(var(--s6) + 220px + var(--s8));
    background: var(--panel);
    border-bottom: 1px solid var(--line);
  }
  @media (max-width: 1023px) { .savebar { padding-left: var(--s4); } }
  .kv { display: grid; grid-template-columns: max-content minmax(0, 1fr); gap: var(--s3) var(--s5); margin: 0; font-size: var(--fs-sm); }
  .kv dt { color: var(--muted); }
  .kv dd { margin: 0; display: flex; flex-direction: column; align-items: flex-start; gap: var(--s1); overflow-wrap: anywhere; }
  .st { display: inline-flex; align-items: center; gap: var(--s2); }
  .sub { color: var(--muted); }
  .bad { color: var(--danger); }
  @media (max-width: 767px) {
    .err { padding: var(--s4) var(--s4) 0; }
    .g2 { grid-template-columns: minmax(0, 1fr); }
    .g2 :global(.box) { height: var(--touch); }
    .savebar { padding: var(--s3) var(--s4); bottom: 0; }
    .savebar :global(.btn) { height: var(--touch); flex: 1; }
    .kv { grid-template-columns: minmax(0, 1fr); gap: var(--s1); }
    .kv dd { margin-bottom: var(--s3); }
  }
</style>
