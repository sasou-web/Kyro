/**
 * Tests unitaires du store dérivé `filteredItems` et de ses
 * helpers purs (`normalize`, `matchesFilters`, `matchesSearch`,
 * `applyFiltersAndSearch`).
 *
 * Couvre les requirements 13.2, 13.3, 13.4, 14.2, 14.3, 14.4.
 * Inclut un test de smoke perf — la cible interne (50 ms / 5000
 * items) est validée par un seuil large (200 ms) pour rester
 * fiable sur runners CI variés.
 */

import { describe, expect, it } from 'vitest';
import { get } from 'svelte/store';

import type { Filters, MediaItem } from '$lib/api/types';

import { filters, items, searchQuery } from './library';
import {
  applyFiltersAndSearch,
  filteredItems,
  matchesFilters,
  matchesSearch,
  normalize
} from './filtered';

// ---------------------------------------------------------------------------
// Fixtures.
// ---------------------------------------------------------------------------

const EMPTY_FILTERS: Filters = {
  genres: [],
  years: [],
  types: [],
  statuses: []
};

function mkItem(overrides: Partial<MediaItem> & { id: number; title: string }): MediaItem {
  return {
    id: overrides.id,
    title: overrides.title,
    original_title: overrides.original_title ?? null,
    year: overrides.year ?? null,
    type: overrides.type ?? 'Film',
    status: overrides.status ?? 'non_vu',
    genres: overrides.genres ?? []
  };
}

const SAMPLE: MediaItem[] = [
  mkItem({
    id: 1,
    title: 'Évangelion',
    original_title: '新世紀エヴァンゲリオン',
    year: 1995,
    type: 'Anime',
    status: 'vu',
    genres: ['Mecha', 'Drame']
  }),
  mkItem({
    id: 2,
    title: 'Pokémon',
    original_title: 'ポケットモンスター',
    year: 1997,
    type: 'Anime',
    status: 'en_cours',
    genres: ['Aventure']
  }),
  mkItem({
    id: 3,
    title: 'Inception',
    year: 2010,
    type: 'Film',
    status: 'vu',
    genres: ['Science-Fiction', 'Thriller']
  }),
  mkItem({
    id: 4,
    title: 'Breaking Bad',
    year: 2008,
    type: 'Series',
    status: 'en_cours',
    genres: ['Drame', 'Thriller']
  }),
  mkItem({
    id: 5,
    title: 'Café Society',
    year: 2016,
    type: 'Film',
    status: 'non_vu',
    genres: ['Drame']
  })
];

// ---------------------------------------------------------------------------
// normalize().
// ---------------------------------------------------------------------------

describe('normalize', () => {
  it('strips diacritics and lowercases', () => {
    expect(normalize('Évangelion')).toBe('evangelion');
    expect(normalize('Pokémon')).toBe('pokemon');
    expect(normalize('CAFÉ')).toBe('cafe');
  });

  it('returns empty for empty input', () => {
    expect(normalize('')).toBe('');
  });

  it('preserves CJK ideograms (no combining marks involved)', () => {
    // Les idéogrammes Han n'ont pas de marques combinantes ; ils
    // restent intacts (la casse ne s'applique pas non plus).
    expect(normalize('新世紀')).toBe('新世紀');
  });

  it('NFD decomposition strips voiced kana marks (dakuten)', () => {
    // Comportement attendu de NFD : ヴ → ウ + ゙ (combining mark),
    // puis `\p{M}` retire le ゙. C'est documenté ici comme
    // contrat — les requêtes japonaises restent comparables car
    // titre et requête subissent la même normalisation.
    expect(normalize('ヴ')).toBe('ウ');
    expect(normalize('ゲ')).toBe('ケ');
  });
});

// ---------------------------------------------------------------------------
// matchesSearch() — requirement 13.2, 13.4.
// ---------------------------------------------------------------------------

