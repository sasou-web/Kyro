<script lang="ts">
  /**
   * Page détail d'un Media_Item — tâche 15.4
   * (Requirements 7.3, 7.4, 8.5, 12.2, 12.3, 12.4, 15.4).
   *
   * Cette page rend la fiche complète d'un Media_Item (Film, Série
   * ou Anime) :
   *
   *   - **En-tête** : affiche locale (visuel SVG par défaut si
   *     `poster_path` absent — Requirement 15.2 délégué à la même
   *     logique que `MediaCard`), titre, titre original, année,
   *     note utilisateur, statut courant, contrôles `Favori` /
   *     `À regarder` (Requirements 11.2, 11.3) et bouton d'action
   *     principal :
   *       - `Lire` pour les Films (`playMedia(id, 'media')`).
   *       - `Reprendre` pour les Séries / Animes lorsqu'au moins
   *         un épisode reste à voir (`playResume(id)` qui résout
   *         `next_resume_episode` côté backend — Requirements 12.1,
   *         12.2, 12.4).
   *       - `Revoir depuis le début` quand tous les épisodes sont
   *         `vu` (`playRewatchFromStart(id)` — Requirement 12.3).
   *
   *   - **Corps** : synopsis, genres (chips), studio / réalisateur,
   *     qualité technique (résolution + codec) et durée pour les
   *     Films.
   *
   *   - **Épisodes (Séries / Animes uniquement)** : liste groupée
   *     par saison, triée `(season ASC, episode ASC)` côté backend
   *     (Requirements 7.3, 7.4 — `list_episodes` garantit l'ordre).
   *     Chaque épisode affiche son statut, un bouton `Lire` (lance
   *     `playMedia(episode.id, 'episode')`) et un lien `Éditer`
   *     vers `/edit-episode/{id}`. Un compteur global
   *     `X / N épisodes vus` est rendu en haut de la liste
   *     (Requirement 8.5 — repris par la propriété 18).
   *
   * Implémentation :
   *   - Le paramètre dynamique `[id]` est lu via `$page.params.id`
   *     puis converti en `number` ; un id non parseable (`NaN`,
   *     valeur ≤ 0) déclenche l'état d'erreur sans tenter d'IPC.
   *   - Au mount, on appelle `getMediaItem(id)` ; pour les Séries /
   *     Animes on enchaîne avec `listEpisodes(id)`. Les épisodes
   *     sont conservés dans l'ordre retourné par le backend.
   *   - Les toggles `Favori` / `À regarder` mettent à jour l'état
   *     local immédiatement après la commande (qui retourne le
   *     nouvel état).
   *   - Si le lecteur externe n'est pas mpv.net, l'événement
   *     `external-player-non-mpv` est observé ailleurs (layout) ;
   *     la fiche affiche un badge dédié quand
   *     `flags.externalPlayerNonMpv` est levé pour l'item courant
   *     (out-of-scope minimal ici — l'affichage du badge global
   *     reste géré par le layout).
   *
   * _Requirements: 7.3, 7.4, 8.5, 12.2, 12.3, 12.4, 15.4_
   */

  import { onDestroy, onMount } from 'svelte';
  import { page } from '$app/stores';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';

  import {
    getMediaItem,
    listEpisodes,
    toggleFavorite,
    toggleToWatch
  } from '$lib/api/commands';
  import {
    launchPlayback,
    launchResume,
    launchRewatchFromStart
  } from '$lib/api/player-window';
  import { toAssetUrl } from '$lib/api/poster-url';
  import {
    EVENT_PLAYBACK_PROGRESS,
    EVENT_PLAYBACK_ENDED,
    type EpisodeItem,
    type MediaItemDetail,
    type PlaybackProgress,
    type Status
  } from '$lib/api/types';

  // ---------------------------------------------------------------------
  // Identifiant de la route — `/detail/[id]`. SvelteKit projette
  // `[id]` en `string` ; on convertit en number et on garde la
  // valeur brute pour les messages d'erreur.
  // ---------------------------------------------------------------------

  $: idParam = $page.params.id ?? '';
  $: id = Number(idParam);

  // ---------------------------------------------------------------------
  // État de la page.
  // ---------------------------------------------------------------------

  /** Fiche complète chargée depuis le backend (null tant que pending). */
  let detail: MediaItemDetail | null = null;

  /**
   * Liste d'épisodes (uniquement pour Series / Anime). `null` tant que
   * la première lecture n'a pas abouti — permet de distinguer
   * « en cours de chargement » de « chargé mais vide ».
   */
  let episodes: EpisodeItem[] | null = null;

  /** `true` tant qu'un appel IPC initial est en cours. */
  let loading = true;
  /** Message d'erreur de chargement (`null` = pas d'erreur). */
  let loadError: string | null = null;
  /** Message d'erreur d'action (lecture, toggle). */
  let actionError: string | null = null;
  /** `true` pendant l'appel d'une action (évite les double-clics). */
  let actionPending = false;

  // ---------------------------------------------------------------------
  // Constantes d'affichage.
  // ---------------------------------------------------------------------

  /** Libellés humains des statuts — alignés avec le reste de l'UI. */
  const STATUS_LABELS: Record<Status, string> = {
    non_vu: 'Non vu',
    en_cours: 'En cours',
    vu: 'Vu'
  };

  // ---------------------------------------------------------------------
  // Helpers — conversion erreur backend en string lisible.
  // ---------------------------------------------------------------------

  function formatError(err: unknown): string {
    if (err instanceof Error) return err.message;
    if (typeof err === 'string') return err;
    return String(err);
  }

  /**
   * Format `Xh YY` ou `YY min` selon la durée. Repris du
   * `HoverOverlay` pour rester cohérent dans toute l'application.
   */
  function formatDuration(totalSeconds: number | null): string | null {
    if (totalSeconds == null || !Number.isFinite(totalSeconds) || totalSeconds <= 0) {
      return null;
    }
    const totalMinutes = Math.floor(totalSeconds / 60);
    const hours = Math.floor(totalMinutes / 60);
    const minutes = totalMinutes % 60;
    if (hours === 0) return `${minutes} min`;
    if (minutes === 0) return `${hours} h`;
    return `${hours} h ${String(minutes).padStart(2, '0')}`;
  }

  /**
   * Calcule le temps restant entre la position courante et la durée
   * totale, formaté de la même façon que `formatDuration`. Retourne
   * `null` si la durée est inconnue, si la progression est nulle, ou
   * si la position dépasse la durée (cas dégénéré, on évite alors
   * d'afficher « 0 min restantes » qui serait trompeur).
   */
  function formatRemaining(
    durationSeconds: number | null,
    progressSeconds: number
  ): string | null {
    if (
      durationSeconds == null ||
      !Number.isFinite(durationSeconds) ||
      durationSeconds <= 0
    ) {
      return null;
    }
    if (!Number.isFinite(progressSeconds) || progressSeconds <= 0) {
      return null;
    }
    const remaining = durationSeconds - progressSeconds;
    if (remaining <= 0) return null;
    return formatDuration(remaining);
  }

  /**
   * Pourcentage de progression entre 0 et 100, ou `null` si
   * indéterminable (durée absente / nulle, progression nulle ou
   * négative). Utilisé pour la barre de progression visuelle.
   */
  function progressPercent(
    durationSeconds: number | null,
    progressSeconds: number
  ): number | null {
    if (
      durationSeconds == null ||
      !Number.isFinite(durationSeconds) ||
      durationSeconds <= 0
    ) {
      return null;
    }
    if (!Number.isFinite(progressSeconds) || progressSeconds <= 0) {
      return null;
    }
    return Math.min(100, Math.max(0, (progressSeconds / durationSeconds) * 100));
  }

  /**
   * URL Tauri locale vers l'affiche, identique à ce que fait
   * `MediaCard.svelte` (cf. tâche 14.1).
   */
  /**
   * URL Tauri locale vers l'affiche : on convertit le chemin absolu
   * persisté en base (`C:\…\posters\media-<id>.<ext>`) vers une URL
   * `asset://localhost/...` consommable par la WebView via
   * `toAssetUrl` (helper qui appelle `convertFileSrc` du SDK Tauri
   * v2 quand le runtime est dispo, sinon retombe sur une URL
   * équivalente pour les tests).
   */
  function posterUrl(detail: MediaItemDetail): string | null {
    return toAssetUrl(detail.poster_path);
  }

  // ---------------------------------------------------------------------
  // Chargement initial.
  // ---------------------------------------------------------------------

  onMount(async () => {
    if (!Number.isFinite(id) || id <= 0) {
      loading = false;
      loadError = `Identifiant invalide : « ${idParam} ».`;
      return;
    }
    try {
      const fetched = await getMediaItem(id);
      detail = fetched;

      // Pour les Séries / Animes, charger la liste d'épisodes triée
      // (Requirements 7.3, 7.4 — l'ordre est garanti par le backend
      // via `list_episodes` qui applique le tri (saison ASC, épisode
      // ASC), cf. propriété 15).
      if (fetched.type === 'Series' || fetched.type === 'Anime') {
        episodes = await listEpisodes(id);
      } else {
        episodes = null;
      }
    } catch (err) {
      loadError = formatError(err);
    } finally {
      loading = false;
    }

    // Live progression : on met à jour la fiche en direct quand mpv
    // pousse un message via le pipe IPC. Pour un Film, on patche
    // `detail.watch_progress_seconds` ; pour un épisode, on patche la
    // ligne correspondante dans `episodes`. À la fin de la lecture
    // (`playback-ended`), on recharge proprement depuis le backend
    // pour être sûr d'afficher le statut final (`vu` notamment).
    try {
      unlistenProgress = await listen<PlaybackProgress>(
        EVENT_PLAYBACK_PROGRESS,
        (event) => {
          const payload = event.payload;
          if (detail === null) return;
          if (payload.kind === 'media' && payload.target_id === detail.id) {
            detail = {
              ...detail,
              watch_progress_seconds: payload.position_seconds
            };
          } else if (
            payload.kind === 'episode' &&
            episodes !== null
          ) {
            const idx = episodes.findIndex((e) => e.id === payload.target_id);
            if (idx >= 0) {
              const next = [...episodes];
              next[idx] = {
                ...next[idx],
                watch_progress_seconds: payload.position_seconds
              };
              episodes = next;
            }
          }
        }
      );
    } catch (err) {
      console.warn('Kyro detail: échec abonnement playback-progress', err);
    }

    try {
      unlistenEnded = await listen(EVENT_PLAYBACK_ENDED, async () => {
        if (detail === null) return;
        try {
          detail = await getMediaItem(detail.id);
          if (detail.type === 'Series' || detail.type === 'Anime') {
            episodes = await listEpisodes(detail.id);
          }
        } catch (err) {
          console.warn('Kyro detail: échec refresh post-playback', err);
        }
      });
    } catch (err) {
      console.warn('Kyro detail: échec abonnement playback-ended', err);
    }
  });

  /** Cleanup des écouteurs Tauri à la destruction de la page. */
  let unlistenProgress: UnlistenFn | null = null;
  let unlistenEnded: UnlistenFn | null = null;
  onDestroy(() => {
    if (unlistenProgress !== null) {
      try {
        unlistenProgress();
      } catch (err) {
        console.warn('Kyro detail: échec détachement playback-progress', err);
      }
    }
    if (unlistenEnded !== null) {
      try {
        unlistenEnded();
      } catch (err) {
        console.warn('Kyro detail: échec détachement playback-ended', err);
      }
    }
  });

  // ---------------------------------------------------------------------
  // Compteur épisodes vus (Requirement 8.5, propriété 18).
  // `total` = card(episodes) ; `watched` = card({e | e.status = 'vu'}).
  // ---------------------------------------------------------------------

  $: total = episodes !== null ? episodes.length : 0;
  $: watched =
    episodes !== null
      ? episodes.filter((e) => e.status === 'vu').length
      : 0;

  /**
   * Détermine le bouton d'action principal selon le type :
   *   - Film ⇒ `Lire` (toujours).
   *   - Series / Anime sans épisode ⇒ pas de bouton (état vide).
   *   - Series / Anime avec tous les épisodes vus ⇒ `Revoir depuis
   *     le début` (Requirement 12.3).
   *   - Series / Anime sinon ⇒ `Reprendre` (Requirement 12.2).
   *
   * On retourne un objet typé pour que le template reste explicite.
   */
  type PrimaryAction =
    | { kind: 'play-film' }
    | { kind: 'resume' }
    | { kind: 'rewatch' }
    | { kind: 'no-episodes' }
    | { kind: 'unknown' };

  function deriveAction(d: MediaItemDetail | null, eps: EpisodeItem[] | null): PrimaryAction {
    if (d === null) return { kind: 'unknown' };
    if (d.type === 'Film') return { kind: 'play-film' };
    if (eps === null) return { kind: 'unknown' };
    if (eps.length === 0) return { kind: 'no-episodes' };
    const allWatched = eps.every((e) => e.status === 'vu');
    return allWatched ? { kind: 'rewatch' } : { kind: 'resume' };
  }

  $: primaryAction = deriveAction(detail, episodes);

  // ---------------------------------------------------------------------
  // Groupement des épisodes par saison pour l'affichage.
  // L'ordre des saisons et l'ordre des épisodes au sein d'une saison
  // sont préservés (le backend garantit déjà le tri).
  // ---------------------------------------------------------------------

  type SeasonGroup = { season: number; episodes: EpisodeItem[] };

  function groupBySeason(eps: EpisodeItem[]): SeasonGroup[] {
    const groups: SeasonGroup[] = [];
    let currentSeason: number | null = null;
    let currentGroup: SeasonGroup | null = null;
    for (const ep of eps) {
      if (currentSeason !== ep.season_number) {
        currentGroup = { season: ep.season_number, episodes: [] };
        groups.push(currentGroup);
        currentSeason = ep.season_number;
      }
      // currentGroup non-null par construction — la première
      // itération crée un groupe avant de pousser l'épisode.
      currentGroup!.episodes.push(ep);
    }
    return groups;
  }

  $: seasonGroups = episodes !== null ? groupBySeason(episodes) : [];

  // ---------------------------------------------------------------------
  // Actions utilisateur.
  // ---------------------------------------------------------------------

  async function handlePrimaryAction(): Promise<void> {
    if (detail === null || actionPending) return;
    actionPending = true;
    actionError = null;
    try {
      switch (primaryAction.kind) {
        case 'play-film':
          await launchPlayback(detail.id, 'media');
          break;
        case 'resume':
          await launchResume(detail.id);
          break;
        case 'rewatch':
          await launchRewatchFromStart(detail.id);
          // Après rewatch, la progression a été remise à zéro côté
          // backend. On rafraîchit la liste d'épisodes pour refléter
          // le nouveau statut local sans recharger toute la page.
          try {
            episodes = await listEpisodes(detail.id);
          } catch (err) {
            // Échec du refresh : non bloquant pour la lecture qui a
            // déjà été lancée. On loggue et on continue.
            console.warn('Kyro: refresh épisodes après rewatch a échoué', err);
          }
          break;
        case 'no-episodes':
        case 'unknown':
          break;
      }
    } catch (err) {
      actionError = formatError(err);
    } finally {
      actionPending = false;
    }
  }

  async function handlePlayEpisode(episodeId: number): Promise<void> {
    if (actionPending) return;
    actionPending = true;
    actionError = null;
    try {
      await launchPlayback(episodeId, 'episode');
    } catch (err) {
      actionError = formatError(err);
    } finally {
      actionPending = false;
    }
  }

  async function handleToggleFavorite(): Promise<void> {
    if (detail === null || actionPending) return;
    actionPending = true;
    actionError = null;
    try {
      const newState = await toggleFavorite(detail.id);
      detail = { ...detail, favorite: newState };
    } catch (err) {
      actionError = formatError(err);
    } finally {
      actionPending = false;
    }
  }

  async function handleToggleToWatch(): Promise<void> {
    if (detail === null || actionPending) return;
    actionPending = true;
    actionError = null;
    try {
      const newState = await toggleToWatch(detail.id);
      detail = { ...detail, to_watch: newState };
    } catch (err) {
      actionError = formatError(err);
    } finally {
      actionPending = false;
    }
  }
