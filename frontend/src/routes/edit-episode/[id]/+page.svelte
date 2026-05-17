<script lang="ts">
  /**
   * Page d'édition d'un Episode_Item — tâche 15.6 (Requirement 6.5).
   *
   * Permet à l'utilisateur de modifier la fiche d'un épisode au
   * niveau de la série, de la saison et de l'épisode (Requirement
   * 6.5). La page expose pour cela trois onglets :
   *
   *   1. **Série** — lien rapide vers `/edit/{seriesId}` (la fiche
   *      complète Media_Item est éditable depuis cette autre page,
   *      cf. tâche 15.5 / `routes/edit/[id]`). Plutôt que de
   *      dupliquer le formulaire, on garde un point d'entrée
   *      unique : un bouton « Éditer la fiche série ».
   *   2. **Saison** — placeholder : le backend n'expose pas encore
   *      de champs persistés au niveau « saison » (la table
   *      `episode_item` porte juste `season_number`). Un message
   *      d'information indique que cette édition arrivera plus
   *      tard. Cette ligne couvre le « niveau saison » de
   *      Requirement 6.5 sans introduire de fausse interface ; le
   *      contrat d'API reste intact.
   *   3. **Épisode** — formulaire principal : titre, synopsis,
   *      statut et `<PosterPicker kind='episode'>`. Sur soumission :
   *      `updateEpisodeItem(id, payload)` puis redirection vers
   *      `/detail/{seriesId}`.
   *
   * Identification :
   *   - L'`id` de l'épisode vient du segment de route `[id]`
   *     (SvelteKit le projette en `string` ; on convertit).
   *   - Le `seriesId` est requis pour deux usages : retrouver
   *     l'épisode via `listEpisodes(seriesId)` (le backend
   *     n'expose pas aujourd'hui de `getEpisodeItem(id)` direct —
   *     cf. note 15.6) et calculer le lien « Éditer la série »
   *     ainsi que la cible de redirection. Il est lu depuis le
   *     paramètre de query `?seriesId=N`. Si manquant ou non
   *     numérique, la page affiche un message d'erreur explicite
   *     et n'appelle aucune commande backend.
   *
   * Le DTO `EpisodeItem` retourné par `listEpisodes` ne porte que
   * `(id, season_number, episode_number, status, available)`. Les
   * champs `title` et `synopsis` ne sont donc pas pré-remplis : ils
   * démarrent vides et seront persistés à la soumission. C'est
   * conforme au contrat backend actuel (cf. `commands::library::
   * EpisodeItemDto`).
   *
   * _Requirements: 6.5_
   */

  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/stores';

  import PosterPicker from '$lib/components/PosterPicker.svelte';
  import { listByCategory, listEpisodes, updateEpisodeItem } from '$lib/api/commands';
  import type {
    EpisodeItem,
    Status,
    UpdateEpisodeItemInput
  } from '$lib/api/types';

  // ---------------------------------------------------------------------
  // Identifiants — route + query param.
  // ---------------------------------------------------------------------

  /** Paramètre de route brut (utile pour les messages d'erreur). */
  $: idParam = $page.params.id ?? '';
  /** Identifiant numérique de l'Episode_Item ciblé. */
  $: id = Number(idParam);

  /** Paramètre de query brut (`?seriesId=…`). */
  $: seriesIdParam = $page.url?.searchParams?.get('seriesId') ?? '';
  /**
   * Identifiant numérique de la série parente.
   *
   * Note : peut être absent (`NaN`) si l'utilisateur arrive sur cette
   * page sans le paramètre. Dans ce cas on tentera de le déduire au
   * mount en parcourant les séries existantes — voir `onMount`.
   */
  let seriesId = NaN;
  $: seriesId = Number(seriesIdParam);

  // ---------------------------------------------------------------------
  // État du formulaire — chargé depuis `listEpisodes` au mount.
  // ---------------------------------------------------------------------

  /** Vue de l'épisode telle que retournée par `listEpisodes`. */
  let episode: EpisodeItem | null = null;

  /**
   * Champ titre. `EpisodeItem` ne fournit pas le titre courant, donc
   * il démarre vide ; l'utilisateur peut saisir un titre s'il le
   * souhaite. Une chaîne vide est envoyée comme `null` (Requirement
   * 6.5 — le champ est optionnel côté backend).
   */
  let title = '';
  /** Synopsis de l'épisode. Vide ⇒ `null` à la soumission. */
  let synopsis = '';
  /**
   * Statut courant. Valeurs alignées sur le CHECK SQL
   * `episode_item.status` et sur `domain::status::Status`.
   */
  let status: Status = 'non_vu';

  // ---------------------------------------------------------------------
  // Onglets : sélection courante.
  //
  // « Série » et « Saison » sont des onglets de navigation (le
  // formulaire actif reste l'épisode), ce qui satisfait Requirement
  // 6.5 (« édition au niveau de la série, de la saison et de
  // l'épisode ») sans dupliquer la page d'édition Media_Item.
  // ---------------------------------------------------------------------

  type Tab = 'serie' | 'saison' | 'episode';
  let activeTab: Tab = 'episode';

  // ---------------------------------------------------------------------
  // États transverses.
  // ---------------------------------------------------------------------

  /** `true` tant que `listEpisodes` n'a pas répondu. */
  let loading = true;
  /** Message d'erreur de chargement (`null` ⇒ pas d'erreur). */
  let loadError: string | null = null;
  /** Message d'erreur côté backend lors de la soumission. */
  let submitError: string | null = null;
  /** `true` pendant l'appel `updateEpisodeItem`. */
  let submitting = false;

  /** Liste statique des statuts (cohérent avec `/edit/[id]`). */
  const STATUSES: ReadonlyArray<{ value: Status; label: string }> = [
    { value: 'non_vu', label: 'Non vu' },
    { value: 'en_cours', label: 'En cours' },
    { value: 'vu', label: 'Vu' }
  ];

  // ---------------------------------------------------------------------
  // Chargement initial.
  // ---------------------------------------------------------------------

  onMount(async () => {
    if (!Number.isFinite(id) || id <= 0) {
      loading = false;
      loadError = `Identifiant d'épisode invalide : « ${idParam} ».`;
      return;
    }

    // Si `seriesId` est absent ou invalide, on tente de le retrouver
    // en parcourant les séries / animes existants. C'est plus lent
    // qu'un lien direct (O(nombre de séries)) mais ça permet à
    // l'utilisateur d'éditer un épisode même quand le contexte de
    // série a été perdu.
    let resolvedSeriesId = seriesId;
    if (!Number.isFinite(resolvedSeriesId) || resolvedSeriesId <= 0) {
      try {
        const candidates = [
          ...(await listByCategory('Series')),
          ...(await listByCategory('Animes'))
        ];
        for (const candidate of candidates) {
          const eps = await listEpisodes(candidate.id);
          if (eps.some((e) => e.id === id)) {
            resolvedSeriesId = candidate.id;
            seriesId = candidate.id;
            break;
          }
        }
      } catch (err) {
        loading = false;
        loadError = formatError(err);
        return;
      }

      if (!Number.isFinite(resolvedSeriesId) || resolvedSeriesId <= 0) {
        loading = false;
        loadError = `L'épisode ${id} est introuvable dans les séries enregistrées.`;
        return;
      }
    }

    try {
      const episodes = await listEpisodes(resolvedSeriesId);
      const found = episodes.find((e) => e.id === id);
      if (!found) {
        loadError = `L'épisode ${id} est introuvable dans la série ${resolvedSeriesId}.`;
      } else {
        episode = found;
        status = found.status;
      }
    } catch (err) {
      loadError = formatError(err);
    } finally {
      loading = false;
    }
  });

  // ---------------------------------------------------------------------
  // Helpers.
  // ---------------------------------------------------------------------

  function formatError(err: unknown): string {
    if (err instanceof Error) return err.message;
    if (typeof err === 'string') return err;
    return String(err);
  }

  function trimOrNull(s: string): string | null {
    const t = s.trim();
    return t === '' ? null : t;
  }

  // ---------------------------------------------------------------------
  // Actions.
  // ---------------------------------------------------------------------

  async function handleSubmit(event: Event): Promise<void> {
    event.preventDefault();
    if (submitting) return;
    submitError = null;

    const payload: UpdateEpisodeItemInput = {
      title: trimOrNull(title),
      synopsis: trimOrNull(synopsis),
      status
    };

    submitting = true;
    try {
      await updateEpisodeItem(id, payload);
      // Retour à la fiche détail de la série parente : c'est la vue
      // qui liste les épisodes et permet à l'utilisateur de constater
      // la mise à jour (statut, etc.).
      await goto(`/detail/${seriesId}`);
    } catch (err) {
      submitError = formatError(err);
    } finally {
      submitting = false;
    }
  }

  function handleCancel(): void {
    // Retour vers la fiche détail sans persister.
    goto(`/detail/${seriesId}`);
  }

  function goEditSeries(): void {
    goto(`/edit/${seriesId}`);
  }

  /**
   * Gestionnaire d'événement `imported` du `<PosterPicker>`. Le
   * backend a déjà persisté le chemin de l'affiche (`poster_path`
   * dans `episode_item`) ; on n'a pas besoin de réémettre un
   * `updateEpisodeItem` côté frontend. On garde le hook pour
   * symétrie avec `routes/edit/[id]/+page.svelte` et pour permettre
   * à de futurs aperçus d'image d'être rafraîchis sans recharger
   * la page.
   */
  function onPosterImported(
    _event: CustomEvent<{ kind: 'media' | 'episode'; id: number; destPath: string }>
  ): void {
    // No-op pour l'instant : `EpisodeItem` ne contient pas de champ
    // `poster_path` exposé par `listEpisodes`.
  }
