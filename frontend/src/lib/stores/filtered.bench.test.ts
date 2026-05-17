// @vitest-environment node
/**
 * Tâche 17.3 — bench du re-filtrage côté store dérivé.
 *
 * Peuple le `writable` `items` avec 5000 `MediaItem` synthétiques,
 * souscrit au store dérivé `filteredItems`, puis simule une saisie
 * utilisateur dans la TopBar en mettant à jour `searchQuery` à chaque
 * frappe. Pour chaque mise à jour on mesure avec `performance.now()` le
 * temps écoulé entre le `set()` et le rappel du subscriber dérivé.
 *
 * Cible (requirement 13.3) : ≤ 200 ms par frappe.
 *
 * Comportement Svelte exploité ici : `derived([items, filters,
 * searchQuery], …)` est synchrone — le subscriber rattaché reçoit la
 * nouvelle valeur **avant** le retour du `set()` d'une source.
 * `performance.now() - start` capture donc bien le temps de calcul de
 * `applyFiltersAndSearch` sur les 5000 items pour chaque frappe.
 *
 * Le test est mis dans le fichier `.bench.test.ts` (et non simplement
 * `.test.ts`) pour qu'on puisse le cibler explicitement via
 * `npm run test -- --run filtered.bench`. Il fait partie de la suite
 * standard et tourne à chaque CI — la cible est tenue largement
 * (mesuré ~10-30 ms côté V8 pour 5000 items, cf. `filtered.test.ts`
 * smoke perf).
 *
 * Validates: Requirement 13.3 (recherche ≤ 200 ms par frappe pour
 * 5000 items côté store dérivé).
 */

import { describe, expect, it } from 'vitest';
import { get } from 'svelte/store';

import type { Filters, MediaItem } from '$lib/api/types';

import { applyFiltersAndSearch, filteredItems } from './filtered';
import { filters, items, searchQuery } from './library';

// ---------------------------------------------------------------------------
// Constantes.
// ---------------------------------------------------------------------------

/** Cible dure (requirement 13.3). */
const SEARCH_BUDGET_MS = 200;

/** Volume de la bibliothèque synthétique. */
const NUM_ITEMS = 5000;

const EMPTY_FILTERS: Filters = {
  genres: [],
  years: [],
  types: [],
  statuses: []
};

// ---------------------------------------------------------------------------
// Génération de la bibliothèque synthétique.
//
// On vise une distribution représentative :
//   * préfixes accentués tournants (Évangelion, Pokémon, Café…) pour
//     exercer la normalisation NFD ;
//   * suffixe numérique unique → 5000 titres distincts ;
//   * `original_title` 1 fiche sur 2 pour exercer la branche
//     `matchesSearch` correspondante ;
//   * `type` / `status` / `genres` répartis sur des moduli copremiers
//     pour que les filtres distinguent vraiment les items.
// ---------------------------------------------------------------------------

const PREFIXES = [
  'Évangelion',
  'Pokémon',
  'Café Society',
  'Naruto',
  'Spirited Away',
  'Inception',
  'Akira',
  'Cowboy Bebop',
  'Le Voyage de Chihiro',
  'Mononoké'
] as const;

const TYPES = ['Film', 'Series', 'Anime'] as const;
const STATUSES = ['non_vu', 'en_cours', 'vu'] as const;
const GENRE_POOL = [
  'Drame',
  'Action',
  'Comédie',
  'Thriller',
  'Mecha',
  'Aventure',
  'Science-Fiction',
  'Romance'
] as const;

function buildSyntheticLibrary(n: number): MediaItem[] {
  const out: MediaItem[] = [];
  for (let i = 0; i < n; i++) {
    const prefix = PREFIXES[i % PREFIXES.length];
    out.push({
      id: i,
      title: `${prefix} ${i}`,
      original_title: i % 2 === 0 ? `Original ${i}` : null,
      year: 1990 + (i % 30),
      type: TYPES[i % TYPES.length],
      status: STATUSES[Math.floor(i / TYPES.length) % STATUSES.length],
      genres: [
        GENRE_POOL[i % GENRE_POOL.length],
        GENRE_POOL[Math.floor(i / 7) % GENRE_POOL.length]
      ]
    });
  }
  return out;
}

