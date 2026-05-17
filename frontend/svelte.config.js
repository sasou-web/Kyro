import adapter from '@sveltejs/adapter-static';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

/**
 * Configuration SvelteKit pour Kyro.
 *
 * Mode SPA strict :
 *  - adapter-static avec fallback `index.html` (pas de SSR, pas de pré-rendu).
 *  - kit.prerender.entries = [] : aucune route n'est pré-rendue.
 *  - Le contrôle SSR/CSR/prerender par route est défini dans `src/routes/+layout.ts`.
 *
 * @type {import('@sveltejs/kit').Config}
 */
const config = {
  preprocess: vitePreprocess(),
  kit: {
    adapter: adapter({
      pages: 'build',
      assets: 'build',
      fallback: 'index.html',
      precompress: false,
      strict: true
    }),
    prerender: {
      entries: []
    }
  }
};

export default config;
