<script lang="ts">
  import { uploadAttachment, message, type Attachment } from '../../lib/api';

  let {
    sessionId,
    running,
    disabled = false,
    send,
    interrupt,
    text = $bindable(''),
  }: {
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
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      submit();
    }
  }
</script>

<div
  class="composer"
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
          <button class="remove" title="Retirer" aria-label="Retirer l’image" onclick={() => images.splice(i, 1)}>×</button>
        </div>
      {/each}
      {#if uploading}<span class="muted small">Envoi de l’image…</span>{/if}
    </div>
  {/if}
  <textarea
    bind:value={text}
    onkeydown={onKey}
    onpaste={onPaste}
    rows="3"
    placeholder="Message à Codex (Entrée pour envoyer, Maj+Entrée pour aller à la ligne)"
  ></textarea>
  <div class="row controls">
    <div class="actions">
      <button class="secondary" onclick={() => picker?.click()} disabled={disabled}>Joindre une image</button>
      <input
        bind:this={picker}
        type="file"
        accept="image/png,image/jpeg,image/webp,image/gif"
        multiple
        hidden
        onchange={(e) => {
          const input = e.currentTarget as HTMLInputElement;
          add([...(input.files ?? [])]);
          input.value = '';
        }}
      />
      {#if error}<span class="error small">{error}</span>{/if}
    </div>
    {#if running}
      <button class="secondary" onclick={interrupt}>Interrompre</button>
    {:else}
      <button onclick={submit} disabled={disabled || sending || uploading > 0 || (!text.trim() && !images.length)}>Envoyer</button>
    {/if}
  </div>
</div>

<style>
  .composer {
    flex-shrink: 0;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    background: var(--panel);
    padding: var(--s2);
    display: flex;
    flex-direction: column;
    gap: var(--s2);
  }
  .composer.dragging { border-color: var(--accent); }
  textarea {
    resize: vertical;
    min-height: 60px;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: var(--s2) var(--s3);
    font: inherit;
  }
  textarea:focus { outline: 2px solid var(--accent); outline-offset: -1px; }
  .controls { gap: var(--s3); }
  .thumbs { display: flex; gap: var(--s2); flex-wrap: wrap; align-items: center; }
  .thumb { position: relative; }
  .thumb img { height: 64px; border-radius: 4px; display: block; }
  .remove {
    position: absolute;
    top: -6px;
    right: -6px;
    width: 20px;
    height: 20px;
    padding: 0;
    border-radius: 50%;
    line-height: 20px;
    background: var(--line);
    color: var(--text);
  }
</style>
