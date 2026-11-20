<script lang="ts">
  import { uploadAttachment, message, type Attachment } from '../../lib/api';
  import { tick } from 'svelte';
  import ImageIcon from '@lucide/svelte/icons/image';
  import Camera from '@lucide/svelte/icons/camera';
  import ArrowUp from '@lucide/svelte/icons/arrow-up';
  import Square from '@lucide/svelte/icons/square';
  import X from '@lucide/svelte/icons/x';
  import Button from '../../lib/ui/Button.svelte';
  import IconButton from '../../lib/ui/IconButton.svelte';
  import Kbd from '../../lib/ui/Kbd.svelte';
  import Icon from '../../lib/ui/Icon.svelte';
  import Select from '../../lib/ui/Select.svelte';
  import { tooltip } from '../../lib/ui/tooltip';
  import { screen } from './phone.svelte';

  let {
    sessionId,
    running,
    disabled = false,
    send,
    interrupt,
    text = $bindable(''),
    effort,
    efforts,
    effortHint = '',
    onEffort,
  }: {
    effort: string;
    efforts: { value: string; label: string }[];
    effortHint?: string;
    onEffort: (v: any) => void;
    sessionId: string;
    running: boolean;
    disabled?: boolean;
    send: (text: string, attachments: string[]) => Promise<boolean>;
    interrupt: () => void;
    /** Zone de saisie (remplie par un retour arrière de la conversation). */
    text?: string;
  } = $props();

  let images = $state<Attachment[]>([]);
  let uploading = $state(0);
  let sending = $state(false);
  let error = $state('');
  let dragging = $state(false);
  let picker: HTMLInputElement | undefined = $state();
  let camera: HTMLInputElement | undefined = $state();
  let area: HTMLTextAreaElement | undefined = $state();

  async function add(files: Iterable<File>) {
    error = '';
    for (const f of files) {
      if (!f.type.startsWith('image/')) {
        error = `${f.name} : seules les images sont acceptées.`;
        continue;
      }
      uploading++;
      try {
        images.push(await uploadAttachment(sessionId, f));
      } catch (e) {
        error = message(e);
      } finally {
        uploading--;
      }
    }
  }

  function onPaste(e: ClipboardEvent) {
    const files = [...(e.clipboardData?.files ?? [])];
    if (files.length) {
      e.preventDefault();
      add(files);
    }
  }

  function onDrop(e: DragEvent) {
    e.preventDefault();
    dragging = false;
    if (e.dataTransfer?.files.length) add([...e.dataTransfer.files]);
  }

  async function submit() {
    if (sending || running || uploading || disabled) return;
    if (!text.trim() && !images.length) return;
    sending = true;
    const ok = await send(text, images.map((i) => i.name));
    sending = false;
    if (ok) {
      text = '';
      images = [];
    }
  }

  function onKey(e: KeyboardEvent) {
    // Au téléphone, Entrée va à la ligne : on envoie avec le bouton.
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing && !screen.phone) {
      e.preventDefault();
      submit();
    }
  }

  function picked(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    add([...(input.files ?? [])]);
    input.value = '';
  }

  // Zone qui grandit avec le texte (jusqu'à 40 % de la hauteur visible).
  async function grow() {
    await tick();
    if (!area) return;
    area.style.height = 'auto';
    const max = Math.round(window.innerHeight * (screen.phone ? 0.3 : 0.4));
    area.style.height = `${Math.min(area.scrollHeight + 2, max)}px`;
  }
  $effect(() => {
    void text;
    void screen.phone;
    grow();
  });

  const canSend = $derived(!disabled && !sending && uploading === 0 && (!!text.trim() || images.length > 0));
</script>

