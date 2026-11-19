<script lang="ts">
  // Thème et taille du texte : réglages du navigateur (localStorage), appliqués tout de suite.
  import Segmented from '../../lib/ui/Segmented.svelte';
  import { ui, setTheme, setSize, THEME_LABEL, SIZE_LABEL, type ThemeChoice, type TextSize } from '../../lib/ui/theme.svelte';
  import Section from './Section.svelte';

  const themes: ThemeChoice[] = ['system', 'light', 'dark'];
  const sizes: TextSize[] = ['s', 'm', 'l'];
  const px: Record<TextSize, number> = { s: 15, m: 16, l: 18 };
</script>

<Section title="Thème" text="Système suit le réglage clair ou sombre de l’appareil.">
  <Segmented label="Thème" size="lg" value={ui.theme} options={themes.map((t) => ({ value: t, label: THEME_LABEL[t] }))} onchange={setTheme} />
  <p class="hint">Mémorisé dans ce navigateur. Aussi dans la barre d’état.</p>
</Section>

<Section title="Taille du texte" text="Agrandit ou réduit toute l’interface, code compris.">
  <Segmented label="Taille du texte" size="lg" value={ui.size} options={sizes.map((s) => ({ value: s, label: SIZE_LABEL[s] }))} onchange={setSize} />
  <p class="hint">Texte de base : {px[ui.size]} px.</p>
  <p class="sample">Le texte des sessions et des réglages s’affiche à cette taille.</p>
  <pre class="code">cargo test -p enginex-server</pre>
</Section>

<style>
  .hint { margin: 0; color: var(--muted); font-size: var(--fs-sm); }
  .sample { margin: 0; }
  .code {
    margin: 0;
    padding: var(--s2) var(--s3);
    width: fit-content;
    max-width: 100%;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: var(--r);
    overflow-x: auto;
  }
</style>
