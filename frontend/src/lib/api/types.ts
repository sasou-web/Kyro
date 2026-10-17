/**
 * Types DTO TypeScript miroir des structs Rust exposés par la couche
 * `commands::*` du backend (cf. `src-tauri/src/commands`).
 *
 * Aucune modification de structure ne doit être faite ici sans toucher
 * en parallèle au DTO Rust correspondant — la sérialisation IPC Tauri
 * passe directement par `serde_json` sans transformation, donc chaque
 * champ doit avoir le **même nom** et la **même casse** que côté Rust.
 *
 * Convention de casse : on conserve `snake_case` (cohérent avec les DTO
 * Rust) plutôt que de convertir en camelCase, ce qui éviterait toute
 * divergence et tout helper de mapping.
 *
 * _Requirements: 1.1_
 */

// ---------------------------------------------------------------------------
// Énumérations textuelles — alignées sur les `as_str()` des enums Rust
// et sur les CHECK SQL.
// ---------------------------------------------------------------------------

/**
 * Statut de visionnage d'un Media_Item ou d'un Episode_Item.
 *
 * Valeurs alignées sur le CHECK SQL `media_item.status` /
 * `episode_item.status` et sur `domain::status::Status::as_str()`.
 */
export type Status = 'non_vu' | 'en_cours' | 'vu';

/**
 * Catégorie d'un dossier de bibliothèque.
 *
 * Valeurs alignées sur le CHECK SQL `library_folder.category` et sur
 * `domain::types::Category::as_str()`.
 */
export type Category = 'Films' | 'Series' | 'Animes';

/**
 * Type d'un Media_Item.
 *
 * Distinct de `Category` (qui s'applique aux dossiers, pluriel) — un
 * Media_Item porte le type singulier (`Film`, `Series`, `Anime`).
 * Aligné sur `domain::types::MediaType::as_str()`.
 */
export type MediaType = 'Film' | 'Series' | 'Anime';

/**
 * Discriminant `(Media | Episode, id)` consommé par les commandes
 * polymorphes (`force_status`, `set_watch_progress`).
 *
 * Aligné sur `commands::library::TargetKind` (#[serde(rename_all =
 * "snake_case")]).
 */
export type TargetKind = 'media' | 'episode';

/**
 * Erreur remontée par toutes les commandes Tauri du backend.
 *
 * Côté Rust, `KyroError` implémente `Display` puis est sérialisée en
 * `String` via la conversion `serde::Serialize` du crate (cf.
 * `error.rs`). Le frontend reçoit donc une chaîne unique — il
 * n'existe pas de variantes typées côté IPC à ce stade.
 */
export type KyroError = string;

// ---------------------------------------------------------------------------
// DTOs métier — projection des structs Rust dans `commands::library`.
// ---------------------------------------------------------------------------

/**
 * Vue minimale d'un Media_Item retournée par les listings et la
 * recherche (`list_by_category`, `search`, `list_favorites`,
 * `list_to_watch`).
 *
 * Miroir de `commands::library::MediaItemDto`.
 */
export interface MediaItem {
  id: number;
  title: string;
  original_title: string | null;
  /** Borné [1900, 2099] côté Rust (requirement 4.2) ; `null` si inconnu. */
  year: number | null;
  /** Champ Rust : `media_type` exposé via `#[serde(rename = "type")]`. */
  type: MediaType;
  status: Status;
  genres: string[];
  /**
   * Chemin absolu local de l'affiche (cf. tâche 14.7), ou `null` si
   * aucune affiche n'a été importée pour cet item. Le frontend le
   * convertit en URL `asset://localhost/...` via `toAssetUrl`
   * (`$lib/api/poster-url`) avant de le passer à `<img src>`.
   */
  poster_path: string | null;
}

/**
 * Fiche complète d'un Media_Item retournée par `get_media_item`.
 *
 * Miroir de `commands::library::MediaItemDetailDto`. Les `i64` Rust
 * sont projetés en `number` ; en pratique les valeurs restent dans
 * `Number.MAX_SAFE_INTEGER` (id auto-incrémenté SQLite).
 */
export interface MediaItemDetail {
  id: number;
  /** Champ Rust : `media_type` exposé via `#[serde(rename = "type")]`. */
  type: MediaType;
  title: string;
  original_title: string | null;
  year: number | null;
  synopsis: string | null;
  studio: string | null;
  /** Plage [0, 10] côté Rust (requirement 6.6). */
  user_rating: number | null;
  poster_path: string | null;
  quality: string | null;
  duration_seconds: number | null;
  video_codec: string | null;
  width: number | null;
  height: number | null;
  watch_progress_seconds: number;
  status: Status;
  favorite: boolean;
  to_watch: boolean;
  available: boolean;
  numbering_unknown: boolean;
  genres: string[];
}

/**
 * Vue d'un Episode_Item retournée par `list_episodes`.
 *
 * Miroir de `commands::library::EpisodeItemDto`.
 */