describe('matchesSearch', () => {
  const item = SAMPLE[0]; // Évangelion / 新世紀エヴァンゲリオン

  it('empty query matches everything (req 13.4)', () => {
    expect(matchesSearch(item, '')).toBe(true);
    expect(matchesSearch(item, '   ')).toBe(true);
  });

  it('matches title case-insensitively', () => {
    expect(matchesSearch(item, 'EVANGELION')).toBe(true);
    expect(matchesSearch(item, 'evangelion')).toBe(true);
  });

  it('matches title accent-insensitively', () => {
    expect(matchesSearch(item, 'Evangelion')).toBe(true);
    expect(matchesSearch(item, 'évangelion')).toBe(true);
  });

  it('matches original_title when title does not contain query', () => {
    expect(matchesSearch(item, 'エヴァ')).toBe(true);
  });

  it('returns false when neither title nor original_title contain query', () => {
    expect(matchesSearch(item, 'naruto')).toBe(false);
  });

  it('matches partial substrings', () => {
    expect(matchesSearch(item, 'evan')).toBe(true);
  });

  it('safely handles items with null original_title', () => {
    const it3 = SAMPLE[2]; // Inception, original_title null
    expect(matchesSearch(it3, 'inception')).toBe(true);
    expect(matchesSearch(it3, 'matrix')).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// matchesFilters() — requirement 14.2, 14.3, 14.4.
// ---------------------------------------------------------------------------

describe('matchesFilters', () => {
  it('empty filters match every item (req 14.4)', () => {
    for (const item of SAMPLE) {
      expect(matchesFilters(item, EMPTY_FILTERS)).toBe(true);
    }
  });

  describe('genres (union — req 14.3)', () => {
    it('matches if any genre intersects', () => {
      const f: Filters = { ...EMPTY_FILTERS, genres: ['Drame'] };
      const out = SAMPLE.filter((i) => matchesFilters(i, f));
      expect(out.map((i) => i.id).sort()).toEqual([1, 4, 5]);
    });

    it('union of two genres', () => {
      const f: Filters = { ...EMPTY_FILTERS, genres: ['Mecha', 'Aventure'] };
      const out = SAMPLE.filter((i) => matchesFilters(i, f));
      expect(out.map((i) => i.id).sort()).toEqual([1, 2]);
    });

    it('genre absent from all items yields empty result', () => {
      const f: Filters = { ...EMPTY_FILTERS, genres: ['Western'] };
      expect(SAMPLE.filter((i) => matchesFilters(i, f))).toEqual([]);
    });
  });

  describe('years (membership)', () => {
    it('keeps only items whose year is listed', () => {
      const f: Filters = { ...EMPTY_FILTERS, years: [1995, 2010] };
      const out = SAMPLE.filter((i) => matchesFilters(i, f));
      expect(out.map((i) => i.id).sort()).toEqual([1, 3]);
    });

    it('items with null year are excluded when filter is non-empty', () => {
      const orphan = mkItem({ id: 99, title: 'Unknown', year: null });
      const f: Filters = { ...EMPTY_FILTERS, years: [2010] };
      expect(matchesFilters(orphan, f)).toBe(false);
    });
  });

  describe('types (membership)', () => {
    it('keeps only matching media types', () => {
      const f: Filters = { ...EMPTY_FILTERS, types: ['Film'] };
      const out = SAMPLE.filter((i) => matchesFilters(i, f));
      expect(out.map((i) => i.id).sort()).toEqual([3, 5]);
    });
  });

  describe('statuses (membership)', () => {
    it('keeps only matching statuses', () => {
      const f: Filters = { ...EMPTY_FILTERS, statuses: ['vu'] };
      const out = SAMPLE.filter((i) => matchesFilters(i, f));
      expect(out.map((i) => i.id).sort()).toEqual([1, 3]);
    });
  });

  describe('combined dimensions (intersection — req 14.2)', () => {
    it('all dimensions must be satisfied simultaneously', () => {
      const f: Filters = {
        genres: ['Drame'],
        years: [],
        types: ['Series', 'Anime'],
        statuses: ['en_cours']
      };
      const out = SAMPLE.filter((i) => matchesFilters(i, f));
      // id 4 : Breaking Bad — Drame, Series, en_cours.
      // id 1 : Évangelion — Drame mais status=vu ⇒ exclu.
      expect(out.map((i) => i.id)).toEqual([4]);
    });
  });
});

// ---------------------------------------------------------------------------
// applyFiltersAndSearch() — composition.
// ---------------------------------------------------------------------------

describe('applyFiltersAndSearch', () => {
  it('empty filters + empty query returns the full list (req 13.4 + 14.4)', () => {
    expect(applyFiltersAndSearch(SAMPLE, EMPTY_FILTERS, '')).toEqual(SAMPLE);
  });

  it('preserves input order', () => {
    const out = applyFiltersAndSearch(SAMPLE, EMPTY_FILTERS, '');
    expect(out.map((i) => i.id)).toEqual([1, 2, 3, 4, 5]);
  });

  it('combines filter and search predicates with AND', () => {
    const f: Filters = { ...EMPTY_FILTERS, types: ['Anime'] };
    const out = applyFiltersAndSearch(SAMPLE, f, 'poke');
    expect(out.map((i) => i.id)).toEqual([2]);
  });

  it('returns empty when filter excludes the search hit', () => {
    const f: Filters = { ...EMPTY_FILTERS, types: ['Film'] };
    const out = applyFiltersAndSearch(SAMPLE, f, 'pokemon');
    expect(out).toEqual([]);
  });

  it('search alone honours accents (req 13.2)', () => {
    const out = applyFiltersAndSearch(SAMPLE, EMPTY_FILTERS, 'cafe');
    expect(out.map((i) => i.id)).toEqual([5]);
  });
});

// ---------------------------------------------------------------------------
// filteredItems store wiring.
// ---------------------------------------------------------------------------

describe('filteredItems store', () => {
  it('reflects changes to items, filters, and searchQuery', () => {
    items.set(SAMPLE);
    filters.set(EMPTY_FILTERS);
    searchQuery.set('');
    expect(get(filteredItems).map((i) => i.id)).toEqual([1, 2, 3, 4, 5]);

    searchQuery.set('drame'); // matches no titles, only filters by name
    expect(get(filteredItems)).toEqual([]);

    searchQuery.set('');
    filters.set({ ...EMPTY_FILTERS, statuses: ['en_cours'] });
    expect(get(filteredItems).map((i) => i.id).sort()).toEqual([2, 4]);

    // Reset for downstream tests.
    items.set([]);
    filters.set(EMPTY_FILTERS);
    searchQuery.set('');
  });

  it('returns empty array when items is empty', () => {
    items.set([]);
    filters.set(EMPTY_FILTERS);
    searchQuery.set('anything');
    expect(get(filteredItems)).toEqual([]);
  });
});

// ---------------------------------------------------------------------------
// Performance smoke test — req 13.3.
// ---------------------------------------------------------------------------

describe('performance', () => {
  it('filters 5000 items in well under 200 ms (target 50 ms)', () => {
    const types = ['Film', 'Series', 'Anime'] as const;
    const statuses = ['non_vu', 'en_cours', 'vu'] as const;
    const genrePool = ['Drame', 'Action', 'Comédie', 'Thriller', 'Mecha', 'Aventure'];

    const big: MediaItem[] = [];
    for (let i = 0; i < 5000; i++) {
      // Coprime moduli so type / status / genre pick independently
      // and items spread across all combinations.
      big.push(
        mkItem({
          id: i,
          title: `Title ${i} Évangelion-ish`,
          original_title: i % 2 === 0 ? `Origin ${i}` : null,
          year: 1990 + (i % 30),
          type: types[i % 3],
          status: statuses[Math.floor(i / 3) % 3],
          genres: [
            genrePool[i % genrePool.length],
            genrePool[Math.floor(i / 7) % genrePool.length]
          ]
        })
      );
    }

    const f: Filters = {
      genres: ['Drame', 'Thriller'],
      years: [],
      types: ['Film', 'Series'],
      statuses: ['en_cours', 'vu']
    };

    const start = performance.now();
    const out = applyFiltersAndSearch(big, f, 'evan');
    const elapsed = performance.now() - start;

    expect(out.length).toBeGreaterThan(0);
    expect(elapsed).toBeLessThan(200);
  });
});