</script>

<svelte:head>
  <title>Kyro — Fiche détail</title>
</svelte:head>

<section class="detail-page" aria-label="Fiche détail d'un Media_Item">
  {#if loading}
    <p class="loading" data-testid="detail-loading">Chargement de la fiche…</p>
  {:else if loadError !== null}
    <p class="error" role="alert" data-testid="detail-load-error">
      {loadError}
    </p>
  {:else if detail !== null}
    <!-- ============================================================ -->
    <!-- Bandeau supérieur : affiche, métadonnées et action principale -->
    <!-- ============================================================ -->
    <header class="header" data-testid="detail-header">
      <div class="poster" data-testid="detail-poster">
        {#if posterUrl(detail)}
          <img src={posterUrl(detail)} alt={detail.title} />
        {:else}
          <!--
            Visuel par défaut intégré (Requirement 15.2). SVG inline
            cohérent avec `MediaCard.svelte` pour rester autonome —
            zéro requête réseau (Requirement 6.4 / 17.3).
          -->
          <svg
            class="poster-placeholder"
            xmlns="http://www.w3.org/2000/svg"
            viewBox="0 0 24 24"
            role="img"
            aria-label={detail.title}
          >
            <rect x="2" y="4" width="20" height="16" rx="2" ry="2"
                  fill="none" stroke="currentColor" stroke-width="1.5" />
            <path d="M2 9 H22 M2 15 H22"
                  stroke="currentColor" stroke-width="1" opacity="0.4" />
            <path d="M5 4 V20 M9 4 V20 M15 4 V20 M19 4 V20"
                  stroke="currentColor" stroke-width="0.75" opacity="0.25" />
            <circle cx="12" cy="12" r="2.5"
                    fill="currentColor" opacity="0.6" />
          </svg>
        {/if}
      </div>

      <div class="meta">
        <h1 class="title" data-testid="detail-title">{detail.title}</h1>
        {#if detail.original_title}
          <p class="original-title" data-testid="detail-original-title">
            {detail.original_title}
          </p>
        {/if}

        <ul class="meta-list">
          {#if detail.year != null}
            <li data-testid="detail-year">{detail.year}</li>
          {/if}
          <li data-testid="detail-type">{detail.type}</li>
          <li data-testid="detail-status">{STATUS_LABELS[detail.status]}</li>
          {#if detail.user_rating != null}
            <li data-testid="detail-rating">{detail.user_rating} / 10</li>
          {/if}
          {#if detail.type === 'Film'}
            {@const dur = formatDuration(detail.duration_seconds)}
            {#if dur}
              <li data-testid="detail-duration">{dur}</li>
            {/if}
            {@const remaining = formatRemaining(
              detail.duration_seconds,
              detail.watch_progress_seconds
            )}
            {#if remaining}
              <li class="remaining-chip" data-testid="detail-remaining">
                {remaining} restantes
              </li>
            {/if}
          {/if}
          {#if detail.quality}
            <li data-testid="detail-quality">{detail.quality}</li>
          {/if}
        </ul>

        <div class="actions">
          {#if primaryAction.kind === 'play-film'}
            {@const filmRemaining = formatRemaining(
              detail.duration_seconds,
              detail.watch_progress_seconds
            )}
            <button
              type="button"
              class="primary"
              on:click={handlePrimaryAction}
              disabled={actionPending}
              data-testid="detail-play-button"
            >
              {filmRemaining ? 'Reprendre' : 'Lire'}
            </button>
          {:else if primaryAction.kind === 'resume'}
            <button
              type="button"
              class="primary"
              on:click={handlePrimaryAction}
              disabled={actionPending}
              data-testid="detail-resume-button"
            >
              Reprendre
            </button>
          {:else if primaryAction.kind === 'rewatch'}
            <button
              type="button"
              class="primary"
              on:click={handlePrimaryAction}
              disabled={actionPending}
              data-testid="detail-rewatch-button"
            >
              Revoir depuis le début
            </button>
          {:else if primaryAction.kind === 'no-episodes'}
            <p class="hint" data-testid="detail-no-episodes">
              Aucun épisode disponible pour cette série.
            </p>
          {/if}

          <button
            type="button"
            class="secondary"
            on:click={handleToggleFavorite}
            disabled={actionPending}
            aria-pressed={detail.favorite}
            data-testid="detail-toggle-favorite"
          >
            {detail.favorite ? '★ Favori' : '☆ Favori'}
          </button>

          <button
            type="button"
            class="secondary"
            on:click={handleToggleToWatch}
            disabled={actionPending}
            aria-pressed={detail.to_watch}
            data-testid="detail-toggle-to-watch"
          >
            {detail.to_watch ? '✓ À regarder' : '+ À regarder'}
          </button>

          <a
            class="secondary link-as-button"
            href={`/edit/${detail.id}`}
            data-testid="detail-edit-link"
          >
            Éditer
          </a>
        </div>

        {#if actionError !== null}
          <p class="error" role="alert" data-testid="detail-action-error">
            {actionError}
          </p>
        {/if}

        {#if detail.type === 'Film'}
          {@const filmPercent = progressPercent(
            detail.duration_seconds,
            detail.watch_progress_seconds
          )}
          {#if filmPercent !== null}
            <div
              class="header-progress"
              role="progressbar"
              aria-label="Progression de lecture"
              aria-valuemin="0"
              aria-valuemax="100"
              aria-valuenow={Math.round(filmPercent)}
              style="--progress: {filmPercent}%"
              data-testid="detail-progress"
            ></div>
          {/if}
        {/if}
      </div>
    </header>

    <!-- ============================================================ -->
    <!-- Corps : synopsis, genres, studio.                            -->
    <!-- ============================================================ -->
    <section class="body" aria-label="Informations détaillées">
      {#if detail.synopsis}
        <div class="block" data-testid="detail-synopsis-block">
          <h2>Synopsis</h2>
          <p data-testid="detail-synopsis">{detail.synopsis}</p>
        </div>
      {/if}

      {#if detail.genres.length > 0}
        <div class="block" data-testid="detail-genres-block">
          <h2>Genres</h2>
          <div class="genres">
            {#each detail.genres as genre (genre)}
              <span class="genre-chip" data-testid="detail-genre-chip">
                {genre}
              </span>
            {/each}
          </div>
        </div>
      {/if}

      {#if detail.studio}
        <div class="block" data-testid="detail-studio-block">
          <h2>Studio ou réalisateur</h2>
          <p data-testid="detail-studio">{detail.studio}</p>
        </div>
      {/if}
    </section>

    <!-- ============================================================ -->
    <!-- Liste d'épisodes (Series / Anime uniquement).                -->
    <!-- ============================================================ -->
    {#if detail.type === 'Series' || detail.type === 'Anime'}
      <section class="episodes-section" aria-label="Saisons et épisodes">
        <header class="episodes-header">
          <h2>Épisodes</h2>
          <span
            class="counter"
            data-testid="detail-episodes-counter"
            aria-label={`${watched} sur ${total} épisodes vus`}
          >
            {watched} / {total} épisodes vus
          </span>
        </header>

        {#if episodes !== null && episodes.length === 0}
          <p class="empty" data-testid="detail-episodes-empty">
            Aucun épisode pour cette série.
          </p>
        {:else}
          {#each seasonGroups as group (group.season)}
            <article
              class="season-group"
              data-testid="detail-season-group"
              data-season={group.season}
            >
              <h3 class="season-title">Saison {group.season}</h3>
              <ul class="episode-list" data-testid="detail-episode-list">
                {#each group.episodes as ep (ep.id)}
                  {@const epRemaining = formatRemaining(
                    ep.duration_seconds,
                    ep.watch_progress_seconds
                  )}
                  {@const epPercent = progressPercent(
                    ep.duration_seconds,
                    ep.watch_progress_seconds
                  )}
                  <li
                    class="episode-row"
                    data-testid="detail-episode-row"
                    data-episode-id={ep.id}
                  >
                    <span class="episode-number">
                      S{String(ep.season_number).padStart(2, '0')}E{String(
                        ep.episode_number
                      ).padStart(2, '0')}
                    </span>
                    <span
                      class="episode-status"
                      data-status={ep.status}
                      data-testid="detail-episode-status"
                    >
                      {STATUS_LABELS[ep.status]}{#if epRemaining}
                        <span class="episode-remaining" data-testid="detail-episode-remaining">
                          · {epRemaining} restantes
                        </span>
                      {/if}
                    </span>
                    <div class="episode-actions">
                      <button
                        type="button"
                        class="secondary"
                        on:click={() => handlePlayEpisode(ep.id)}
                        disabled={actionPending || !ep.available}
                        data-testid="detail-episode-play"
                      >
                        {epRemaining ? 'Reprendre' : 'Lire'}
                      </button>
                      <a
                        class="secondary link-as-button"
                        href={`/edit-episode/${ep.id}?seriesId=${detail.id}`}
                        data-testid="detail-episode-edit"
                      >
                        Éditer
                      </a>
                    </div>
                    {#if epPercent !== null}
                      <div
                        class="episode-progress"
                        role="progressbar"
                        aria-label="Progression de lecture"
                        aria-valuemin="0"
                        aria-valuemax="100"
                        aria-valuenow={Math.round(epPercent)}
                        style="--progress: {epPercent}%"
                      ></div>
                    {/if}
                  </li>
                {/each}
              </ul>
            </article>
          {/each}
        {/if}
      </section>
    {/if}
  {/if}
</section>

<style>
  .detail-page {
    display: flex;
    flex-direction: column;
    gap: var(--space-5, 24px);
    max-width: 960px;
    margin: 0 auto;
    padding: var(--space-4, 16px);
    color: var(--text-primary);
    font-family: var(--font-family);
  }

  .header {
    display: grid;
    grid-template-columns: minmax(180px, 220px) 1fr;
    gap: var(--space-5, 24px);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-card, 12px);
    padding: var(--space-5, 24px);
    box-shadow: var(--shadow-card);
  }

  .poster {
    aspect-ratio: 2 / 3;
    background: var(--bg-secondary);
    border-radius: var(--radius-card, 12px);
    overflow: hidden;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .poster img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .poster-placeholder {
    width: 50%;
    height: 50%;
    color: var(--text-secondary);
  }

  .meta {
    display: flex;
    flex-direction: column;
    gap: var(--space-3, 12px);
  }

  .title {
    margin: 0;
    font-size: 1.75rem;
    font-weight: 600;
    color: var(--text-primary);
  }

  .original-title {
    margin: 0;
    color: var(--text-secondary);
    font-style: italic;
  }

  .meta-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3, 12px);
    color: var(--text-secondary);
    font-size: 0.9rem;
  }

  .meta-list li {
    background: var(--bg-secondary);
    border: 1px solid var(--border);
    border-radius: var(--radius-control, 8px);
    padding: var(--space-1, 4px) var(--space-2, 8px);
  }

  /* Chip dédiée à la progression : accent visuel discret pour
     distinguer « X min restantes » des autres métadonnées (durée
     totale, qualité, etc.). */
  .meta-list li.remaining-chip {
    background: rgba(53, 132, 228, 0.14);
    border-color: var(--accent);
    color: var(--text-primary);
    font-weight: 600;
  }

  /* Barre de progression sous l'en-tête (Film). Couleur d'accent,
     fond transparent, séparée des actions par un peu d'espace. */
  .header-progress {
    margin-top: var(--space-2, 8px);
    width: 100%;
    height: 4px;
    background: var(--bg-secondary);
    border-radius: 2px;
    overflow: hidden;
    position: relative;
  }

  .header-progress::after {
    content: '';
    position: absolute;
    inset: 0;
    width: var(--progress, 0%);
    background: var(--accent);
    transition: width var(--duration-base, 200ms) var(--ease-out, ease-out);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2, 8px);
    align-items: center;
  }

  .hint {
    margin: 0;
    color: var(--text-secondary);
    font-size: 0.9rem;
  }

  button,
  .link-as-button {
    padding: var(--space-2, 8px) var(--space-4, 16px);
    border-radius: var(--radius-control, 8px);
    border: 1px solid var(--border);
    cursor: pointer;
    font-family: var(--font-family);
    font-size: 0.95rem;
    color: var(--text-primary);
    background: var(--surface);
    text-decoration: none;
    display: inline-block;
  }

  button.primary {
    background: var(--accent);
    color: var(--text-primary);
    border-color: var(--accent);
  }

  button.primary:hover:not(:disabled) {
    background: var(--accent-hover);
    border-color: var(--accent-hover);
  }

  button.secondary:hover:not(:disabled),
  .link-as-button:hover {
    border-color: var(--accent);
  }

  button:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .body {
    display: flex;
    flex-direction: column;
    gap: var(--space-4, 16px);
  }

  .block {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-card, 12px);
    padding: var(--space-4, 16px);
  }

  .block h2 {
    margin: 0 0 var(--space-2, 8px) 0;
    font-size: 1rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-secondary);
  }

  .block p {
    margin: 0;
    line-height: 1.5;
  }

  .genres {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2, 8px);
  }

  .genre-chip {
    background: var(--bg-secondary);
    border: 1px solid var(--border);
    border-radius: var(--radius-control, 8px);
    padding: var(--space-1, 4px) var(--space-2, 8px);
    font-size: 0.85rem;
    color: var(--text-primary);
  }

  /* Section épisodes. */

  .episodes-section {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-card, 12px);
    padding: var(--space-4, 16px);
    display: flex;
    flex-direction: column;
    gap: var(--space-4, 16px);
  }

  .episodes-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: var(--space-3, 12px);
    flex-wrap: wrap;
  }

  .episodes-header h2 {
    margin: 0;
    font-size: 1.1rem;
    color: var(--text-primary);
  }

  .counter {
    font-size: 0.95rem;
    font-weight: 600;
    color: var(--accent);
  }

  .season-group {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
  }

  .season-title {
    margin: 0;
    font-size: 1rem;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .episode-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
  }

  .episode-row {
    display: grid;
    grid-template-columns: auto 1fr auto;
    grid-template-rows: auto auto;
    gap: var(--space-3, 12px);
    align-items: center;
    background: var(--bg-secondary);
    border: 1px solid var(--border);
    border-radius: var(--radius-control, 8px);
    padding: var(--space-2, 8px) var(--space-3, 12px);
  }

  /* Barre de progression par épisode — occupe toute la largeur de la
     row sur la deuxième ligne du grid pour ne pas écraser le statut
     ni les actions. Affichée uniquement quand il reste du temps. */
  .episode-progress {
    grid-column: 1 / -1;
    grid-row: 2;
    width: 100%;
    height: 3px;
    background: var(--bg-primary);
    border-radius: 2px;
    overflow: hidden;
    position: relative;
  }

  .episode-progress::after {
    content: '';
    position: absolute;
    inset: 0;
    width: var(--progress, 0%);
    background: var(--accent);
    transition: width var(--duration-base, 200ms) var(--ease-out, ease-out);
  }

  .episode-remaining {
    color: var(--accent);
    font-weight: 600;
    margin-left: var(--space-1, 4px);
  }

  .episode-number {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 0.9rem;
    color: var(--text-primary);
  }

  .episode-status {
    color: var(--text-secondary);
    font-size: 0.85rem;
  }

  .episode-status[data-status='vu'] {
    color: var(--accent);
    font-weight: 600;
  }

  .episode-actions {
    display: flex;
    gap: var(--space-2, 8px);
  }

  .empty,
  .loading {
    margin: 0;
    color: var(--text-secondary);
  }

  .error {
    margin: 0;
    color: #e01b24;
    font-size: 0.9rem;
  }
</style>
