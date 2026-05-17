/**
 * @vitest-environment jsdom
 *
 * Test de propriété frontend — Property 33 : Overlay au survol.
 *
 * *For any* Media_Item rendu en `MediaCard`, le rendu de l'overlay de
 * survol contient la note utilisateur (si définie), la durée totale
 * (Films) ou le nombre d'épisodes (Séries / Animes), et le statut
 * courant.
 *
 * Méthodologie :
 *   - On construit `arbHoverItem` : un générateur fast-check qui produit
 *     un `Media_Item` arbitraire couvrant les deux variantes structurelles
 *     attendues par l'overlay :
 *       (a) Films          → `duration_seconds` toujours défini, `episode_count` null ;
 *       (b) Series / Animes → `episode_count` toujours défini, `duration_seconds` null.
 *     Le statut, la note utilisateur et les champs de base (id, title,
 *     year, genres) sont également générés aléatoirement.
 *   - Pour chaque item généré, on monte le composant `HoverOverlay`
 *     avec `isHovering: true` (équivalent direct du déclenchement de
 *     `mouseenter` côté parent — la prop est précisément le hook par
 *     lequel `MediaCard` reflète l'état hover dans l'overlay).
 *   - On vérifie alors :
 *       1. l'overlay (`data-testid="hover-overlay"`) existe et porte la
 *          classe `visible` ;
 *       2. la ligne statut (`hover-status`) est toujours présente ;
 *       3. si `user_rating != null`, la ligne note (`hover-rating`)
 *          contient le rating ;
 *       4. si `type === 'Film'`, la ligne durée (`hover-duration`) est
 *          présente ; sinon, la ligne épisodes (`hover-episodes`) est
 *          présente et contient le compteur.
 *
 * **Validates: Requirements 15.4**
 */

import { afterEach, describe, it, expect } from 'vitest';
import { cleanup, render } from '@testing-library/svelte';
import fc from 'fast-check';

import HoverOverlay from './HoverOverlay.svelte';
import type { MediaItem, MediaType, Status } from '$lib/api/types';

// ---------------------------------------------------------------------------
// Type local : `MediaItem` enrichi des champs optionnels que l'overlay
// consomme (note utilisateur, durée pour les Films, compteur d'épisodes
// pour les Séries / Animes). Aligné sur le `HoverOverlayItem` interne du
// composant.
// ---------------------------------------------------------------------------
type HoverOverlayItem = MediaItem & {
  user_rating: number | null;
  duration_seconds: number | null;
  episode_count: number | null;
};

// ---------------------------------------------------------------------------
// Générateurs fast-check
// ---------------------------------------------------------------------------

/** Statut de visionnage, aligné sur `Status`. */
const arbStatus: fc.Arbitrary<Status> = fc.constantFrom<Status>(
  'non_vu',
  'en_cours',
  'vu'
);

/** Année dans la plage spec [1900, 2099] (cf. requirement 4.2), ou null. */
const arbYear: fc.Arbitrary<number | null> = fc.option(
  fc.integer({ min: 1900, max: 2099 }),
  { nil: null }
);

/** Genres : liste courte de chaînes non vides. */
const arbGenres: fc.Arbitrary<string[]> = fc.array(
  fc.string({ minLength: 1, maxLength: 12 }),
  { maxLength: 4 }
);

/** Note utilisateur : entier [0, 10] ou null (cf. requirement 6.6). */
const arbUserRating: fc.Arbitrary<number | null> = fc.option(
  fc.integer({ min: 0, max: 10 }),
  { nil: null }
);

/**
 * Item Film — `duration_seconds` toujours défini (≥ 60s pour garantir
 * un `durationLabel` non vide côté composant), `episode_count` null.
 */
const arbFilm: fc.Arbitrary<HoverOverlayItem> = fc.record({
  id: fc.integer({ min: 1, max: 1_000_000 }),
  title: fc.string({ minLength: 1, maxLength: 60 }),
  original_title: fc.constant(null),
  year: arbYear,
  type: fc.constant<MediaType>('Film'),
  status: arbStatus,
  genres: arbGenres,
  user_rating: arbUserRating,
  duration_seconds: fc.integer({ min: 60, max: 14_400 }),
  episode_count: fc.constant(null)
});

