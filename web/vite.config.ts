import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Le binaire Rust sert web/dist ; en développement, Vite sert l'UI et
// relaie /api vers le binaire.
export default defineConfig({
  plugins: [svelte()],
  build: { target: 'es2022', assetsInlineLimit: 0 },
  server: { proxy: { '/api': { target: 'http://127.0.0.1:8090', ws: true } } },
});
