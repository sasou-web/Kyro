/**
 * Wrappers TypeScript typés pour chaque commande `#[tauri::command]`
 * exposée par le backend Kyro (cf. `src-tauri/src/commands`).
 *
 * Chaque fonction se contente d'appeler `invoke` avec :
 *   1. le **nom littéral** de la commande Rust (snake_case, identique
 *      au nom de la fonction `#[tauri::command]` côté backend) ;
 *   2. un objet d'arguments dont les **clés** correspondent aux
 *      paramètres positionnels Rust en camelCase (Tauri v2 convertit
 *      automatiquement les clés JS camelCase vers les noms snake_case
 *      attendus par les commandes — comportement par défaut depuis
 *      Tauri 1.x, cf. https://v2.tauri.app/develop/calling-rust/).
 *
 * Centraliser ces wrappers permet :
 *   - d'avoir un seul point où typer les retours (le frontend ne
 *     manipule jamais `invoke<unknown>`) ;
 *   - de retrouver toutes les commandes d'un coup d'œil pour vérifier
 *     la couverture quand un nouveau `#[tauri::command]` est ajouté ;
 *   - de mocker `@tauri-apps/api/core` une seule fois dans les tests.
 *
 * _Requirements: 1.1_
 */

import { invoke } from '@tauri-apps/api/core';

import type {
  Category,
  EpisodeItem,
  LibraryFolder,
  MediaItem,
  MediaItemDetail,
  ScanStatus,
  SettingsSummary,
  Status,
  TargetKind,
  UpdateEpisodeItemInput,
  UpdateMediaItemInput
} from './types';

// ===========================================================================
// Library — `commands::library`.
// ===========================================================================

/** Liste les Media_Item d'une catégorie (requirement 7.2). */
export async function listByCategory(category: Category): Promise<MediaItem[]> {
  return invoke<MediaItem[]>('list_by_category', { category });
}

/**
 * Recherche par titre / titre original, insensible à la casse et aux
 * accents (requirement 13.2).
 */
export async function search(query: string): Promise<MediaItem[]> {
  return invoke<MediaItem[]>('search', { query });
}

/** Liste les Media_Item marqués favoris (requirement 11.4). */
export async function listFavorites(): Promise<MediaItem[]> {
  return invoke<MediaItem[]>('list_favorites');
}

/** Liste les Media_Item marqués « à regarder » (requirement 11.5). */
export async function listToWatch(): Promise<MediaItem[]> {
  return invoke<MediaItem[]>('list_to_watch');
}

/** Récupère la fiche complète d'un Media_Item (requirement 6.1). */
export async function getMediaItem(id: number): Promise<MediaItemDetail> {
  return invoke<MediaItemDetail>('get_media_item', { id });
}

/** Liste les Episode_Item d'une série triés (requirements 7.3, 7.4). */
export async function listEpisodes(seriesId: number): Promise<EpisodeItem[]> {
  return invoke<EpisodeItem[]>('list_episodes', { seriesId });
}

/** Met à jour les champs éditables d'un Media_Item (requirement 6.1). */
export async function updateMediaItem(
  id: number,
  input: UpdateMediaItemInput
): Promise<void> {
  return invoke<void>('update_media_item', { id, input });
}

/** Met à jour les champs éditables d'un Episode_Item (requirement 6.5). */
export async function updateEpisodeItem(
  id: number,
  input: UpdateEpisodeItemInput
): Promise<void> {
  return invoke<void>('update_episode_item', { id, input });
}

/** Bascule le drapeau `favorite` (requirement 11.2). Retourne le nouvel état. */
export async function toggleFavorite(id: number): Promise<boolean> {
  return invoke<boolean>('toggle_favorite', { id });
}

/** Bascule le drapeau `to_watch` (requirement 11.3). Retourne le nouvel état. */
export async function toggleToWatch(id: number): Promise<boolean> {
  return invoke<boolean>('toggle_to_watch', { id });
}

/**
 * Force manuellement le statut d'un Media_Item ou Episode_Item
 * (requirement 8.4).
 */
export async function forceStatus(
  targetKind: TargetKind,
  targetId: number,
  status: Status
): Promise<void> {
  return invoke<void>('force_status', { targetKind, targetId, status });
}

