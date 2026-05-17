import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

/**
 * Configuration Vite pour Kyro.
 *
 * Le port 1420 et strictPort sont alignés avec `src-tauri/tauri.conf.json`
 * (build.devUrl = http://localhost:1420). Tauri échoue volontairement si le
 * port n'est pas exactement disponible, ce qui garantit qu'aucun autre
 * service n'est utilisé en cas de conflit.
 *
 * Aucune configuration de proxy ou de réseau sortant : conformément à
 * Requirements 17.3, le frontend ne doit pas émettre de requêtes vers
 * l'extérieur.
 */
export default defineConfig({
  plugins: [sveltekit()],
  clearScreen: false,
  server: {
    host: '127.0.0.1',
    port: 1420,
    strictPort: true,
    fs: {
      strict: true
    }
  },
  preview: {
    host: '127.0.0.1',
    port: 1420,
    strictPort: true
  },
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    target: 'es2022',
    sourcemap: true
  }
});
