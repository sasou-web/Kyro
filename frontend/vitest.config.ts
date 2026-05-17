/**
 * Configuration Vitest dédiée au projet Kyro.
 *
 * Étend la config Vite (qui charge `@sveltejs/kit/vite`) et ajoute
 * la résolution de la condition `browser` pour les modules Svelte.
 *
 * **Pourquoi `resolve.conditions = ['browser']` est nécessaire** :
 * Sans cette option, vite-node (utilisé par Vitest) résout les
 * `package.json` avec la condition `node`, ce qui charge la version
 * SSR du runtime Svelte (`svelte/internal/disclose-version` côté
 * serveur). Dans ce cas, `onMount()` est un no-op : les callbacks
 * passés ne sont jamais ajoutés au tableau `$$.on_mount` du
 * composant CSR rendu par `@testing-library/svelte`. Forcer la
 * condition `browser` aligne le runtime utilisé par vitest sur
 * celui consommé en production par la WebView Tauri.
 *
 * Cf. https://github.com/sveltejs/vite-plugin-svelte/blob/main/docs/faq.md#vitest
 */

import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  plugins: [sveltekit()],
  resolve: {
    conditions: ['browser']
  },
  test: {
    include: ['src/**/*.{test,spec}.{js,ts}'],
    // Les tests qui nécessitent un DOM activent `jsdom` via le
    // pragma `@vitest-environment jsdom` en tête de fichier — les
    // tests purs (logique store, helpers) tournent en `node` par
    // défaut, qui est plus rapide à initialiser.
    environment: 'node'
  }
});