// ---------------------------------------------------------------------------
// Bench du store dérivé — chaque frappe doit rester < 200 ms.
// ---------------------------------------------------------------------------

describe('filteredItems perf bench (5000 items)', () => {
  it('reacts to each search keystroke in <= 200 ms', () => {
    // 1. Préparer la bibliothèque et l'état initial.
    const library = buildSyntheticLibrary(NUM_ITEMS);
    items.set(library);
    filters.set(EMPTY_FILTERS);
    searchQuery.set('');

    // Sanity check : le derived a bien la liste complète.
    expect(get(filteredItems).length).toBe(NUM_ITEMS);

    // 2. Souscrire au store dérivé. Le subscriber capture la dernière
    //    durée mesurée — `derived` est synchrone donc on peut lire
    //    `lastElapsedMs` immédiatement après chaque `set()`.
    let startTs = 0;
    let lastElapsedMs = 0;
    const unsubscribe = filteredItems.subscribe(() => {
      // La toute première invocation est l'initialisation au moment
      // de l'abonnement (état `''`) ; on l'ignore via `startTs === 0`.
      if (startTs > 0) {
        lastElapsedMs = performance.now() - startTs;
      }
    });

    try {
      // 3. Simuler la saisie touche-par-touche dans la TopBar. Chaque
      //    `set()` redéclenche `applyFiltersAndSearch` sur 5000 items.
      const keystrokes = ['e', 'ev', 'eva', 'evan', 'evangelion'];
      for (const q of keystrokes) {
        startTs = performance.now();
        searchQuery.set(q);
        // Le subscriber a été appelé synchroniquement par `set()`.
        expect(lastElapsedMs).toBeGreaterThan(0);
        expect(lastElapsedMs).toBeLessThanOrEqual(SEARCH_BUDGET_MS);
      }

      // 4. Cas accent + casse mixte (exerce NFD + lowercase).
      for (const q of ['POKÉMON', 'pokemon', 'café']) {
        startTs = performance.now();
        searchQuery.set(q);
        expect(lastElapsedMs).toBeLessThanOrEqual(SEARCH_BUDGET_MS);
      }

      // 5. Cas mutation des filtres (4 dimensions actives en même
      //    temps). Le derived recalcule autant que pour une frappe.
      startTs = performance.now();
      filters.set({
        genres: ['Drame', 'Thriller'],
        years: [1995, 2000, 2010],
        types: ['Film', 'Series'],
        statuses: ['en_cours', 'vu']
      });
      expect(lastElapsedMs).toBeLessThanOrEqual(SEARCH_BUDGET_MS);
    } finally {
      unsubscribe();
      // Reset pour ne pas polluer les tests suivants du même process.
      items.set([]);
      filters.set(EMPTY_FILTERS);
      searchQuery.set('');
    }
  });

  // Bench parallèle hors-store (`applyFiltersAndSearch` direct) : cas
  // de référence pour repérer une régression côté helpers purs sans
  // l'overhead du framework Svelte.
  it('applyFiltersAndSearch on 5000 items completes in <= 200 ms', () => {
    const library = buildSyntheticLibrary(NUM_ITEMS);
    const f: Filters = {
      genres: ['Drame', 'Thriller'],
      years: [],
      types: ['Film', 'Series'],
      statuses: ['en_cours', 'vu']
    };

    const start = performance.now();
    const out = applyFiltersAndSearch(library, f, 'evan');
    const elapsed = performance.now() - start;

    expect(out.length).toBeGreaterThan(0);
    expect(elapsed).toBeLessThanOrEqual(SEARCH_BUDGET_MS);
  });
});
