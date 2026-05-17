/**
 * Stores Svelte de la bibliothèque (tâche 13.2).
 *
 * Centralise l'état applicatif observé par les vues (`/`, `/edit`,
 * `/settings`, `/logs`) et les composants partagés (`<TopBar>`,
 * `<Sidebar>`, …). Les stores sont des `writable` minimalistes — la
 * logique métier (filtrage, dérivation des compteurs, persistance)
 * sera ajoutée par les tâches dérivées (13.3, 14.x).
 *
 * Les stores listés ici matchent exactement le contrat décrit dans
 * `design.md` § « State management » :
 *
 * | Store          | Type                                                       | Source des mises à jour                  |
 * | -------------- | ---------------------------------------------------------- | ----------------------------------------- |
 * | `items`        | `MediaItem[]`                                              | Commandes IPC `list_by_category`, etc.    |
 * | `episodes`     | `Map<MediaItemId, EpisodeItem[]>`                          | Commande `list_episodes(seriesId)`        |
 * | `filters`      | `Filters` (genres / years / types / statuses)              | UI (`<FilterChip>`)                       |
 * | `searchQuery`  | `string`                                                   | UI (`<TopBar>` zone de recherche)         |
 * | `settings`     | `SettingsSummary \| null`                                  | Commande `get_settings_summary` au boot   |
 * | `scanProgress` | `ScanProgress \| null`                                     | Événement Tauri `scan-progress` (13.2)    |
 * | `flags`        | `{ ffprobeMissing: boolean; backendDegraded: boolean }`    | Événements `ffprobe-missing` / `backend-degraded` |
 *
 * Note de nommage : le store de recherche est exporté sous le nom
 * `searchQuery` plutôt que `search` pour ne pas entrer en collision
 * avec la commande `search()` de `$lib/api/commands` lorsque les deux
 * sont importées dans un même module (`import { search } from '$lib/api/commands'`).
 *
 * _Requirements: 3.6, 5.4, 18.4_
 */

import { writable, type Writable } from 'svelte/store';

import type {
  EpisodeItem,
  Filters,
  MediaItem,
  ScanProgress,
  SettingsSummary
} from '$lib/api/types';

// ---------------------------------------------------------------------------
// Items et épisodes — données primaires de la bibliothèque.
// ---------------------------------------------------------------------------

/**
 * Liste plate des Media_Item de la catégorie courante, telle que
 * retournée par la dernière commande de listing (`list_by_category`,
 * `search`, `list_favorites`, `list_to_watch`).
 *
 * Les composants `<MediaCard>` rendent depuis ce store. La dérivation
 * filtrée (recherche + filtres) est branchée plus tard
 * (cf. `design.md` § « Recherche et filtres »).
 */
export const items: Writable<MediaItem[]> = writable<MediaItem[]>([]);

/**
 * Cache `seriesId → EpisodeItem[]` peuplé à la volée par les vues
 * détail / lecture (commande `list_episodes`). On utilise un `Map`
 * plutôt qu'un objet pour conserver l'ordre d'insertion et offrir un
 * accès O(1) sans risque de collision avec un nom de propriété.
 *
 * Le type de la clé est `number` — c.-à-d. `MediaItemId` côté Rust,
 * qui est un `i64` projeté en `number` JS (cf. doc `MediaItem`).
 */
export const episodes: Writable<Map<number, EpisodeItem[]>> = writable<
  Map<number, EpisodeItem[]>
>(new Map());

// ---------------------------------------------------------------------------
// Recherche et filtres.
// ---------------------------------------------------------------------------

/**
 * Critères de filtrage actifs. Une liste vide signifie « aucune
 * contrainte » sur la dimension correspondante (cf. `Filters`).
 *
 * Valeur par défaut : aucun filtre actif sur les quatre dimensions.
 */
export const filters: Writable<Filters> = writable<Filters>({
  genres: [],
  years: [],
  types: [],
  statuses: []
});

/**
 * Texte de recherche libre saisi dans la TopBar. La normalisation
 * Unicode (NFD + retrait des `\p{M}` + lowercase locale) est
 * appliquée par le store dérivé `filteredItems` plus tard, pas ici.
 */
export const searchQuery: Writable<string> = writable<string>('');

// ---------------------------------------------------------------------------
// Settings et scan.
// ---------------------------------------------------------------------------

/**
 * Vue agrégée des paramètres persistés ; chargée au boot via
 * `get_settings_summary`. `null` tant que la première lecture n'a pas
 * abouti — les composants paramètres / TopBar doivent gérer cet état
 * de chargement (afficher un placeholder ou désactiver les actions).
 */
export const settings: Writable<SettingsSummary | null> =
  writable<SettingsSummary | null>(null);

/**
 * Progression du scan en cours, mise à jour par chaque événement
 * `scan-progress`. `null` quand aucun scan n'est actif (état initial,
 * ou réinitialisation après réception de `scan-completed`).
 */
export const scanProgress: Writable<ScanProgress | null> =
  writable<ScanProgress | null>(null);

// ---------------------------------------------------------------------------
// Drapeaux de mode dégradé.
// ---------------------------------------------------------------------------

/**
 * Drapeaux globaux exposés à la TopBar pour afficher des bandeaux
 * persistants :
 *
 * * `ffprobeMissing` — passé à `true` à la réception de l'événement
 *   `ffprobe-missing` (requirement 5.4). Reste affiché tant que
 *   l'utilisateur n'a pas configuré un chemin valide dans
 *   `/settings`.
 * * `backendDegraded` — passé à `true` à la réception de l'événement
 *   `backend-degraded` (requirement 18.4). Affiche un bandeau rouge
 *   avec un bouton « Relancer ».
 *
 * Aucun des deux drapeaux n'est jamais remis à `false` automatiquement
 * par le module `events.ts` ; la remise à zéro est une décision
 * explicite du composant qui gère la résolution (paramètres ffprobe,
 * relance backend).
 */
export const flags: Writable<{ ffprobeMissing: boolean; backendDegraded: boolean }> =
  writable<{ ffprobeMissing: boolean; backendDegraded: boolean }>({
    ffprobeMissing: false,
    backendDegraded: false
  });
