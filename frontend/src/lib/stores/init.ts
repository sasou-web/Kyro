/**
 * Boot des stores Kyro depuis les settings persistés (tâche 15.1).
 *
 * `initSettings()` est appelée dans `+layout.svelte` au montage du
 * layout racine pour amorcer le store `settings` avec l'agrégat
 * retourné par la commande Tauri `get_settings_summary` (lecture en
 * bloc des paramètres SQLite : dossiers de bibliothèque, lecteur
 * externe, chemin ffprobe, scan au démarrage, thème, dernière
 * catégorie consultée).
 *
 * Le store `settings` reste à `null` tant que la commande n'a pas
 * abouti — les composants paramètres / TopBar gèrent cet état de
 * chargement (placeholder ou désactivation des actions). Le drapeau
 * `flags.ffprobeMissing` n'est PAS dérivé ici : le backend émet
 * l'événement `ffprobe-missing` quand il ne parvient pas à résoudre
 * un binaire (sidecar / `setting_value` / PATH). C'est la
 * souscription `setupEventListeners()` qui le pose à `true` —
 * raison pour laquelle l'ordre d'appel dans `+layout.svelte` est
 * d'abord `setupEventListeners()` puis `initSettings()`.
 *
 * En cas d'échec irrécupérable de la commande (pool SQLite verrouillé,
 * IO transport), on passe `flags.backendDegraded = true` pour
 * afficher le bandeau rouge dégradé (requirement 18.4) — l'événement
 * `backend-degraded` aurait normalement été émis par le backend, mais
 * la branche défensive ici garantit que l'utilisateur voit l'état
 * dégradé même si l'IPC n'a pas pu l'émettre.
 *
 * _Requirements: 5.4, 18.4, 16.1_
 */

import { getSettingsSummary } from '$lib/api/commands';
import { flags, settings } from './library';

/**
 * Charge la vue agrégée des paramètres et l'écrit dans le store
 * `settings`. À appeler une seule fois au boot du frontend, depuis
 * `+layout.svelte` après l'installation des listeners d'événements.
 */
export async function initSettings(): Promise<void> {
  try {
    const summary = await getSettingsSummary();
    settings.set(summary);
  } catch (err) {
    console.warn('Kyro: échec init settings', err);
    flags.update((f) => ({ ...f, backendDegraded: true }));
  }
}
