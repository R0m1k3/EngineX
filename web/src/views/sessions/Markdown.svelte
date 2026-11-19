<script lang="ts" module>
  import DOMPurify from 'dompurify';

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
</script>

<script lang="ts">
  import { marked } from 'marked';

  let { text }: { text: string } = $props();
  // Texte du modèle : rendu Markdown, puis nettoyé (aucun script, aucun gestionnaire).
  let html = $derived(DOMPurify.sanitize(marked.parse(text ?? '', { async: false, gfm: true, breaks: false }) as string));
</script>

<div class="md">{@html html}</div>

<style>
  .md :global(p) { margin: 0 0 var(--s2); }
  .md :global(p:last-child) { margin-bottom: 0; }
  .md :global(pre) {
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: var(--s3);
    overflow: auto;
    font-size: var(--fs-sm);
  }
  .md :global(code) { background: var(--bg); padding: 0 3px; border-radius: 3px; }
  .md :global(pre code) { background: none; padding: 0; }
  .md :global(ul), .md :global(ol) { margin: var(--s1) 0 var(--s2); padding-left: var(--s6); }
  .md :global(table) { width: auto; }
  .md :global(blockquote) { margin: 0; padding-left: var(--s3); border-left: 3px solid var(--line); color: var(--muted); }
</style>
