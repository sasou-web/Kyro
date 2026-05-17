/**
 * Système de toasts — feedback non bloquant pour les actions
 * utilisateur (succès, erreurs, infos).
 *
 * Utilisation depuis n'importe quelle page / composant :
 *
 * ```ts
 * import { toast } from '$lib/stores/toast';
 *
 * toast.success('Affiche importée');
 * toast.error('Échec de l\'import : extension non supportée');
 * toast.info('Scan en cours…');
 * ```
 *
 * Le composant `<ToastContainer>` (rendu une seule fois dans le
 * layout racine) souscrit à ce store et affiche les toasts dans un
 * coin de l'écran avec une animation d'entrée / sortie.
 *
 * Auto-dismiss : par défaut chaque toast disparaît après 4 secondes
 * (3 s pour les info, 5 s pour les erreurs — les erreurs ont
 * besoin de plus de temps de lecture). L'utilisateur peut aussi
 * cliquer sur la croix pour le fermer immédiatement.
 *
 * Limite : on borne le nombre de toasts simultanés à 4 pour éviter
 * que la pile envahisse l'écran si une boucle d'erreurs fire en
 * masse. Au-delà, les plus anciens sont retirés en silence.
 */

import { writable, type Readable } from 'svelte/store';

export type ToastKind = 'success' | 'error' | 'info' | 'warning';

export interface Toast {
  /** Identifiant interne, utilisé pour `{#each ... (id)}` Svelte. */
  id: number;
  /** Catégorie qui détermine couleur + icône. */
  kind: ToastKind;
  /** Texte affiché dans le toast. */
  message: string;
  /** Durée d'affichage avant auto-dismiss (ms). */
  durationMs: number;
}

/** Borne supérieure de la pile pour ne pas envahir l'écran. */
const MAX_TOASTS = 4;

/** Durées par défaut selon le `kind`. */
const DEFAULT_DURATION_MS: Record<ToastKind, number> = {
  success: 3500,
  info: 3000,
  warning: 4500,
  error: 5500
};

const _toasts = writable<Toast[]>([]);
let nextId = 1;
const timers = new Map<number, ReturnType<typeof setTimeout>>();

/** Lecture seule — c'est ce que le composant `<ToastContainer>` consomme. */
export const toasts: Readable<Toast[]> = { subscribe: _toasts.subscribe };

function dismiss(id: number): void {
  const timer = timers.get(id);
  if (timer !== undefined) {
    clearTimeout(timer);
    timers.delete(id);
  }
  _toasts.update((list) => list.filter((t) => t.id !== id));
}

function push(kind: ToastKind, message: string, durationMs?: number): number {
  const id = nextId++;
  const duration = durationMs ?? DEFAULT_DURATION_MS[kind];
  const toast: Toast = { id, kind, message, durationMs: duration };

  _toasts.update((list) => {
    const next = [...list, toast];
    // Trim pour respecter MAX_TOASTS (les plus anciens partent).
    if (next.length > MAX_TOASTS) {
      const dropped = next.splice(0, next.length - MAX_TOASTS);
      // Annuler aussi leurs timers pour ne pas tenter de retirer un
      // id déjà absent.
      for (const t of dropped) {
        const timer = timers.get(t.id);
        if (timer !== undefined) {
          clearTimeout(timer);
          timers.delete(t.id);
        }
      }
    }
    return next;
  });

  // Auto-dismiss : `durationMs <= 0` désactive le timer (toast
  // persistant que seul le clic ferme).
  if (duration > 0) {
    const timer = setTimeout(() => dismiss(id), duration);
    timers.set(id, timer);
  }

  return id;
}

/**
 * API publique du store. Toutes les fonctions retournent l'`id` du
 * toast créé pour permettre à l'appelant de le dismisser
 * programmatiquement (ex. fermer un toast « scan en cours » quand
 * `scan-completed` arrive).
 */
export const toast = {
  success(message: string, durationMs?: number): number {
    return push('success', message, durationMs);
  },
  error(message: string, durationMs?: number): number {
    return push('error', message, durationMs);
  },
  info(message: string, durationMs?: number): number {
    return push('info', message, durationMs);
  },
  warning(message: string, durationMs?: number): number {
    return push('warning', message, durationMs);
  },
  dismiss
};