/**
 * Met à jour la progression de lecture (requirements 8.1–8.3, 9.4).
 * Retourne le nouveau statut sérialisé (`non_vu` / `en_cours` / `vu`).
 */
export async function setWatchProgress(
  targetKind: TargetKind,
  targetId: number,
  position: number
): Promise<Status> {
  return invoke<Status>('set_watch_progress', {
    targetKind,
    targetId,
    position
  });
}

/**
 * Importe une affiche locale pour un Media_Item (requirements 6.2, 6.3).
 * Retourne le chemin final dans `posters_dir`.
 */
export async function importPosterForMedia(
  mediaId: number,
  sourcePath: string,
  postersDir: string
): Promise<string> {
  return invoke<string>('import_poster_for_media', {
    mediaId,
    sourcePath,
    postersDir
  });
}

/**
 * Importe une affiche locale pour un Episode_Item (requirements 6.2,
 * 6.3, 6.5).
 */
export async function importPosterForEpisode(
  episodeId: number,
  sourcePath: string,
  postersDir: string
): Promise<string> {
  return invoke<string>('import_poster_for_episode', {
    episodeId,
    sourcePath,
    postersDir
  });
}

/**
 * Calcule l'épisode à reprendre dans une série (requirement 12.2).
 * Retourne `null` si aucun épisode n'est jouable.
 */
export async function nextResumeEpisode(seriesId: number): Promise<number | null> {
  return invoke<number | null>('next_resume_episode', { seriesId });
}

// ===========================================================================
// Scan — `commands::scan`.
// ===========================================================================

/**
 * Déclenche un scan complet **non-bloquant** (requirements 3.1, 3.6).
 * Le frontend doit s'abonner aux événements `scan-progress` /
 * `scan-completed` pour suivre l'avancée.
 */
export async function startScan(): Promise<void> {
  return invoke<void>('start_scan');
}

/** Retourne `{ running: bool }` indiquant si un scan est en cours. */
export async function scanStatus(): Promise<ScanStatus> {
  return invoke<ScanStatus>('scan_status');
}

// ===========================================================================
// Playback — `commands::playback`.
// ===========================================================================

/**
 * Installe `kyro.lua` dans `%APPDATA%\mpv.net\scripts\` (requirement
 * 9.7). Retourne le chemin écrit.
 */
export async function installLuaScript(): Promise<string> {
  return invoke<string>('install_lua_script');
}

/**
 * Lance la lecture d'un Media_Item ou d'un Episode_Item via le lecteur
 * externe configuré (requirements 9.1, 9.3, 9.4, 10.3, 10.4, 10.5,
 * 12.4). `kind` ∈ {`'media'`, `'episode'`}.
 */
export async function playMedia(
  targetId: number,
  kind: 'media' | 'episode'
): Promise<void> {
  return invoke<void>('play_media', { targetId, kind });
}

/** Reprend une série depuis l'épisode courant (requirements 12.2, 12.3). */
export async function playResume(seriesId: number): Promise<void> {
  return invoke<void>('play_resume', { seriesId });
}

/**
 * Remet à 0 toute la progression de la série puis relance depuis le
 * premier épisode (requirement 12.3).
 */
export async function playRewatchFromStart(seriesId: number): Promise<void> {
  return invoke<void>('play_rewatch_from_start', { seriesId });
}

// ===========================================================================
// Settings — `commands::settings`.
// ===========================================================================

/**
 * Ajoute un dossier de bibliothèque (requirements 2.2, 2.4, 2.5, 7.1).
 * Retourne l'`id` du dossier créé.
 */
export async function addLibraryFolder(
  path: string,
  category: Category
): Promise<number> {
  return invoke<number>('add_library_folder', { path, category });
}

/**
 * Retire un dossier de bibliothèque ; les items rattachés sont
 * marqués `available = 0` (requirement 2.3).
 */
export async function removeLibraryFolder(id: number): Promise<void> {
  return invoke<void>('remove_library_folder', { id });
}

/** Liste tous les dossiers de bibliothèque (requirement 2.1). */
export async function listLibraryFolders(): Promise<LibraryFolder[]> {
  return invoke<LibraryFolder[]>('list_library_folders');
}

/**
 * Enregistre le chemin du lecteur externe après validation existence
 * + extension `.exe` (requirements 10.1, 10.3, 10.5).
 */
