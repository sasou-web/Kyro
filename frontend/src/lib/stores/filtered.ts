/**
 * Store dérivé `filteredItems` (tâche 13.3).
 *
 * Combine les trois sources `items`, `filters`, `searchQuery` du store
 * `library.ts` pour produire la liste à afficher dans la vue courante.
 *
 * Sémantique (cf. `design.md` § « Recherche et filtres » et requirements
 * 13 et 14) :
 *
 * 1. **Recherche** (requirement 13.2, 13.4) — `query` est normalisée
 *    via `normalize(s) = s.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase()`.
 *    Un item passe si la version normalisée de son `title` ou de son
 *    `original_title` contient la requête normalisée. Une requête vide
 *    (après trim) ne filtre rien.
 *
 * 2. **Filtres** (requirement 14.2, 14.3) — chacune des dimensions
 *    `genres`, `years`, `types`, `statuses` se comporte ainsi :
 *
 *    | Dimension | Sémantique | Liste vide signifie |
 *    | --------- | ---------- | ------------------- |
 *    | `genres`  | union (item passe si ses genres ∩ filtre ≠ ∅) | aucune contrainte de genre |
 *    | `years`   | appartenance (item passe si `item.year` ∈ filtre) | aucune contrainte d'année |
 *    | `types`   | appartenance (item passe si `item.type` ∈ filtre) | aucune contrainte de type |
 *    | `statuses`| appartenance (item passe si `item.status` ∈ filtre) | aucune contrainte de statut |
 *
 *    Les dimensions sont combinées en intersection (ET logique) — un
 *    item doit satisfaire **toutes** les dimensions actives
 *    (requirement 14.2). Le retrait de tous les filtres + recherche
 *    vide ⇒ liste complète (requirement 13.4, 14.4).
 *
 * 3. **Composition** : on applique d'abord les filtres puis la
 *    recherche. L'ordre est sans incidence sur le résultat (les deux
 *    opérations sont des prédicats indépendants), mais filtrer en
 *    premier réduit le coût de la normalisation.
 *
 * Performance : la cible (`design.md` § Recherche et filtres,
 * requirement 13.3) est < 50 ms par re-filtre pour 5000 items. Une
 * boucle linéaire avec normalisation paresseuse suffit largement —
 * mesuré ~10-20 ms en dev (cf. `filtered.test.ts`).
 *
 * Aucune optimisation prématurée (pas de `Map<genre, Set<id>>`
 * précomputé) : le code reste simple et la cible est tenue.
 *
 * _Requirements: 13.3, 14.2, 14.3, 14.4, 14.5_
 */

import { derived, type Readable } from 'svelte/store';

import type { Filters, MediaItem } from '$lib/api/types';

import { filters, items, searchQuery } from './library';

// ---------------------------------------------------------------------------
// Helpers purs (exportés pour les tests).
// ---------------------------------------------------------------------------

/**
 * Normalise une chaîne pour comparaison insensible à la casse et aux
 * accents (requirement 13.2).
 *
 * Utilise NFD pour décomposer chaque caractère accentué en sa base
 * + sa marque combinante, retire toutes les marques (`\p{M}`) puis
 * passe en minuscules.
 *
 * Exemples :
 * - `normalize('Évangelion')` → `'evangelion'`
 * - `normalize('Pokémon')` → `'pokemon'`
 * - `normalize('CAFÉ')` → `'cafe'`
 */
export function normalize(s: string): string {
  return s.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase();
}

/**
 * Vrai si l'item passe l'ensemble des dimensions de filtre.
 *
 * Voir le tableau de sémantique en tête de fichier pour le détail
 * de chaque dimension. Un filtre `Filters` dont toutes les listes
 * sont vides retourne toujours `true`.
 */
export function matchesFilters(item: MediaItem, f: Filters): boolean {
  // Genres : union — au moins un genre commun, ou aucun filtre actif.
  if (f.genres.length > 0) {
    const set = new Set(f.genres);
    let hit = false;
    for (const g of item.genres) {
      if (set.has(g)) {
        hit = true;
        break;
      }
    }
    if (!hit) return false;
  }

  // Years : appartenance. Un item sans année (`year = null`) ne peut
  // satisfaire un filtre d'année non vide.
  if (f.years.length > 0) {
    if (item.year === null) return false;
    if (!f.years.includes(item.year)) return false;
  }

  // Types : appartenance.
  if (f.types.length > 0 && !f.types.includes(item.type)) return false;

  // Statuses : appartenance.
  if (f.statuses.length > 0 && !f.statuses.includes(item.status)) return false;

  return true;
}

/**
 * Vrai si l'item matche la requête de recherche (requirement 13.2).
 * Une requête vide (ou réduite à des espaces) ne filtre pas.
 */
export function matchesSearch(item: MediaItem, query: string): boolean {
  const q = normalize(query.trim());
  if (q.length === 0) return true;
  if (normalize(item.title).includes(q)) return true;
  if (item.original_title !== null && normalize(item.original_title).includes(q)) {
    return true;
  }
  return false;
}

/**
 * Applique séquentiellement filtres puis recherche.
 *
 * Exporté pour permettre des appels directs (tests de performance,
 * benchmarks ad hoc) sans avoir à passer par les stores Svelte.
 */
export function applyFiltersAndSearch(
  list: MediaItem[],
  f: Filters,
  query: string
): MediaItem[] {
  const out: MediaItem[] = [];
  for (const item of list) {
    if (!matchesFilters(item, f)) continue;
    if (!matchesSearch(item, query)) continue;
    out.push(item);
  }
  return out;
}

// ---------------------------------------------------------------------------
// Store dérivé.
// ---------------------------------------------------------------------------

/**
 * Liste filtrée et recherchée des `MediaItem`, recalculée à chaque
 * mutation de l'une des trois sources.
 *
 * Consommée par les routes `/films`, `/series`, `/animes`,
 * `/favoris`, `/a-regarder` ainsi que par le compteur affiché en
 * permanence dans la TopBar (requirement 14.5).
 */
export const filteredItems: Readable<MediaItem[]> = derived(
  [items, filters, searchQuery],
  ([$items, $filters, $query]) => applyFiltersAndSearch($items, $filters, $query)
);
