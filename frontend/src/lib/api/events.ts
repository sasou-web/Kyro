/**
 * Branchement des événements Tauri émis par le backend sur les
 * stores de bibliothèque (tâche 13.2).
 *
 * Le module expose une fonction unique [`setupEventListeners`]
 * appelée depuis `+layout.svelte` (au montage du layout racine) qui :
 *
 *   1. souscrit, via `listen()` (`@tauri-apps/api/event`), à tous les
 *      événements canoniques décrits dans `design.md` ;
 *   2. branche chaque événement sur le store correspondant
 *      (`scanProgress`, `items`, `flags`) ;
 *   3. retourne une fonction de cleanup qui appelle séquentiellement
 *      les `unlisten` retournés par chaque souscription. Le layout
 *      doit invoquer cette fonction dans `onDestroy` (et idéalement
 *      `await`-er `setupEventListeners` avant de la stocker pour ne
 *      pas perdre d'événements émis pendant l'attente).
 *
 * Conception en une **seule** fonction d'initialisation plutôt que des
 * appels `listen` épars : on garantit qu'il n'y a qu'un seul point
 * d'attache (pas de doubles abonnements en cas de re-rendu) et qu'on
 * peut détacher proprement.
 *
 * Aucun de ces événements n'est rejoué à la souscription par Tauri ;
 * il est donc important d'appeler `setupEventListeners` au plus tôt
 * (couche layout racine, avant le premier rendu de page utile) pour
 * ne pas rater un `scan-progress` ou un `backend-degraded` émis
 * pendant l'init.
 *
 * _Requirements: 3.6, 5.4, 18.4_
 */

import { listen, type UnlistenFn } from '@tauri-apps/api/event';

import { flags, items, scanProgress } from '$lib/stores/library';
import { listByCategory } from './commands';
import { toast } from '$lib/stores/toast';
import {
  EVENT_BACKEND_DEGRADED,
  EVENT_EXTERNAL_PLAYER_MISSING,
  EVENT_FFPROBE_MISSING,
  EVENT_LIBRARY_AUTOREFRESH,
  EVENT_PLAYBACK_ENDED,
  EVENT_SCAN_COMPLETED,
  EVENT_SCAN_PROGRESS,
  type Category,
  type ScanProgress
} from './types';

/**
 * Souscrit aux six événements Tauri consommés par la bibliothèque et
 * câble chacun sur les stores correspondants.
 *
 * @param refreshCategory - catégorie courante à recharger
 *   (`list_by_category`) à la fin d'un scan ou d'une session de
 *   lecture pour matérialiser les changements (nouveaux items,
 *   statuts mis à jour). `null` pour désactiver le refresh — utile
 *   pendant les tests ou avant la sélection de la catégorie initiale.
 *
 * @returns une fonction de cleanup à appeler depuis `onDestroy` du
 *   layout. Toutes les souscriptions sont détachées séquentiellement ;
 *   les erreurs de détachement sont log-and-ignored (best-effort).
 */
export async function setupEventListeners(
  refreshCategory: Category | null = null
): Promise<UnlistenFn> {
  // Helper local : recharge la liste pour la catégorie courante en
  // best-effort (échec silencieux logué, ne casse pas l'UI). Inline
  // plutôt qu'export pour éviter une API publique de plus.
  const refreshItems = async (): Promise<void> => {
    if (refreshCategory === null) {
      return;
    }
    try {
      const next = await listByCategory(refreshCategory);
      items.set(next);
    } catch (err) {
      console.warn('Kyro events: échec refresh items après scan/playback', err);
    }
  };

  // 1. scan-progress → met à jour la progression visible (TopBar).
  const unlistenScanProgress = await listen<ScanProgress>(
    EVENT_SCAN_PROGRESS,
    (event) => {
      scanProgress.set(event.payload);
    }
  );

  // 2. scan-completed → reset de la progression + refresh des items
  //    pour la catégorie courante (les nouveaux fichiers détectés
  //    apparaissent immédiatement sans rechargement manuel). Le
  //    payload `ScanReport` est ignoré ici ; les vues qui veulent
  //    afficher un résumé peuvent s'abonner séparément.
  const unlistenScanCompleted = await listen(EVENT_SCAN_COMPLETED, () => {
    scanProgress.set(null);
    void refreshItems();
  });

  // 3. ffprobe-missing → bandeau persistant via flags.ffprobeMissing.
  const unlistenFfprobeMissing = await listen(EVENT_FFPROBE_MISSING, () => {
    flags.update((f) => ({ ...f, ffprobeMissing: true }));
  });

  // 4. backend-degraded → bandeau rouge via flags.backendDegraded.
  const unlistenBackendDegraded = await listen<string>(
    EVENT_BACKEND_DEGRADED,
    (event) => {
      console.warn('Kyro backend dégradé:', event.payload);
      flags.update((f) => ({ ...f, backendDegraded: true }));
    }
  );

  // 5. external-player-missing → log uniquement ; la commande
  //    `play_media` propage déjà l'erreur applicative qui sera
  //    affichée par la fiche. On garde une trace côté console pour
  //    le diagnostic terrain.
  const unlistenExternalPlayerMissing = await listen<string>(
    EVENT_EXTERNAL_PLAYER_MISSING,
    (event) => {
      console.warn('Kyro lecteur externe introuvable:', event.payload);
    }
  );

  // 6. playback-ended → refresh des items pour matérialiser le
  //    nouveau statut (`vu` / `en_cours`) calculé pendant la session.
  const unlistenPlaybackEnded = await listen(EVENT_PLAYBACK_ENDED, () => {
    void refreshItems();
  });

  // 7. library-autorefresh → le watcher filesystem a détecté un
  //    changement et déclenché un scan incrémental. On rafraîchit
  //    la grille de la catégorie courante et on affiche un toast
  //    léger pour informer l'utilisateur.
  const unlistenLibraryAutorefresh = await listen(
    EVENT_LIBRARY_AUTOREFRESH,
    () => {
      toast.info('Bibliothèque mise à jour');
      void refreshItems();
    }
  );

  // Cleanup composé. On appelle chaque unlisten dans un try/catch
  // best-effort pour ne pas qu'une erreur isolée empêche les autres
  // détachements (cas typique : la fenêtre est en train de se fermer).
  return () => {
    for (const unlisten of [
      unlistenScanProgress,
      unlistenScanCompleted,
      unlistenFfprobeMissing,
      unlistenBackendDegraded,
      unlistenExternalPlayerMissing,
      unlistenPlaybackEnded,
      unlistenLibraryAutorefresh
    ]) {
      try {
        unlisten();
      } catch (err) {
        console.warn('Kyro events: échec détachement listener', err);
      }
    }
  };
}
