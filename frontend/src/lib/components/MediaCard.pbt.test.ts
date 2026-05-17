/**
 * @vitest-environment jsdom
 *
 * Test de propriété frontend — Property 32 : Carte d'item contient
 * affiche / titre / année.
 *
 * Pour tout `MediaItem` (élargi avec `poster_path`), le rendu d'une
 * `MediaCard` via `@testing-library/svelte` doit exposer dans le DOM :
 *   (a) une affiche, soit comme `<img>` (cas `poster_path` non vide),
 *       soit comme un visuel par défaut inline `<svg role="img">`
 *       (cas `poster_path` absent / vide / null) ;
 *   (b) le titre du Media_Item, accessible via le contenu textuel de
 *       la carte ;
 *   (c) l'année (au format chaîne) si elle est définie, idem.
 *
 * Le test complète `MediaCard.test.ts` qui couvre des cas statiques.
 * Ici, on génère via fast-check un large échantillon de `MediaItem`
 * pour s'assurer que les trois invariants tiennent quelles que soient
 * les valeurs des champs (titres unicode, années aux bornes [1900,
 * 2099], type / statut variés, genres, poster_path présent ou non).
 *
 * **Validates: Requirements 15.1, 15.2**
 */

import { afterEach, describe, it } from 'vitest';
import { cleanup, render } from '@testing-library/svelte';
import fc from 'fast-check';

import MediaCard from './MediaCard.svelte';
import type { MediaItem, MediaType, Status } from '$lib/api/types';

// Type structurel attendu par le composant : `MediaItem` plus
// `poster_path` optionnel (cf. `MediaCard.svelte`).
type MediaCardItem = MediaItem & { poster_path?: string | null };

// On nettoie systématiquement le DOM après chaque itération de la
// propriété pour éviter que les rendus successifs ne s'accumulent
// dans `document.body` et que les `querySelectorAll` ne mesurent
// l'union de toutes les cartes rendues.
afterEach(() => {
  cleanup();
});

// ---------------------------------------------------------------------------
// Générateurs fast-check.
//
// Les bornes sont alignées sur le DTO Rust (`MediaItem`) :
//  - `id` : entier positif (auto-incrément SQLite, on borne à 1e6
//           pour que les valeurs restent lisibles dans les rapports
//           de contre-exemples) ;
//  - `title` : chaîne unicode non vide après trim (le composant
//              s'attend à un titre affichable) ;
//  - `year`  : optionnel, dans [1900, 2099] (cf. requirement 4.2) ;
//  - `type` / `status` : valeurs littérales énumérées ;
//  - `genres` : liste courte de chaînes non vides ;
//  - `poster_path` : optionnel ; non vide quand présent.
// ---------------------------------------------------------------------------

const arbMediaType: fc.Arbitrary<MediaType> = fc.constantFrom<MediaType>(
  'Film',
  'Series',
  'Anime'
);

const arbStatus: fc.Arbitrary<Status> = fc.constantFrom<Status>(
  'non_vu',
  'en_cours',
  'vu'
);

const arbMediaItem: fc.Arbitrary<MediaCardItem> = fc.record({
  id: fc.integer({ min: 1, max: 1_000_000 }),
  title: fc
    .string({ minLength: 1, maxLength: 80 })
    .filter((s) => s.trim().length > 0),
  original_title: fc.option(fc.string({ maxLength: 80 }), { nil: null }),
  year: fc.option(fc.integer({ min: 1900, max: 2099 }), { nil: null }),
  type: arbMediaType,
  status: arbStatus,
  genres: fc.array(fc.string({ minLength: 1, maxLength: 12 }), {
    maxLength: 5
  }),
  poster_path: fc.option(fc.string({ minLength: 1, maxLength: 30 }), {
    nil: null
  })
});

// ---------------------------------------------------------------------------
// Propriété.
// ---------------------------------------------------------------------------

describe("Property 32 — Carte d'item contient affiche / titre / année", () => {
  it(
    "Property 32 : pour tout MediaItem, <MediaCard> expose une affiche " +
      "(img ou svg role=img), le titre, et l'année si définie " +
      '(Validates Requirements 15.1, 15.2)',
    () => {
      fc.assert(
        fc.property(arbMediaItem, (item) => {
          // Rendu isolé du composant. `cleanup()` (afterEach) garantit
          // qu'aucune carte précédente ne pollue les requêtes du DOM.
          const { container } = render(MediaCard, { props: { item } });

          // (a) Affiche : exactement un élément porteur d'image.
          //
          // Branches de `MediaCard.svelte` :
          //  - `poster_path` non vide  → un `<img>` (avec src tauri://) ;
          //  - sinon                   → un `<svg role="img">` inline.
          // On exige donc *exactement un* élément image-porteur, en
          // acceptant les deux formes (cf. notes du task : tolérer
          // l'inline SVG comme « visuel par défaut intégré »).
          const imgElements = container.querySelectorAll('img');
          const inlineImageSvgs = container.querySelectorAll(
            'svg[role="img"]'
          );
          const imageBearingCount =
            imgElements.length + inlineImageSvgs.length;

          if (imageBearingCount !== 1) {
            // On retourne `false` plutôt qu'on ne lance une erreur :
            // fast-check sait alors shrinker proprement et reporter le
            // contre-exemple (item) sans bruit de stack-trace.
            return false;
          }

          // Cohérence avec la branche du composant : `<img>` ssi
          // `poster_path` est une chaîne non vide.
          const hasPoster =
            typeof item.poster_path === 'string' &&
            item.poster_path.length > 0;
          if (hasPoster) {
            if (imgElements.length !== 1) return false;
            if (inlineImageSvgs.length !== 0) return false;
          } else {
            if (imgElements.length !== 0) return false;
            if (inlineImageSvgs.length !== 1) return false;
          }

          // (b) Titre : présent dans le contenu textuel de la carte.
          //    Svelte rend `{item.title}` dans un nœud texte ; le
          //    `textContent` du conteneur contient donc la chaîne
          //    littérale (sans échappement HTML), ce qui rend la
          //    vérification stable même pour les titres unicode ou
          //    contenant des caractères spéciaux (`<`, `&`, ...).
          const text = container.textContent ?? '';
          if (!text.includes(item.title)) {
            return false;
          }

          // (c) Année : si définie, la chaîne décimale doit apparaître.
          //    Le composant rend `<span class="year">{item.year}</span>`
          //    sous condition `item.year != null`.
          if (item.year != null) {
            if (!text.includes(String(item.year))) {
              return false;
            }
          } else {
            // Vérification structurelle complémentaire : pas d'élément
            // `.year` quand l'année n'est pas définie (cf. requirement
            // 15.1 — l'année n'est affichée que si connue).
            if (container.querySelector('.year') !== null) {
              return false;
            }
          }

          return true;
        }),
        // Le rendu Svelte + jsdom est lourd (création / destruction
        // d'une carte par itération) ; 50 runs offrent une couverture
        // confortable sans rendre la suite excessive.
        { numRuns: 50 }
      );
    }
  );
});
