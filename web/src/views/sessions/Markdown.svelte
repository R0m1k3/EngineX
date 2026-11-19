<script lang="ts" module>
  import DOMPurify from 'dompurify';
  import { Marked } from 'marked';

  // Liens du modèle : jamais dans l'onglet EngineX (un tour en cours serait
  // quitté, et un `#x` changerait de page). Externes → nouvel onglet ; les
  // autres (relatifs, ancres) n'ont pas de cible sûre : texte seul.
  DOMPurify.addHook('afterSanitizeAttributes', (node) => {
    if (node.tagName !== 'A') return;
    const href = node.getAttribute('href') ?? '';
    if (/^(https?:|mailto:)/i.test(href)) {
      node.setAttribute('target', '_blank');
      node.setAttribute('rel', 'noopener noreferrer');
    } else {
      node.removeAttribute('href');
    }
  });

  const esc = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
  // Icône « copier » (lucide copy), dessinée en SVG brut dans le HTML rendu.
  const COPY =
    '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect width="14" height="14" x="8" y="8" rx="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/></svg>';

  // Instance locale (le `marked` global des autres vues n'est pas touché) :
  // blocs de code avec un en-tête (nom de fichier ou langage) et un bouton copier.
  // Info de la clôture : « ts », « ts web/src/x.ts » ou « web/src/x.ts ».
  const md = new Marked({ gfm: true, breaks: false });
  md.use({
    renderer: {
      code({ text, lang }) {
        const info = (lang ?? '').trim().split(/\s+/).filter(Boolean);
        const file = info.find((w) => /[/.]/.test(w) && !/^\.+$/.test(w)) ?? '';
        const language = info.find((w) => w !== file) ?? '';
        const label = file || language || 'code';
        return (
          `<div class="cb"><div class="cb-h"><span class="cb-n">${esc(label)}</span>` +
          `<button type="button" class="cb-copy" data-copy aria-label="Copier le code" title="Copier">${COPY}</button></div>` +
          `<pre><code${language ? ` class="language-${esc(language)}"` : ''}>${esc(text)}</code></pre></div>`
        );
      },
    },
  });
</script>

<script lang="ts">
  import { toast } from '../../lib/ui/toast.svelte';

  let { text }: { text: string } = $props();
  // Texte du modèle : rendu Markdown, puis nettoyé (aucun script, aucun gestionnaire).
  let html = $derived(DOMPurify.sanitize(md.parse(text ?? '', { async: false }) as string));

  async function onClick(e: MouseEvent) {
    const btn = (e.target as HTMLElement).closest<HTMLButtonElement>('[data-copy]');
    if (!btn) return;
    const code = btn.closest('.cb')?.querySelector('pre')?.textContent ?? '';
    try {
      await navigator.clipboard.writeText(code);
      toast('Code copié.', 'ok', 2000);
    } catch {
      toast('Copie refusée par le navigateur.', 'error');
    }
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="md" onclick={onClick}>{@html html}</div>

<style>
  .md { overflow-wrap: anywhere; }
  .md :global(p) { margin: 0 0 var(--s3); }
  .md :global(> :last-child), .md :global(.cb:last-child) { margin-bottom: 0; }
  .md :global(h1), .md :global(h2), .md :global(h3), .md :global(h4) { font-size: var(--fs-md); margin: var(--s4) 0 var(--s2); }
  .md :global(h1) { font-size: var(--fs-lg); }
  .md :global(:not(pre) > code) { background: var(--raised); padding: 1px 5px; border-radius: 3px; font-size: 0.875em; }
  .md :global(pre) { margin: 0 0 var(--s3); padding: var(--s3) var(--s4); background: var(--bg); border: 1px solid var(--line); border-radius: var(--r); overflow: auto; line-height: 1.6; overflow-wrap: normal; }
  .md :global(pre code) { background: none; padding: 0; font-size: var(--fs-code); }
  .md :global(.cb) { border: 1px solid var(--line); border-radius: var(--r); overflow: hidden; margin: 0 0 var(--s3); }
  .md :global(.cb pre) { margin: 0; border: 0; border-radius: 0; }
  .md :global(.cb-h) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s2);
    height: 32px;
    padding: 0 var(--s1) 0 var(--s3);
    background: var(--panel);
    border-bottom: 1px solid var(--line);
    font-size: var(--fs-xs);
    color: var(--muted);
  }
  .md :global(.cb-n) { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: var(--mono); }
  .md :global(.cb-copy) {
    display: inline-grid;
    place-items: center;
    width: 28px;
    height: 28px;
    min-height: 0;
    padding: 0;
    border: 0;
    border-radius: var(--r);
    background: none;
    color: var(--faint);
    cursor: pointer;
    filter: none;
    flex: none;
  }
  .md :global(.cb-copy:hover) { background: var(--raised); color: var(--text); }
  .md :global(ul), .md :global(ol) { margin: 0 0 var(--s3); padding-left: 1.4em; }
  .md :global(li) { margin: 2px 0; }
  .md :global(table) { width: auto; margin: 0 0 var(--s3); }
  .md :global(blockquote) { margin: 0 0 var(--s3); padding-left: var(--s3); border-left: 2px solid var(--line-strong); color: var(--muted); }
  .md :global(hr) { border: 0; border-top: 1px solid var(--line); margin: var(--s4) 0; }
  .md :global(img) { max-width: 100%; }
  @media (max-width: 767px) {
    .md :global(.cb-copy) { width: var(--touch); height: var(--touch); }
    .md :global(.cb-h) { height: var(--touch); }
  }
</style>
