<script lang="ts">
  /**
   * HoverOverlay — overlay affiché au survol d'une `MediaCard`
   * (tâche 14.2, requirement 15.4).
   *
   * Affiche dans l'ordre :
   *   - la note utilisateur (si non nulle),
   *   - soit la durée totale formatée (Films), soit le nombre
   *     d'épisodes (Séries / Animes),
   *   - le statut courant.
   *
   * Le composant reçoit un `MediaItem` (DTO de listing). Les champs
   * `user_rating`, `duration_seconds` et `episode_count` ne font pas
   * partie du DTO de listing minimal — ils sont déclarés ici en
   * optionnel pour permettre au parent de fournir une vue enrichie
   * (équivalent `MediaItemDetail` ou agrégat calculé côté store)
   * sans imposer un nouveau type. Quand un champ est `null` ou
   * absent, la ligne correspondante n'est pas rendue.
   *
   * Le placement est absolu par rapport au parent (`MediaCard` ou
   * conteneur équivalent en `position: relative`). La visibilité est
   * pilotée :
   *   1. par la prop `isHovering` (utile aux tests, et à un parent
   *      qui veut un contrôle programmatique), et
   *   2. en fallback CSS pur, par `:hover` sur le parent direct via
   *      le sélecteur global ciblant `.media-card:hover`.
   *
   * Une transition d'`opacity` rend l'apparition / disparition
   * progressive (pas de saut visuel).
   *
   * Style : Adwaita Dark via les tokens du thème
   * (`frontend/src/styles/theme.css`).
   *
   * _Requirements: 15.4_
   */
  import type { MediaItem, Status } from '$lib/api/types';

  /**
   * Type structurel attendu par l'overlay : un `MediaItem` étendu
   * d'attributs optionnels permettant de matérialiser la durée
   * (Films) ou le nombre d'épisodes (Séries / Animes), ainsi que la
   * note utilisateur. Tout objet structurellement compatible (y
   * compris `MediaItemDetail`) est accepté.
   */
  type HoverOverlayItem = MediaItem & {
    user_rating?: number | null;
    duration_seconds?: number | null;
    episode_count?: number | null;
  };

  export let item: HoverOverlayItem;
  /**
   * `true` quand le parent (généralement la `MediaCard`) est
   * survolé. Permet à un parent qui gère lui-même l'état hover
   * (mouseenter / mouseleave) de piloter la visibilité.
   */
  export let isHovering = false;

  /**
   * Libellés humains des statuts — alignés avec
   * `FilterPanel.svelte` pour rester cohérent dans toute l'UI.
   */
  const STATUS_LABELS: Record<Status, string> = {
    non_vu: 'Non vu',
    en_cours: 'En cours',
    vu: 'Vu'
  };

  /**
   * Format `Xh YY` ou `YY min` selon la durée.
   * - `0` ou valeur invalide ⇒ chaîne vide (la ligne ne sera pas
   *   rendue).
   * - `< 1 h` ⇒ `YY min`.
   * - `≥ 1 h` et minutes = 0 ⇒ `Xh`.
   * - sinon ⇒ `Xh YY` (minutes en deux chiffres).
   */
  function formatDuration(totalSeconds: number): string {
    if (!Number.isFinite(totalSeconds) || totalSeconds <= 0) return '';
    const totalMinutes = Math.floor(totalSeconds / 60);
    const hours = Math.floor(totalMinutes / 60);
    const minutes = totalMinutes % 60;
    if (hours === 0) return `${minutes} min`;
    if (minutes === 0) return `${hours} h`;
    return `${hours} h ${String(minutes).padStart(2, '0')}`;
  }

  $: isFilm = item.type === 'Film';

  $: durationLabel =
    isFilm && item.duration_seconds != null && item.duration_seconds > 0
      ? formatDuration(item.duration_seconds)
      : null;

  $: episodeLabel =
    !isFilm && item.episode_count != null && item.episode_count >= 0
      ? `${item.episode_count} épisode${item.episode_count > 1 ? 's' : ''}`
      : null;

  $: statusLabel = STATUS_LABELS[item.status];
</script>

<div
  class="hover-overlay"
  class:visible={isHovering}
  data-testid="hover-overlay"
  aria-hidden={!isHovering}
>
  {#if item.user_rating != null}
    <span class="row rating" data-testid="hover-rating">
      <span class="label">Note</span>
      <span class="value">{item.user_rating} / 10</span>
    </span>
  {/if}

  {#if durationLabel}
    <span class="row duration" data-testid="hover-duration">
      <span class="label">Durée</span>
      <span class="value">{durationLabel}</span>
    </span>
  {/if}

  {#if episodeLabel}
    <span class="row episodes" data-testid="hover-episodes">
      <span class="label">Épisodes</span>
      <span class="value">{episodeLabel}</span>
    </span>
  {/if}

  <span class="row status" data-testid="hover-status">
    <span class="label">Statut</span>
    <span class="value">{statusLabel}</span>
  </span>
</div>

<style>
  /*
    Positionnement absolu sur le parent (qui doit être en
    `position: relative` — c'est le cas de `.media-card`). On
    couvre toute la zone du parent pour un fond sombre lisible.

    Visibilité contrôlée :
      - par la prop `isHovering` (classe `.visible`), et
      - en fallback CSS pur, via le sélecteur global
        `:global(.media-card:hover) .hover-overlay`.
  */
  .hover-overlay {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    gap: var(--space-1);
    padding: var(--space-3);
    background: linear-gradient(
      180deg,
      rgba(30, 30, 46, 0) 0%,
      rgba(30, 30, 46, 0.55) 45%,
      rgba(30, 30, 46, 0.92) 100%
    );
    color: var(--text-primary);
    font-family: var(--font-family);
    font-size: 0.85rem;
    border-top: 1px solid transparent;
    border-radius: inherit;
    pointer-events: none;
    opacity: 0;
    transition: opacity 140ms ease;
  }

  .hover-overlay.visible {
    opacity: 1;
  }

  /*
    Fallback CSS pur : si le parent direct est une `MediaCard` et
    que la prop `isHovering` n'est pas pilotée, on s'appuie sur
    `:hover` du parent.
  */
  :global(.media-card:hover) .hover-overlay {
    opacity: 1;
  }

  .row {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: var(--space-3);
    line-height: 1.25;
  }

  .label {
    color: var(--text-secondary);
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .value {
    color: var(--text-primary);
    font-weight: 600;
  }

  /* Le statut est mis en avant via la couleur d'accent. */
  .status .value {
    color: var(--accent);
  }
</style>
