<script lang="ts">
  // Marque EngineX : un cadre (l'atelier) et deux traits croisés (le X), trait de 1,75 comme les icônes.
  let { size = 24 }: { size?: number } = $props();
</script>

<svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  <rect x="3" y="3" width="18" height="18" rx="3" />
  <path d="M7.5 8h4M7.5 12h3M7.5 16h4M13.5 8l3.5 8M17 8l-3.5 8" />
</svg>