/**
 * Item Série / Anime — `episode_count` toujours défini (≥ 1 pour
 * garantir un `episodeLabel` non vide côté composant),
 * `duration_seconds` null.
 */
const arbSeriesOrAnime: fc.Arbitrary<HoverOverlayItem> = fc.record({
  id: fc.integer({ min: 1, max: 1_000_000 }),
  title: fc.string({ minLength: 1, maxLength: 60 }),
  original_title: fc.constant(null),
  year: arbYear,
  type: fc.constantFrom<MediaType>('Series', 'Anime'),
  status: arbStatus,
  genres: arbGenres,
  user_rating: arbUserRating,
  episode_count: fc.integer({ min: 1, max: 500 }),
  duration_seconds: fc.constant(null)
});

/** Union des deux familles : un Media_Item arbitraire pour l'overlay. */
const arbHoverItem: fc.Arbitrary<HoverOverlayItem> = fc.oneof(
  arbFilm,
  arbSeriesOrAnime
);

// ---------------------------------------------------------------------------
// `@testing-library/svelte` impose un `cleanup` entre montages successifs
// pour éviter que les composants des itérations précédentes ne polluent
// le DOM (et donc les `getByTestId`). On nettoie après chaque test, et
// surtout après chaque itération à l'intérieur de la propriété.
// ---------------------------------------------------------------------------
afterEach(() => {
  cleanup();
});

// ---------------------------------------------------------------------------
// Property 33 — Overlay au survol
// ---------------------------------------------------------------------------

describe('Property 33 — Overlay au survol', () => {
  it("Property 33 : pour tout Media_Item, l'overlay rendu avec isHovering=true contient note (si définie), durée (Films) ou épisodes (Séries / Animes), et statut", () => {
    fc.assert(
      fc.property(arbHoverItem, (item) => {
        // Nettoyage explicite avant chaque itération : `afterEach` ne se
        // déclenche qu'entre tests, pas entre itérations de la propriété.
        cleanup();

        const { getByTestId, queryByTestId } = render(HoverOverlay, {
          props: { item, isHovering: true }
        });

        // 1. L'overlay existe et porte la classe `visible` (équivalent
        //    visuel du `mouseenter` propagé par la `MediaCard`).
        const overlay = getByTestId('hover-overlay');
        expect(overlay.classList.contains('visible')).toBe(true);

        // 2. Le statut est toujours rendu, quelle que soit la variante.
        const statusEl = getByTestId('hover-status');
        expect(statusEl).not.toBeNull();
        expect((statusEl.textContent ?? '').length).toBeGreaterThan(0);

        // 3. Note utilisateur : présente ssi `user_rating != null`, et
        //    contient alors la valeur numérique tirée par le générateur.
        if (item.user_rating != null) {
          const ratingEl = getByTestId('hover-rating');
          expect(ratingEl.textContent ?? '').toContain(String(item.user_rating));
        } else {
          expect(queryByTestId('hover-rating')).toBeNull();
        }

        // 4. Variante structurelle : Films → durée ; Series/Anime →
        //    nombre d'épisodes (avec son compteur).
        if (item.type === 'Film') {
          const durationEl = getByTestId('hover-duration');
          expect(durationEl).not.toBeNull();
          expect((durationEl.textContent ?? '').length).toBeGreaterThan(0);
          expect(queryByTestId('hover-episodes')).toBeNull();
        } else {
          const episodesEl = getByTestId('hover-episodes');
          expect(episodesEl).not.toBeNull();
          expect(episodesEl.textContent ?? '').toContain(
            String(item.episode_count)
          );
          expect(queryByTestId('hover-duration')).toBeNull();
        }
      }),
      { numRuns: 50 }
    );
  });
});