export interface EpisodeItem {
  id: number;
  season_number: number;
  episode_number: number;
  status: Status;
  available: boolean;
  /** Position de lecture courante en secondes. */
  watch_progress_seconds: number;
  /** Durée totale en secondes (`null` tant que ffprobe n'a pas mesuré). */
  duration_seconds: number | null;
}

/**
 * Payload `update_media_item`. Miroir de
 * `commands::library::UpdateMediaItemInput`.
 */
export interface UpdateMediaItemInput {
  title: string;
  original_title: string | null;
  year: number | null;
  synopsis: string | null;
  studio: string | null;
  user_rating: number | null;
  status: Status;
}

/**
 * Payload `update_episode_item`. Miroir de
 * `commands::library::UpdateEpisodeItemInput`.
 */
export interface UpdateEpisodeItemInput {
  title: string | null;
  synopsis: string | null;
  status: Status;
}

// ---------------------------------------------------------------------------
// Filtres (domaine `Filters`) — utilisés par les listings filtrés.
// ---------------------------------------------------------------------------

/**
 * Critères de filtrage applicables à une bibliothèque (requirement 14).
 *
 * Sémantique : `genres` est une union (au moins un genre commun
 * suffit) ; `years`, `types`, `statuses` sont des appartenances
 * ensemblistes. Une liste vide signifie « pas de contrainte sur cette
 * dimension ». Cf. `domain::filters::Filters`.
 */
export interface Filters {
  genres: string[];
  years: number[];
  types: MediaType[];
  statuses: Status[];
}

// ---------------------------------------------------------------------------
// Settings (domaine `commands::settings`).
// ---------------------------------------------------------------------------

/**
 * Projection JSON-friendly d'un `library_folder` (miroir de
 * `commands::settings::SerializableFolder`).
 */
export interface LibraryFolder {
  id: number;
  path: string;
  category: Category;
}

/**
 * Vue agrégée de tous les paramètres persistés dans `setting_value`,
 * consommée par la page Paramètres (miroir de
 * `commands::settings::SettingsSummary`).
 */
export interface SettingsSummary {
  library_folders: LibraryFolder[];
  external_player_path: string | null;
  ffprobe_path: string | null;
  scan_at_startup: boolean;
  /** Toujours présent ; défaut `"adwaita-dark"` au premier démarrage. */
  theme: string;
  last_category: Category | null;
  /** Toujours présent ; défaut `"embedded"` au premier démarrage. */
  playback_mode: 'embedded' | 'external';
}

// ---------------------------------------------------------------------------
// Scan (domaine `commands::scan` + `infra::scanner`).
// ---------------------------------------------------------------------------

/**
 * Réponse de `scan_status`. Miroir de
 * `commands::scan::ScanStatus`.
 */
export interface ScanStatus {
  /** `true` ssi un scan complet est actuellement en cours. */
  running: boolean;
}

/**
 * Payload de l'événement Tauri `scan-progress` émis pendant un scan.
 *
 * Miroir de `infra::scanner::ScanProgress`. `processed` est 1-indexé
 * (post-traitement du fichier courant) ; `total` reste constant
 * pendant tout le scan.
 */
export interface ScanProgress {
  processed: number;
  total: number;
  current_file: string;
}

/**
 * Catégorie d'échec rencontrée pendant le scan d'un fichier.
 *
 * Miroir de `infra::scanner::ScanFailureKind`. `serde` sérialise les
 * variantes en PascalCase par défaut.
 */
export type ScanFailureKind = 'Io' | 'Probe' | 'Db' | 'Walk';

/**
 * Détail d'un échec isolé. Miroir de
 * `infra::scanner::ScanFailure`.
 */
export interface ScanFailure {
  file_path: string;
  kind: ScanFailureKind;
  message: string;
}

/**
 * Rapport agrégé d'un scan complet — payload de l'événement
 * `scan-completed`. Miroir de `commands::scan::ScanCompleted` (les
 * échecs sont projetés en compteur, le détail individuel passe par
 * les logs `tracing`).
 */
export interface ScanReport {
  total_files: number;
  processed: number;
  created: number;
  updated: number;
  marked_unavailable: number;
  /** Nombre total d'échecs « par fichier » sur le scan. */
  failures: number;
  /** Nombre de dossiers dont le scan a globalement échoué. */
  folder_errors: number;
}

// ---------------------------------------------------------------------------
// Constantes d'événements Tauri (miroir des `pub const EVENT_*`).
//
// Centraliser les noms côté frontend évite que plusieurs composants ne
// référencent des littéraux différents par accident.
// ---------------------------------------------------------------------------

/** Nom canonique de l'événement émis pendant un scan. */
export const EVENT_SCAN_PROGRESS = 'scan-progress';

/** Nom canonique de l'événement émis à la fin d'un scan complet. */
export const EVENT_SCAN_COMPLETED = 'scan-completed';

/** Émis quand mpv signale la fin d'une session de lecture. */
export const EVENT_PLAYBACK_ENDED = 'playback-ended';

/**
 * Émis quand le watcher filesystem a détecté un changement et déclenché
 * un scan incrémental sur un library_folder. Le payload est l'`id`
 * (number) du folder concerné. Permet à l'UI d'afficher un toast léger
 * « bibliothèque mise à jour » et de recharger la liste courante.
 */