export async function setExternalPlayerPath(path: string): Promise<void> {
  return invoke<void>('set_external_player_path', { path });
}

/** Retourne le chemin du lecteur externe enregistré (`null` si absent). */
export async function getExternalPlayerPath(): Promise<string | null> {
  return invoke<string | null>('get_external_player_path');
}

/** Enregistre le chemin manuel vers `ffprobe.exe` (requirement 5.4). */
export async function setFfprobePath(path: string): Promise<void> {
  return invoke<void>('set_ffprobe_path', { path });
}

/** Active / désactive le scan automatique au démarrage (requirement 3.7). */
export async function setScanAtStartup(enabled: boolean): Promise<void> {
  return invoke<void>('set_scan_at_startup', { enabled });
}

/** Enregistre le thème actif (requirements 16.1, 16.14). */
export async function setTheme(theme: string): Promise<void> {
  return invoke<void>('set_theme', { theme });
}

/** Retourne le thème actif, défaut `"adwaita-dark"`. */
export async function getTheme(): Promise<string> {
  return invoke<string>('get_theme');
}

/** Mémorise la dernière catégorie consultée (requirement 7.1). */
export async function setLastCategory(category: Category): Promise<void> {
  return invoke<void>('set_last_category', { category });
}

/** Retourne la dernière catégorie consultée (`null` au premier démarrage). */
export async function getLastCategory(): Promise<Category | null> {
  return invoke<Category | null>('get_last_category');
}

/**
 * Enregistre le mode de lecture actif. Valeurs acceptées :
 * `"embedded"` (player libmpv intégré, défaut) ou `"external"`
 * (mpv.net ou autre exécutable). Côté backend, l'aiguillage est
 * réalisé par `commands::playback::play_media` qui lit ce setting
 * avant chaque démarrage de lecture.
 */
export async function setPlaybackMode(
  mode: 'embedded' | 'external'
): Promise<void> {
  return invoke<void>('set_playback_mode', { mode });
}

/** Retourne le mode de lecture actif, défaut `"embedded"` au premier démarrage. */
export async function getPlaybackMode(): Promise<'embedded' | 'external'> {
  return invoke<'embedded' | 'external'>('get_playback_mode');
}

// ===========================================================================
// Embedded player — `commands::embedded_player`.
// ===========================================================================

/**
 * Démarre une session de lecture dans la fenêtre Tauri `player`.
 * Pré-condition : la fenêtre `player` doit avoir été créée par le
 * frontend (`WebviewWindow.getByLabel('player')` ou `new WebviewWindow`)
 * avant l'invoke.
 *
 * Erreurs explicites :
 * - `playback_mode != "embedded"` → utilisez `playMedia` à la place.
 * - `libmpv-2.dll` introuvable → bascule recommandée vers `external`.
 */
export async function embeddedPlayerOpen(
  targetId: number,
  kind: 'media' | 'episode'
): Promise<void> {
  return invoke<void>('embedded_player_open', { targetId, kind });
}

/** Bascule pause / lecture sur la session embarquée courante. */
export async function embeddedPlayerTogglePause(): Promise<void> {
  return invoke<void>('embedded_player_toggle_pause');
}

/** Saute à `positionSeconds` (absolu). */
export async function embeddedPlayerSeek(
  positionSeconds: number
): Promise<void> {
  return invoke<void>('embedded_player_seek', { positionSeconds });
}

/** Sélectionne une piste audio. `null` = piste par défaut, `-1` = mute. */
export async function embeddedPlayerSetAudioTrack(
  aid: number | null
): Promise<void> {
  return invoke<void>('embedded_player_set_audio_track', { aid });
}

/** Sélectionne une piste sous-titres. `null` ou `-1` = pas de sous-titres. */
export async function embeddedPlayerSetSubtitleTrack(
  sid: number | null
): Promise<void> {
  return invoke<void>('embedded_player_set_subtitle_track', { sid });
}

/** Termine la session embarquée courante. */
export async function embeddedPlayerQuit(): Promise<void> {
  return invoke<void>('embedded_player_quit');
}

/**
 * Repositionne / redimensionne le HWND vidéo dans la fenêtre Kyro.
 * Coordonnées en **pixels physiques** dans le repère de la fenêtre
 * `main`. Le frontend doit multiplier les valeurs CSS par
 * `window.devicePixelRatio` avant l'invoke.
 */
