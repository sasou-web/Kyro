/**
 * Store global du player embarqué intégré.
 *
 * Le player est rendu en overlay dans `+layout.svelte` (tâche refonte
 * embedded-player). Trois modes d'affichage :
 *
 * - `'closed'` — aucun lecteur actif, HWND vidéo inexistant.
 * - `'fullscreen'` — le lecteur prend toute la zone de contenu (par-
 *    dessus la liste actuelle), avec contrôles complets.
 * - `'pip'` — mode Picture-in-Picture : mini fenêtre flottante en
 *    bas-droite, contrôles minimaux. La navigation Kyro reste
 *    pleinement utilisable derrière.
 *
 * Le store expose aussi le titre, la position et la durée pour
 * permettre à n'importe quel composant (sidebar, fiche détail) de
 * refléter l'état du player.
 */

import { writable, derived, get } from 'svelte/store';

import {
  embeddedPlayerQuit,
  embeddedPlayerSetVisible
} from '$lib/api/commands';

export type PlayerMode = 'closed' | 'fullscreen' | 'pip';

export interface PlayerState {
  mode: PlayerMode;
  /** Titre humain de la cible en cours (Film, ou "S01E04 — Titre"). */
  title: string | null;
  /** Backend cible (pour les "vu" et progress côté Repo). */
  target: { kind: 'media' | 'episode'; id: number } | null;
  /** Position courante (s) — alimentée par `embedded-player-progress`. */
  position: number;
  /** Durée totale (s) — alimentée par `embedded-player-progress`. */
  duration: number;
  /** Pause / lecture courante. */
  paused: boolean;
}

const initial: PlayerState = {
  mode: 'closed',
  title: null,
  target: null,
  position: 0,
  duration: 0,
  paused: false
};

export const player = writable<PlayerState>({ ...initial });

/** Indique si le lecteur est actif (fullscreen ou pip). */
export const playerActive = derived(player, ($p) => $p.mode !== 'closed');

/**
 * Marque le début d'une session (commande `embedded_player_open` réussie).
 * Le composant `<EmbeddedPlayer>` se chargera de poser la geometry.
 */
export function startSession(
  target: PlayerState['target'],
  title: string | null
): void {
  player.set({
    mode: 'fullscreen',
    title,
    target,
    position: 0,
    duration: 0,
    paused: false
  });
}

/** Bascule fullscreen ↔ pip sans toucher à la session libmpv. */
export function toggleMode(): void {
  player.update((s) => ({
    ...s,
    mode: s.mode === 'pip' ? 'fullscreen' : 'pip'
  }));
}

/** Force le mode (fullscreen | pip). */
export function setMode(mode: Exclude<PlayerMode, 'closed'>): void {
  player.update((s) => ({ ...s, mode }));
}

/**
 * Termine proprement la session libmpv et remet le store à zéro.
 * Tolère un appel quand aucune session n'est active.
 */
export async function closeSession(): Promise<void> {
  if (get(player).mode === 'closed') {
    return;
  }
  try {
    await embeddedPlayerQuit();
  } catch (err) {
    console.warn('player: quit', err);
  }
  player.set({ ...initial });
}

/**
 * Cache temporairement le HWND vidéo (utilisé quand l'utilisateur
 * minimise le PiP en taskbar/overlay sans vouloir tuer la session).
 */
export async function hideHostWindow(): Promise<void> {
  try {
    await embeddedPlayerSetVisible(false);
  } catch (err) {
    console.warn('player: hide host window', err);
  }
}
