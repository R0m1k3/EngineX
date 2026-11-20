<script lang="ts">
  import { uploadAttachment, message, type Attachment } from '../../lib/api';
  import { tick } from 'svelte';
  import ArrowUp from '@lucide/svelte/icons/arrow-up';
  import Square from '@lucide/svelte/icons/square';
  import X from '@lucide/svelte/icons/x';
  import Paperclip from '@lucide/svelte/icons/paperclip';
  import Brain from '@lucide/svelte/icons/brain';
  import ChevronDown from '@lucide/svelte/icons/chevron-down';
  import LoaderCircle from '@lucide/svelte/icons/loader-circle';
  import IconButton from '../../lib/ui/IconButton.svelte';
  import Icon from '../../lib/ui/Icon.svelte';
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

    <textarea
      bind:this={area}
      bind:value={text}
      onkeydown={onKey}
      onpaste={onPaste}
      rows={screen.phone ? 1 : 2}
      placeholder={disabled ? 'Session archivée : désarchivez-la pour écrire.' : 'Message à EX'}
      aria-label="Message à EX"
      title={screen.phone ? undefined : 'Entrée : envoyer, Maj+Entrée : à la ligne'}
      {disabled}
    ></textarea>
    <div class="row">
      <IconButton icon={Paperclip} label="Joindre une image" size={screen.phone ? 'lg' : 'md'} {disabled} onclick={() => picker?.click()} />
      <label class="eff" use:tooltip={effortHint}>
          <Icon icon={Brain} size={15} />
          <select aria-label="Niveau de réflexion" value={effort} {disabled}
            onchange={(e) => onEffort((e.currentTarget as HTMLSelectElement).value)}>
            {#each efforts as o (o.value)}<option value={o.value}>{o.label}</option>{/each}
          </select>
          <Icon icon={ChevronDown} size={14} />
        </label>
      <span class="grow"></span>
      {#if running}
        <button class="round stop" aria-label="Arrêter le tour" use:tooltip={'Arrêter'} onclick={interrupt}><Icon icon={Square} size={14} /></button>
      {:else}
        <button class="round" aria-label="Envoyer" use:tooltip={screen.phone ? null : { text: 'Envoyer', kbd: 'Entrée' }} disabled={!canSend} onclick={submit}>
          {#if sending}<Icon icon={LoaderCircle} size={18} class="spin" />{:else}<Icon icon={ArrowUp} size={18} />{/if}
        </button>
      {/if}
    </div>
  </div>
  {#if error}<p class="err" role="alert">{error}</p>{/if}
  <input bind:this={picker} type="file" accept="image/*" multiple hidden onchange={picked} />
</div>

<style>
  .composer { flex: none; padding: var(--s2) var(--s6) var(--s4); }
  .box {
    position: relative;
    max-width: 760px;
    margin: 0 auto;
    border: 1px solid var(--line-strong);
    border-radius: 14px;
    background: var(--panel);
    box-shadow: 0 1px 2px rgb(0 0 0 / 0.04);
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
    min-height: 52px;
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
  .row { display: flex; align-items: center; gap: var(--s1); padding: 0 var(--s2) var(--s2); }
  .grow { flex: 1; }
  .eff {
    position: relative;
    display: inline-flex;
    flex-direction: row;
    align-items: center;
    gap: 4px;
    height: 30px;
    padding: 0 var(--s2);
    border-radius: 15px;
    color: var(--muted);
    font-size: var(--fs-sm);
    cursor: pointer;
  }
  .eff:hover { background: var(--raised); color: var(--text); }
  .eff:focus-within { outline: 2px solid var(--accent); outline-offset: 1px; }
  .eff select {
    appearance: none;
    border: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    padding: 0;
    width: auto;
    min-height: 0;
    font-size: var(--fs-sm);
    cursor: pointer;
    outline: none;
  }
  .eff select option { color: var(--text); background: var(--panel); }
  .round {
    flex: none;
    width: 34px;
    height: 34px;
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
  .round:disabled { background: var(--raised); color: var(--faint); opacity: 1; cursor: default; }
  .round.stop { background: var(--text); color: var(--bg); }
  .round :global(.spin) { animation: spin 1s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .thumbs { display: flex; gap: var(--s2); flex-wrap: wrap; align-items: center; padding: var(--s3) var(--s3) 0; }
  .thumb { position: relative; border: 1px solid var(--line-strong); border-radius: var(--r); background: var(--bg); }
  .thumb img { height: 56px; display: block; border-radius: var(--r); }
  .remove {
    position: absolute;
    top: -8px;
    right: -8px;
    width: 22px;
    height: 22px;
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

  /* Téléphone : carte collée en bas, mêmes éléments, cibles tactiles */
  .composer.phone {
    padding: var(--s2) calc(var(--s2) + var(--safe-right)) calc(var(--s2) + var(--safe-bottom)) calc(var(--s2) + var(--safe-left));
    border-top: 1px solid var(--line);
    background: var(--panel);
  }
  .phone .box { border-radius: 18px; background: var(--bg); }
  .phone textarea { min-height: 44px; padding: var(--s3) var(--s4) var(--s1); }
  .phone .round { width: 40px; height: 40px; }
  .phone .eff { height: 40px; }
</style>
