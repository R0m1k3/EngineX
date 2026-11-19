import { defineConfig, type Plugin } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { brotliCompressSync, gzipSync, constants } from 'node:zlib';
import { readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

// Fichiers texte du build compressés d'avance (`.br`, `.gz`) : le binaire
// les sert tels quels selon Accept-Encoding (environ 3 fois moins d'octets).
function precompress(): Plugin {
  let outDir = 'dist';
  const walk = (d: string): string[] =>
    readdirSync(d).flatMap((f) => {
      const p = join(d, f);
      return statSync(p).isDirectory() ? walk(p) : [p];
    });
  return {
    name: 'enginex-precompress',
    apply: 'build',
    configResolved(c) {
      outDir = c.build.outDir;
    },
    closeBundle() {
      for (const f of walk(outDir)) {
        if (!/\.(js|css|html|svg|json|txt|map)$/.test(f)) continue;
        const data = readFileSync(f);
        if (data.length < 1024) continue;
        writeFileSync(
          `${f}.br`,
          brotliCompressSync(data, { params: { [constants.BROTLI_PARAM_QUALITY]: 11 } }),
        );
        writeFileSync(`${f}.gz`, gzipSync(data, { level: 9 }));
      }
    },
  };
}

// Le binaire Rust sert web/dist ; en développement, Vite sert l'UI et
// relaie /api vers le binaire.
export default defineConfig({
  plugins: [svelte(), precompress()],
  build: { target: 'es2022', assetsInlineLimit: 0 },
  server: { proxy: { '/api': { target: 'http://127.0.0.1:8090', ws: true } } },
});