<div class="composer" class:phone={screen.phone}>
  <div
    class="box"
    class:dragging
    role="group"
    aria-label="Nouveau message"
    ondragover={(e) => {
      e.preventDefault();
      dragging = true;
    }}
    ondragleave={() => (dragging = false)}
    ondrop={onDrop}
  >
    {#if images.length || uploading}
      <div class="thumbs">
        {#each images as img, i (img.name)}
          <div class="thumb">
            <img src={img.url} alt="Pièce jointe" />
            <button class="remove" aria-label="Retirer l’image" use:tooltip={'Retirer'} onclick={() => images.splice(i, 1)}><Icon icon={X} size={14} /></button>
          </div>
        {/each}
        {#if uploading}<span class="up">Envoi de l’image…</span>{/if}
      </div>
    {/if}
    {#if dragging}<div class="drop">Déposez l’image ici</div>{/if}

    {#if screen.phone}
      <div class="ph-row">
        <IconButton icon={Camera} label="Prendre une photo" size="lg" {disabled} onclick={() => camera?.click()} />
        <IconButton icon={ImageIcon} label="Joindre une image" size="lg" {disabled} onclick={() => picker?.click()} />
        <span class="eff ph"><Select aria-label="Niveau de réflexion" title={effortHint} value={effort} options={efforts}
          onchange={(e) => onEffort((e.currentTarget as HTMLSelectElement).value)} /></span>
        <textarea
          bind:this={area}
          bind:value={text}
          onkeydown={onKey}
          onpaste={onPaste}
          rows="1"
          placeholder="Message à EX"
          aria-label="Message à EX"
          {disabled}
        ></textarea>
        {#if running}
          <button class="round stop" aria-label="Arrêter le tour" onclick={interrupt}><Icon icon={Square} size={18} /></button>
        {:else}
          <button class="round" aria-label="Envoyer" disabled={!canSend} onclick={submit}><Icon icon={ArrowUp} size={22} /></button>
        {/if}
      </div>
    {:else}
      <textarea
        bind:this={area}
        bind:value={text}
        onkeydown={onKey}
        onpaste={onPaste}
        rows="2"
        placeholder={disabled ? 'Session archivée : désarchivez-la pour écrire.' : 'Message à EX'}
        aria-label="Message à EX"
        {disabled}
      ></textarea>
      <div class="bar">
        <Button variant="ghost" size="sm" icon={ImageIcon} {disabled} onclick={() => picker?.click()}>Joindre une image</Button>
        <span class="eff"><span class="efl">Réflexion</span><Select aria-label="Niveau de réflexion" title={effortHint} value={effort} options={efforts}
          onchange={(e) => onEffort((e.currentTarget as HTMLSelectElement).value)} /></span>
        <span class="hint"><Kbd>Entrée</Kbd> envoyer, <Kbd>Maj</Kbd>+<Kbd>Entrée</Kbd> à la ligne</span>
        {#if running}
          <Button icon={Square} onclick={interrupt}>Arrêter</Button>
        {:else}
          <Button variant="primary" icon={ArrowUp} disabled={!canSend} busy={sending} onclick={submit}>Envoyer</Button>
        {/if}
      </div>
    {/if}
  </div>
  {#if error}<p class="err" role="alert">{error}</p>{/if}
  <input bind:this={picker} type="file" accept="image/png,image/jpeg,image/webp,image/gif" multiple hidden onchange={picked} />
  <input bind:this={camera} type="file" accept="image/*" capture="environment" hidden onchange={picked} />
</div>

<style>
  .composer { flex: none; padding: var(--s2) var(--s6) var(--s4); }
  .box {
    position: relative;
    max-width: 760px;
    margin: 0 auto;
    border: 1px solid var(--line-strong);
    border-radius: var(--r);
    background: var(--panel);
  }
  .box:focus-within { border-color: var(--accent); }
  .box.dragging { border-color: var(--accent); border-style: dashed; }
  .drop {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--accent-soft) 88%, transparent);
    color: var(--accent);
    font-weight: 600;
    border-radius: var(--r);
    pointer-events: none;
    z-index: 2;
  }
  textarea {
    display: block;
    width: 100%;
    min-height: 64px;
    border: 0;
    background: transparent;
    resize: none;
    padding: var(--s3) var(--s4);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-md);
    line-height: 1.5;
    outline: none;
  }
  textarea:focus-visible { outline: none; border: 0; }
  .bar { display: flex; align-items: center; gap: var(--s2); padding: var(--s2); }
  .eff { display: inline-flex; align-items: center; gap: var(--s2); }
  .efl { font-size: var(--fs-xs); color: var(--faint); }
  .eff.ph { align-self: center; max-width: 96px; }
  .hint { margin-left: auto; margin-right: var(--s2); font-size: var(--fs-xs); color: var(--faint); white-space: nowrap; }
  .thumbs { display: flex; gap: var(--s2); flex-wrap: wrap; align-items: center; padding: var(--s3) var(--s3) 0; }
  .thumb { position: relative; border: 1px solid var(--line-strong); border-radius: var(--r); background: var(--bg); }
  .thumb img { height: 64px; display: block; border-radius: var(--r); }
  .remove {
    position: absolute;
    top: -8px;
    right: -8px;
    width: 24px;
    height: 24px;
    min-height: 0;
    padding: 0;
    display: grid;
    place-items: center;
    border: 1px solid var(--line-strong);
    border-radius: 50%;
    background: var(--panel);
    color: var(--text);
    filter: none;
    cursor: pointer;
  }
  .up { font-size: var(--fs-sm); color: var(--muted); }
  .err { max-width: 760px; margin: var(--s2) auto 0; color: var(--danger); font-size: var(--fs-sm); }
  @media (max-width: 900px) { .hint { display: none; } }

  /* Téléphone : collé en bas, champ en pilule, gros boutons */
  .composer.phone {
    padding: var(--s2) calc(var(--s2) + var(--safe-right)) calc(var(--s2) + var(--safe-bottom)) calc(var(--s2) + var(--safe-left));
    border-top: 1px solid var(--line);
    background: var(--panel);
  }
  .phone .box { border: 0; background: none; }
  .phone .thumbs { padding: var(--s2) var(--s2) var(--s3); }
  .ph-row { display: flex; align-items: flex-end; gap: 2px; }
  .phone textarea {
    flex: 1;
    min-width: 0;
    min-height: var(--touch);
    padding: var(--s3) var(--s4);
    margin: 0 var(--s1);
    border: 1px solid var(--line-strong);
    border-radius: 22px;
    background: var(--bg);
  }
  .phone textarea:focus { border-color: var(--accent); }
  .round {
    flex: none;
    width: 48px;
    height: 48px;
    min-height: 0;
    padding: 0;
    display: grid;
    place-items: center;
    border: 0;
    border-radius: 50%;
    background: var(--accent-fill);
    color: var(--accent-ink);
    filter: none;
    cursor: pointer;
  }
  .round:disabled { background: var(--raised); color: var(--faint); opacity: 1; }
  .round.stop { background: var(--raised); color: var(--text); border: 1px solid var(--line-strong); }
</style>
