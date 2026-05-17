<script lang="ts">
  /**
   * TopBar — barre supérieure de Kyro.
   *
   * Trois zones, de gauche à droite :
   *
   *  1. **Recherche** — délègue à `<SearchBar>` (tâche 14.4) qui
   *     pousse dans le store `searchQuery` (requirement 13.1).
   *
   *  2. **Indicateur de scan** — visible uniquement quand
   *     `$scanProgress` n'est pas `null`. Affiche
   *     `Scan: {processed}/{total} - {current_file}` ; le chemin du
   *     fichier courant est tronqué via CSS (`text-overflow: ellipsis`)
   *     pour éviter de pousser les badges hors champ
   *     (requirement 3.6).
   *
   *  3. **Badges d'état** — bandeau persistant pour les modes
   *     dégradés (requirements 5.4 et 18.4) :
   *       * `ffprobe-missing` (jaune) lorsque `$flags.ffprobeMissing`
   *         est `true` ;
   *       * `backend-degraded` (rouge) lorsque
   *         `$flags.backendDegraded` est `true`.
   *     Les deux badges peuvent être affichés simultanément.
   *
   * Style : Adwaita Dark via les variables CSS de
   * `frontend/src/styles/theme.css`. Les badges utilisent leurs
   * propres couleurs (jaune / rouge) — pas de variable CSS dédiée
   * dans le thème, mais les hex restent cohérents avec l'esthétique
   * sombre du reste de l'UI.
   *
   * _Requirements: 13.1, 5.4, 18.4, 3.6_
   */
  import SearchBar from './SearchBar.svelte';
  import { flags, scanProgress } from '$lib/stores/library';
</script>

<header class="top-bar">
  <div class="search">
    <SearchBar />
  </div>

  <div class="status">
    {#if $scanProgress !== null}
      <span class="scan" data-testid="scan-indicator" aria-live="polite">
        Scan: {$scanProgress.processed}/{$scanProgress.total} - {$scanProgress.current_file}
      </span>
    {/if}

    {#if $flags.ffprobeMissing}
      <span
        class="badge badge-warning"
        data-testid="badge-ffprobe-missing"
        role="status"
      >
        ffprobe-missing
      </span>
    {/if}

    {#if $flags.backendDegraded}
      <span
        class="badge badge-error"
        data-testid="badge-backend-degraded"
        role="status"
      >
        backend-degraded
      </span>
    {/if}
  </div>
</header>

<style>
  .top-bar {
    display: flex;
    align-items: center;
    gap: var(--space-4, 16px);
    padding: var(--space-3, 12px) var(--space-4, 16px);
    background: var(--bg-secondary);
    border-bottom: 1px solid var(--border);
    color: var(--text-primary);
    font-family: var(--font-family);
  }

  .search {
    flex: 0 0 auto;
  }

  .status {
    display: flex;
    align-items: center;
    gap: var(--space-3, 12px);
    margin-left: auto;
    min-width: 0;
  }

  .scan {
    display: inline-block;
    max-width: 360px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-secondary);
    font-size: 0.9rem;
  }

  .badge {
    display: inline-block;
    padding: var(--space-1, 4px) var(--space-2, 8px);
    border-radius: var(--radius-control, 8px);
    font-size: 0.8rem;
    font-weight: 600;
    line-height: 1;
    white-space: nowrap;
  }

  /* Jaune Adwaita pour l'avertissement ffprobe-missing. */
  .badge-warning {
    background: #f5c211;
    color: #1e1e2e;
  }

  /* Rouge Adwaita pour l'erreur backend-degraded. */
  .badge-error {
    background: #e01b24;
    color: #ffffff;
  }
</style>