export async function embeddedPlayerSetGeometry(
  x: number,
  y: number,
  width: number,
  height: number
): Promise<void> {
  return invoke<void>('embedded_player_set_geometry', { x, y, width, height });
}

/**
 * Affiche / masque le HWND vidéo sans détruire la session libmpv.
 */
export async function embeddedPlayerSetVisible(visible: boolean): Promise<void> {
  return invoke<void>('embedded_player_set_visible', { visible });
}

/** Règle le volume libmpv (0 à 150). 100 = volume nominal. */
export async function embeddedPlayerSetVolume(volume: number): Promise<void> {
  return invoke<void>('embedded_player_set_volume', { volume });
}

/** Mute / unmute. */
export async function embeddedPlayerSetMute(muted: boolean): Promise<void> {
  return invoke<void>('embedded_player_set_mute', { muted });
}

/** Vitesse de lecture (0.1 à 8.0). 1.0 = normale. */
export async function embeddedPlayerSetSpeed(speed: number): Promise<void> {
  return invoke<void>('embedded_player_set_speed', { speed });
}

/**
 * Retourne la `track-list` mpv au format JSON (string).
 * Voir https://mpv.io/manual/master/#command-interface-track-list
 * pour la spec exacte du tableau retourné.
 */
export async function embeddedPlayerTrackList(): Promise<string> {
  return invoke<string>('embedded_player_track_list');
}

/**
 * Retourne l'agrégat `SettingsSummary` consommé par la page
 * Paramètres (lecture en bloc).
 */
export async function getSettingsSummary(): Promise<SettingsSummary> {
  return invoke<SettingsSummary>('get_settings_summary');
}

// ===========================================================================
// Logs — `commands::logs`.
// ===========================================================================

/**
 * Retourne le contenu (UTF-8 lossy) du fichier de log le plus récent
 * écrit par le `RollingFileAppender` ; tronqué à 5 MiB
 * (requirement 18.3).
 */
export async function readLatestLog(): Promise<string> {
  return invoke<string>('read_latest_log');
}

// ===========================================================================
// Backup / restore — `commands::backup`.
// ===========================================================================

/**
 * Manifest renvoyé par les commandes export/restore.
 *
 * Aligné sur `infra::backup::BackupManifest` côté Rust.
 */
export interface BackupManifest {
  kyro_version: string;
  created_at_unix: number;
  media_count: number;
  episode_count: number;
  posters_count: number;
}

/**
 * Crée une archive ZIP à `destPath` contenant `library.db` (copie
 * cohérente via `VACUUM INTO`) et le dossier `posters/`. Le chemin de
 * destination doit être absolu et son dossier parent doit exister.
 */
export async function exportLibraryBackup(
  destPath: string
): Promise<BackupManifest> {
  return invoke<BackupManifest>('export_library_backup', { destPath });
}

/**
 * Restaure une archive ZIP vers le data dir courant. **Le pool SQLite
 * est fermé pendant la restauration** : l'appelant DOIT recharger la
 * fenêtre (`window.location.reload()`) après succès pour que la base
 * soit ré-ouverte.
 */
export async function restoreLibraryBackup(
  archivePath: string
): Promise<BackupManifest> {
  return invoke<BackupManifest>('restore_library_backup', { archivePath });
}

// ===========================================================================
// Updates / version — `commands::updates`.
// ===========================================================================

/**
 * Métadonnées de version exposées par `get_app_version`. Aligné sur
 * `commands::updates::AppVersionInfo` côté Rust.
 */
export interface AppVersionInfo {
  version: string;
  releases_url: string;
}

/**
 * Retourne la version courante de Kyro (compilée à `cargo build`) et
 * l'URL canonique des releases publiques.
 */
export async function getAppVersion(): Promise<AppVersionInfo> {
  return invoke<AppVersionInfo>('get_app_version');
}

/**
 * Ouvre la page GitHub Releases dans le navigateur système. Aucune
 * requête HTTP n'est faite par Kyro lui-même — la responsabilité est
 * déléguée au navigateur (cf. Requirement 17.3, Property 13).
 */
export async function openReleasesPage(): Promise<void> {
  return invoke<void>('open_releases_page');
}