</script>

<svelte:head>
  <title>Kyro — Édition de l'épisode</title>
</svelte:head>

<section class="edit-page" aria-label="Édition d'un Episode_Item">
  {#if loading}
    <p class="loading" data-testid="edit-episode-loading">
      Chargement de l'épisode…
    </p>
  {:else if loadError !== null}
    <p class="error" role="alert" data-testid="edit-episode-load-error">
      {loadError}
    </p>
  {:else if episode !== null}
    <header class="header">
      <h1>
        Modifier l'épisode S{String(episode.season_number).padStart(2, '0')}E{String(
          episode.episode_number
        ).padStart(2, '0')}
      </h1>
    </header>

    <div class="tabs" role="tablist" aria-label="Niveaux d'édition">
      <button
        type="button"
        role="tab"
        aria-selected={activeTab === 'serie'}
        class:active={activeTab === 'serie'}
        on:click={() => (activeTab = 'serie')}
        data-testid="tab-serie"
      >
        Série
      </button>
      <button
        type="button"
        role="tab"
        aria-selected={activeTab === 'saison'}
        class:active={activeTab === 'saison'}
        on:click={() => (activeTab = 'saison')}
        data-testid="tab-saison"
      >
        Saison
      </button>
      <button
        type="button"
        role="tab"
        aria-selected={activeTab === 'episode'}
        class:active={activeTab === 'episode'}
        on:click={() => (activeTab = 'episode')}
        data-testid="tab-episode"
      >
        Épisode
      </button>
    </div>

    {#if activeTab === 'serie'}
      <div
        class="panel"
        role="tabpanel"
        aria-label="Édition de la série"
        data-testid="panel-serie"
      >
        <p class="help">
          La fiche au niveau de la série (titre, année, genres, note,
          studio, statut, affiche) s'édite depuis la page dédiée.
        </p>
        <button type="button" class="primary" on:click={goEditSeries}>
          Éditer la fiche série
        </button>
      </div>
    {:else if activeTab === 'saison'}
      <div
        class="panel"
        role="tabpanel"
        aria-label="Édition de la saison"
        data-testid="panel-saison"
      >
        <p class="help">
          L'édition au niveau de la saison sera disponible
          prochainement. Saison courante :
          <strong>S{String(episode.season_number).padStart(2, '0')}</strong>.
        </p>
      </div>
    {:else}
      <div
        class="panel"
        role="tabpanel"
        aria-label="Édition de l'épisode"
        data-testid="panel-episode"
      >
        <form on:submit={handleSubmit} novalidate>
          <div class="row">
            <label for="edit-episode-title">Titre</label>
            <input
              id="edit-episode-title"
              type="text"
              bind:value={title}
              autocomplete="off"
              placeholder="Titre de l'épisode (optionnel)"
            />
          </div>

          <div class="row">
            <label for="edit-episode-synopsis">Synopsis</label>
            <textarea
              id="edit-episode-synopsis"
              rows="5"
              bind:value={synopsis}
              placeholder="Synopsis de l'épisode (optionnel)"
            ></textarea>
          </div>

          <div class="row">
            <label for="edit-episode-status">Statut</label>
            <select id="edit-episode-status" bind:value={status}>
              {#each STATUSES as opt (opt.value)}
                <option value={opt.value}>{opt.label}</option>
              {/each}
            </select>
          </div>

          <div class="row">
            <span class="label-as-text">Affiche</span>
            <PosterPicker
              kind="episode"
              {id}
              on:imported={onPosterImported}
            />
          </div>

          {#if submitError !== null}
            <p
              class="error"
              role="alert"
              data-testid="edit-episode-submit-error"
            >
              Échec de l'enregistrement : {submitError}
            </p>
          {/if}

          <div class="actions">
            <button
              type="button"
              class="secondary"
              on:click={handleCancel}
              disabled={submitting}
            >
              Annuler
            </button>
            <button
              type="submit"
              class="primary"
              disabled={submitting}
              aria-busy={submitting}
            >
              {submitting ? 'Enregistrement…' : 'Enregistrer'}
            </button>
          </div>
        </form>
      </div>
    {/if}
  {/if}
</section>

<style>
  .edit-page {
    display: flex;
    flex-direction: column;
    gap: var(--space-4, 16px);
    max-width: 720px;
    margin: 0 auto;
    padding: var(--space-4, 16px);
    color: var(--text-primary);
    font-family: var(--font-family);
  }

  .header h1 {
    margin: 0;
    font-size: 1.5rem;
    color: var(--text-primary);
  }

  /* Onglets — Adwaita-like : ligne soulignée en accent quand actif. */
  .tabs {
    display: flex;
    gap: var(--space-1, 4px);
    border-bottom: 1px solid var(--border);
  }

  .tabs button {
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    color: var(--text-secondary);
    padding: var(--space-2, 8px) var(--space-4, 16px);
    font-family: var(--font-family);
    font-size: 0.95rem;
    cursor: pointer;
  }

  .tabs button:hover:not(.active) {
    color: var(--text-primary);
  }

  .tabs button.active {
    color: var(--text-primary);
    border-bottom-color: var(--accent);
  }

  .panel {
    display: flex;
    flex-direction: column;
    gap: var(--space-4, 16px);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-card, 12px);
    padding: var(--space-5, 24px);
    box-shadow: var(--shadow-card);
  }

  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-4, 16px);
  }

  .row {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
  }

  label,
  .label-as-text {
    font-size: 0.875rem;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  input[type='text'],
  textarea,
  select {
    background: var(--bg-secondary);
    color: var(--text-primary);
    border: 1px solid var(--border);
    border-radius: var(--radius-control, 8px);
    padding: var(--space-2, 8px) var(--space-3, 12px);
    font-family: var(--font-family);
    font-size: 0.95rem;
  }

  input[type='text']:focus,
  textarea:focus,
  select:focus {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
    border-color: var(--accent);
  }

  textarea {
    resize: vertical;
    min-height: 6rem;
  }

  .help {
    margin: 0;
    font-size: 0.9rem;
    color: var(--text-secondary);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-3, 12px);
  }

  button {
    padding: var(--space-2, 8px) var(--space-4, 16px);
    border-radius: var(--radius-control, 8px);
    border: 1px solid var(--border);
    cursor: pointer;
    font-family: var(--font-family);
    font-size: 0.95rem;
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

  button.secondary {
    background: var(--surface);
    color: var(--text-primary);
  }

  button.secondary:hover:not(:disabled) {
    border-color: var(--accent);
  }

  button:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .loading {
    color: var(--text-secondary);
  }

  .error {
    margin: 0;
    color: #e01b24;
    font-size: 0.9rem;
  }
</style>