export const EVENT_LIBRARY_AUTOREFRESH = 'library-autorefresh';

/**
 * Émis à chaque message de progression reçu pendant la session IPC
 * mpv (toutes les ~5 s, ou sur pause/reprise). Permet à la fiche
 * détail courante de mettre à jour la chip « X min restantes » et la
 * barre de progression en direct sans rechargement.
 */
export const EVENT_PLAYBACK_PROGRESS = 'playback-progress';

/** Payload de [`EVENT_PLAYBACK_PROGRESS`]. */
export interface PlaybackProgress {
  kind: 'media' | 'episode';
  target_id: number;
  position_seconds: number;
}

/** Émis quand le lecteur externe configuré est introuvable / invalide. */
export const EVENT_EXTERNAL_PLAYER_MISSING = 'external-player-missing';

/** Émis quand le lecteur externe utilisé n'est pas mpv (pas d'IPC). */
export const EVENT_EXTERNAL_PLAYER_NON_MPV = 'external-player-non-mpv';

/**
 * Émis au boot quand `ffprobe` n'a pu être résolu (sidecar absent,
 * setting invalide, PATH dépourvu) — requirement 5.4. Aligné sur
 * `infra::ffprobe_locator::EVENT_FFPROBE_MISSING`.
 */
export const EVENT_FFPROBE_MISSING = 'ffprobe-missing';

/**
 * Émis quand une erreur irrécupérable du backend remonte (pool SQLite
 * verrouillé / fermé / IO transport) — requirement 18.4. Aligné sur
 * `events::EVENT_BACKEND_DEGRADED`.
 */
export const EVENT_BACKEND_DEGRADED = 'backend-degraded';

// ---------------------------------------------------------------------------
// Player embarqué (libmpv chargé in-process). Aligné sur
// `commands::embedded_player::EVENT_*` côté Rust.
// ---------------------------------------------------------------------------

/** Émis ~10 fois/s pendant la lecture embarquée pour pousser la
 * position courante (et la durée totale dès qu'elle est connue). */
export const EVENT_EMBEDDED_PLAYER_PROGRESS = 'embedded-player-progress';

/** Payload de [`EVENT_EMBEDDED_PLAYER_PROGRESS`] et de
 * [`EVENT_EMBEDDED_PLAYER_CLOSED`]. */
export interface EmbeddedPlayerProgress {
  position: number;
  duration: number;
}

/** Émis quand mpv atteint `eof-reached = true` : permet à l'UI de
 * basculer le statut "Vu" et d'enchaîner l'épisode suivant. */
export const EVENT_EMBEDDED_PLAYER_COMPLETED = 'embedded-player-completed';

/** Émis quand l'état pause/lecture change (clavier, espace, etc.). */
export const EVENT_EMBEDDED_PLAYER_PAUSE_CHANGED = 'embedded-player-pause-changed';

/** Payload de [`EVENT_EMBEDDED_PLAYER_PAUSE_CHANGED`]. */
export interface EmbeddedPlayerPause {
  paused: boolean;
}

/** Émis une seule fois quand la session libmpv est terminée
 * (mpv_terminate_destroy). Plus aucun event ne suivra. */
export const EVENT_EMBEDDED_PLAYER_CLOSED = 'embedded-player-closed';

/** Émis quand l'utilisateur clique sur la zone vidéo (HWND mpv).
 * Le frontend bascule pause / lecture en réaction. */
export const EVENT_EMBEDDED_PLAYER_CLICKED = 'embedded-player-clicked';

/** Émis quand l'utilisateur double-clique sur la zone vidéo.
 * Le frontend bascule le plein écran système. */
export const EVENT_EMBEDDED_PLAYER_DOUBLE_CLICKED =
  'embedded-player-double-clicked';

/** Modes de lecture acceptés par le réglage `playback_mode`. */
export type PlaybackMode = 'embedded' | 'external';

/**
 * Élément de la `track-list` mpv. Spec officielle :
 * https://mpv.io/manual/master/#command-interface-track-list
 *
 * Champs marqués optionnels (`?`) car mpv les omet selon le type
 * de piste et la source.
 */
export interface MpvTrack {
  id: number;
  type: 'video' | 'audio' | 'sub';
  /** Nom lisible. Souvent une langue ("French") ou un codec. */
  title?: string;
  /** Code langue ISO (ex. "fre", "eng", "jpn"). */
  lang?: string;
  /** Flag mpv : piste sélectionnée. */
  selected?: boolean;
  /** Flag mpv : piste par défaut côté conteneur. */
  default?: boolean;
  /** Flag mpv : piste forced (sous-titres forcés). */
  forced?: boolean;
  /** Codec (ex. "h264", "aac", "ass"). */
  codec?: string;
  /** Pour les pistes audio : nb de canaux (2, 6, 8...). */
  'demux-channel-count'?: number;
  /** Pour les pistes vidéo : largeur. */
  'demux-w'?: number;
  /** Pour les pistes vidéo : hauteur. */
  'demux-h'?: number;
}
