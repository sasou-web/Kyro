<script lang="ts">
  /**
   * `<ToastContainer>` — pile de toasts visible dans le coin
   * inférieur droit. Monté une seule fois dans le layout racine.
   *
   * Souscrit au store `toasts` (`$lib/stores/toast`) et rend chaque
   * toast comme une carte flottante avec :
   *   - icône SVG inline (succès / erreur / warning / info) ;
   *   - message ;
   *   - bouton ✕ pour fermer manuellement (utile si la durée est
   *     longue ou si l'utilisateur veut nettoyer rapidement).
   *
   * Animation : entrée par fly horizontal depuis la droite + fade,
   * sortie par fade rapide. La transition est appliquée via la
   * directive Svelte `transition:fly` couplée à `transition:fade`
   * pour un rendu fluide sans dépendance externe.
   *
   * Accessibilité :
   *   - `role="status"` (info / success) ou `role="alert"` (error /
   *     warning) pour que les lecteurs d'écran annoncent le toast.
   *   - `aria-live="polite"` au container pour les toasts info.
   */
  import { fly, fade } from 'svelte/transition';
  import { quintOut } from 'svelte/easing';

  import { toasts, toast as toastApi, type Toast } from '$lib/stores/toast';

  function ariaRole(kind: Toast['kind']): 'status' | 'alert' {
    return kind === 'error' || kind === 'warning' ? 'alert' : 'status';
  }
</script>

<div
  class="toast-container"
  aria-live="polite"
  aria-atomic="false"
  data-testid="toast-container"
>
  {#each $toasts as t (t.id)}
    <div
      class="toast"
      class:toast--success={t.kind === 'success'}
      class:toast--error={t.kind === 'error'}
      class:toast--warning={t.kind === 'warning'}
      class:toast--info={t.kind === 'info'}
      role={ariaRole(t.kind)}
      data-testid="toast"
      in:fly={{ x: 24, duration: 220, easing: quintOut }}
      out:fade={{ duration: 140 }}
    >
      <span class="toast-icon" aria-hidden="true">
        {#if t.kind === 'success'}
          <svg viewBox="0 0 24 24" width="20" height="20">
            <circle cx="12" cy="12" r="10" fill="currentColor" opacity="0.15" />
            <path d="M7 12.5l3.5 3.5L17 8.5" fill="none" stroke="currentColor"
                  stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        {:else if t.kind === 'error'}
          <svg viewBox="0 0 24 24" width="20" height="20">
            <circle cx="12" cy="12" r="10" fill="currentColor" opacity="0.15" />
            <path d="M9 9l6 6M15 9l-6 6" fill="none" stroke="currentColor"
                  stroke-width="2" stroke-linecap="round" />
          </svg>
        {:else if t.kind === 'warning'}
          <svg viewBox="0 0 24 24" width="20" height="20">
            <path d="M12 3l10 17H2z" fill="currentColor" opacity="0.15" />
            <path d="M12 3l10 17H2z" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round" />
            <path d="M12 10v5" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
            <circle cx="12" cy="18" r="1.1" fill="currentColor" />
          </svg>
        {:else}
          <svg viewBox="0 0 24 24" width="20" height="20">
            <circle cx="12" cy="12" r="10" fill="currentColor" opacity="0.15" />
            <path d="M12 8v8" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
            <circle cx="12" cy="6" r="1.1" fill="currentColor" />
          </svg>
        {/if}
      </span>

      <span class="toast-message">{t.message}</span>

      <button
        type="button"
        class="toast-close"
        aria-label="Fermer ce message"
        on:click={() => toastApi.dismiss(t.id)}
      >
        <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true">
          <path d="M6 6l12 12M18 6L6 18" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
        </svg>
      </button>
    </div>
  {/each}
</div>

<style>
  .toast-container {
    position: fixed;
    right: var(--space-4);
    bottom: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    z-index: 9999;
    pointer-events: none;
    max-width: min(420px, calc(100vw - var(--space-5) * 2));
  }

  .toast {
    pointer-events: auto;
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    background: var(--surface);
    color: var(--text-primary);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    box-shadow: var(--shadow-card-hover);
    font-family: var(--font-family);
    font-size: 0.9rem;
    line-height: 1.4;
    /* Bordure latérale épaisse colorée selon le `kind`. */
    border-left-width: 3px;
  }

  .toast--success { border-left-color: var(--color-success); }
  .toast--error   { border-left-color: var(--color-error);   }
  .toast--warning { border-left-color: var(--color-warning); }
  .toast--info    { border-left-color: var(--accent);        }

  .toast-icon {
    display: inline-flex;
    flex-shrink: 0;
  }

  .toast--success .toast-icon { color: var(--color-success); }
  .toast--error   .toast-icon { color: var(--color-error);   }
  .toast--warning .toast-icon { color: var(--color-warning); }
  .toast--info    .toast-icon { color: var(--accent);        }

  .toast-message {
    word-break: break-word;
  }

  .toast-close {
    background: transparent;
    border: none;
    padding: var(--space-1);
    color: var(--text-secondary);
    border-radius: var(--radius-control);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transition: background-color var(--duration-fast) var(--ease-standard),
      color var(--duration-fast) var(--ease-standard);
  }

  .toast-close:hover {
    background: var(--bg-secondary);
    color: var(--text-primary);
  }

  .toast-close:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
</style>
