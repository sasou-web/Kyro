/**
 * Orchestration du player embarqué côté frontend.
 *
 * Le player est désormais **intégré** à la fenêtre principale Kyro :
 * libmpv rend dans une fenêtre Win32 enfant, positionnée par le
 * composant `<EmbeddedPlayer>` (cf. `lib/components/EmbeddedPlayer.svelte`)
 * sur une `<div>` qui sert de "trou pour la vidéo". Aucune fenêtre
 * Tauri secondaire n'est plus créée.
 *
 * Ce module expose les helpers de plus haut niveau utilisés par les
 * pages (Détail, etc.) pour :
 *
 *   - lire le réglage `playback_mode`,
 *   - démarrer la session libmpv (mode embedded),
 *   - tomber sur le flux historique mpv.net (mode external).
 */

import { invoke } from '@tauri-apps/api/core';

import { embeddedPlayerOpen, getPlaybackMode } from './commands';
import { startSession } from '$lib/stores/player';

/**
 * Démarre une session de lecture pour `(targetId, kind)`.
 *
 * - **`embedded`** : appelle `embedded_player_open`. Le composant
 *   `<EmbeddedPlayer>` réagira au changement du store `player` et
 *   posera la `set_geometry` après le premier resize.
 * - **`external`** : appelle `play_media` (flux historique mpv.net).
 *
 * Le paramètre `title` est purement cosmétique (titre affiché dans
 * la barre de contrôles) ; passer `null` si non disponible.
 */
export async function launchPlayback(
  targetId: number,
  kind: 'media' | 'episode',
  title: string | null = null
): Promise<void> {
  const mode = await getPlaybackMode();
  if (mode !== 'embedded') {
    await invoke<void>('play_media', { targetId, kind });
    return;
  }

  await embeddedPlayerOpen(targetId, kind);
  startSession({ kind, id: targetId }, title);
}

/**
 * Variante pour `play_resume`. Le backend résoudra l'épisode cible
 * et appellera `play_media` qui à son tour route vers `embedded`.
 */
export async function launchResume(seriesId: number): Promise<void> {
  const mode = await getPlaybackMode();
  if (mode === 'embedded') {
    // En mode embedded, le store est mis à jour par le composant
    // <EmbeddedPlayer> dès qu'un événement embedded-player-progress
    // arrive (le backend démarre la session avant de retourner).
    startSession(null, null);
  }
  await invoke<void>('play_resume', { seriesId });
}

/** Variante pour `play_rewatch_from_start`. */
export async function launchRewatchFromStart(seriesId: number): Promise<void> {
  const mode = await getPlaybackMode();
  if (mode === 'embedded') {
    startSession(null, null);
  }
  await invoke<void>('play_rewatch_from_start', { seriesId });
}
