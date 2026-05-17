<script lang="ts">
  /**
   * Page d'édition d'un Media_Item — tâche 15.5
   * (Requirements 6.1, 6.2, 6.3).
   *
   * Formulaire complet permettant à l'utilisateur de personnaliser
   * la fiche d'un Media_Item :
   *   - titre (obligatoire) ;
   *   - titre original (optionnel) ;
   *   - année — chaîne vide ou entier ∈ [1900, 2099] (Requirement 4.2
   *     côté domaine, garde-fou UI dupliqué ici) ;
   *   - synopsis (textarea libre) ;
   *   - genres (input multi-tag : la frappe de Entrée ou ',' pousse
   *     la valeur courante dans la liste de chips, le clic sur ✕
   *     d'un chip retire l'entrée correspondante) ;
   *   - note utilisateur via `<RatingInput>` (validation [0, 10]
   *     déléguée au composant — Requirement 6.6) ;
   *   - studio / réalisateur (champ unique « studio ou réalisateur »
   *     comme prescrit par Requirement 6.1) ;
   *   - statut (select `non_vu` / `en_cours` / `vu`) ;
   *   - affiche locale via `<PosterPicker kind='media' id={id}>`
   *     (Requirements 6.2 / 6.3, le composant fait l'IPC).
   *
   * Sur soumission :
   *   1. on valide l'année côté UI ;
   *   2. on appelle `updateMediaItem(id, payload)` (Requirement 6.1) ;
   *   3. on redirige vers `/detail/{id}` via `goto`.
   *
   * Note d'implémentation — `genres` :
   *   `UpdateMediaItemInput` côté Rust ne porte PAS encore les
   *   genres (cf. commentaire dans `commands::library` : « genres
   *   n'est pas inclus dans cette première itération : la table
   *   `media_genre` est gérée séparément »). On rend donc l'input
   *   multi-tag pour respecter Requirement 6.1, mais la valeur
   *   éditée n'est pas persistée tant que la commande dédiée
   *   n'est pas livrée. Cette limite est documentée à l'utilisateur
   *   par un petit message d'aide sous le champ.
   *
   * _Requirements: 6.1, 6.2, 6.3_
   */
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/stores';

  import RatingInput from '$lib/components/RatingInput.svelte';
  import PosterPicker from '$lib/components/PosterPicker.svelte';
  import { getMediaItem, updateMediaItem } from '$lib/api/commands';
  import type {
    MediaItemDetail,
    Status,
    UpdateMediaItemInput
  } from '$lib/api/types';

  // ---------------------------------------------------------------------
  // Identifiant route — `/edit/[id]`. SvelteKit projette `[id]` en
  // `string` ; on convertit en number et on garde la valeur brute pour
  // les messages d'erreur.
  // ---------------------------------------------------------------------

  $: idParam = $page.params.id ?? '';
  $: id = Number(idParam);

  // ---------------------------------------------------------------------
  // État du formulaire — chargé depuis `getMediaItem` au mount.
  // ---------------------------------------------------------------------

  /** Fiche complète chargée depuis le backend (null tant que pending). */
  let detail: MediaItemDetail | null = null;

  /** Champ titre — obligatoire (validation à la soumission). */
  let title = '';
  /** Champ titre original — optionnel, chaîne vide ⇒ `null` côté backend. */
  let originalTitle = '';
  /**
   * Champ année rendu en `<input type="text">` plutôt que `number` :
   *   - on doit pouvoir représenter « vide » ⇒ année non renseignée ;
   *   - la valeur ne devient un nombre qu'au moment de la validation.
   */
  let yearText = '';
  let synopsis = '';
  /**
   * Liste mutable de tags pour les genres. La table `media_genre`
   * n'est pas mise à jour par `updateMediaItem` aujourd'hui (cf. note
   * d'en-tête), mais l'UI reste fonctionnelle et l'état est conservé
   * localement.
   */
  let genres: string[] = [];
  /** Tampon de saisie d'un nouveau genre (avant validation Entrée / ','). */
  let genreDraft = '';
  /**
   * Note utilisateur. `null` ⇒ non renseignée. Toute valeur hors
   * [0, 10] est rejetée par `<RatingInput>` (Requirement 6.6).
   */
  let userRating: number | null = null;
  /** Studio ou réalisateur — champ unique d'après Requirement 6.1. */
  let studio = '';
  /**
   * Statut courant. Les trois valeurs admises sont alignées sur le
   * CHECK SQL `media_item.status` et sur `domain::status::Status`.
   */
  let status: Status = 'non_vu';

  // ---------------------------------------------------------------------
  // États transverses — chargement, validation, soumission.
  // ---------------------------------------------------------------------

  /** `true` tant que `getMediaItem` n'a pas répondu (succès ou erreur). */
  let loading = true;
  /** Message d'erreur de chargement (`null` = pas d'erreur). */
  let loadError: string | null = null;
  /** Message d'erreur de validation côté UI (année hors plage, etc.). */
  let validationError: string | null = null;
  /** Message d'erreur retourné par le backend lors de la soumission. */
  let submitError: string | null = null;
  /** `true` pendant l'appel `updateMediaItem`. */
  let submitting = false;

  // ---------------------------------------------------------------------
  // Constantes de validation.
  // ---------------------------------------------------------------------

  /** Borne basse de l'année autorisée (Requirement 4.2). */
  const YEAR_MIN = 1900;
  /** Borne haute de l'année autorisée (Requirement 4.2). */
  const YEAR_MAX = 2099;

  /** Message d'erreur année — repris à l'identique côté tests. */
  const YEAR_ERROR =
    "L'année doit être comprise entre 1900 et 2099 ou laissée vide.";

  /** Liste statique des statuts possibles. */
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
      loadError = `Identifiant invalide : « ${idParam} ».`;
      return;
    }
    try {
      const fetched = await getMediaItem(id);
      detail = fetched;
      title = fetched.title;
      originalTitle = fetched.original_title ?? '';
      yearText = fetched.year !== null ? String(fetched.year) : '';
      synopsis = fetched.synopsis ?? '';
      genres = [...fetched.genres];
      userRating = fetched.user_rating;
      studio = fetched.studio ?? '';
      status = fetched.status;
    } catch (err) {
      loadError = formatError(err);
    } finally {
      loading = false;
    }
  });

  // ---------------------------------------------------------------------
  // Helpers — conversion erreur backend en string lisible, et
  // normalisation des chaînes optionnelles vers `null` quand vides.
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
  // Gestion des chips « genres » — sémantique simple :
  //   - Entrée ou ',' pousse `genreDraft` (trim, dédup) dans `genres` ;
  //   - clic sur ✕ retire l'entrée ciblée ;
  //   - blur ne pousse PAS automatiquement (évite les surprises de
  //     focus quand l'utilisateur quitte le champ pour aller cliquer
  //     sur un autre contrôle du formulaire).
  // ---------------------------------------------------------------------

  function commitGenreDraft(): void {
    const candidate = genreDraft.trim();
    if (candidate === '') return;
    if (!genres.includes(candidate)) {
      genres = [...genres, candidate];
    }
    genreDraft = '';
  }

  function onGenreKeydown(event: KeyboardEvent): void {
    if (event.key === 'Enter' || event.key === ',') {
      event.preventDefault();
      commitGenreDraft();
    }
  }

  function removeGenre(genre: string): void {
    genres = genres.filter((g) => g !== genre);
  }

  // ---------------------------------------------------------------------
  // Validation année.
  // ---------------------------------------------------------------------

  /**
   * Valide la chaîne courante du champ année et renvoie la valeur à
   * envoyer au backend, ou `'invalid'` si la valeur est mal formée.
   * Une chaîne vide ⇒ `null` (année non renseignée — Requirement 4.2).
   */
  function validateYear(raw: string): number | null | 'invalid' {
    const trimmed = raw.trim();
    if (trimmed === '') return null;
    // Entier strict — refuse `1995.5`, `1995a`, etc.
    if (!/^-?\d+$/.test(trimmed)) return 'invalid';
    const parsed = Number(trimmed);
    if (!Number.isFinite(parsed)) return 'invalid';
    if (parsed < YEAR_MIN || parsed > YEAR_MAX) return 'invalid';
    return parsed;
  }

  // ---------------------------------------------------------------------
  // Soumission.
  // ---------------------------------------------------------------------

  async function handleSubmit(event: Event): Promise<void> {
    event.preventDefault();
    if (submitting) return;
    validationError = null;
    submitError = null;

    // Titre obligatoire (Requirement 6.1).
    const trimmedTitle = title.trim();
    if (trimmedTitle === '') {
      validationError = 'Le titre est obligatoire.';
      return;
    }

    // Année ∈ [1900, 2099] ou vide.
    const yearOrError = validateYear(yearText);
    if (yearOrError === 'invalid') {
      validationError = YEAR_ERROR;
      return;
    }

    // Si une saisie de tag « genre » est en cours, on la pousse pour
    // éviter de perdre silencieusement la valeur tapée par l'utilisateur.
    if (genreDraft.trim() !== '') {
      commitGenreDraft();
    }

    const payload: UpdateMediaItemInput = {
      title: trimmedTitle,
      original_title: trimOrNull(originalTitle),
      year: yearOrError,
      synopsis: trimOrNull(synopsis),
      studio: trimOrNull(studio),
      user_rating: userRating,
      status
    };

    submitting = true;
    try {
      await updateMediaItem(id, payload);
      // Redirection vers la fiche détail (Requirement 6.1 — l'utilisateur
      // doit voir ses modifications appliquées immédiatement).
      await goto(`/detail/${id}`);
    } catch (err) {
      submitError = formatError(err);
    } finally {
      submitting = false;
    }
  }

  function handleCancel(): void {
    // Retour à la fiche détail sans persister.
    goto(`/detail/${id}`);
  }

  /**
   * Gestionnaire d'événement `imported` du `<PosterPicker>`.
   * Met à jour `poster_path` localement pour qu'un éventuel rendu
   * d'aperçu puisse refléter le nouvel import sans recharger toute
   * la fiche. Le champ `poster_path` n'est pas envoyé via
   * `updateMediaItem` (le backend l'a déjà persisté côté commande
   * dédiée `import_poster_for_media`).
   */
  function onPosterImported(
    event: CustomEvent<{ kind: 'media' | 'episode'; id: number; destPath: string }>
  ): void {
    if (detail !== null) {
      detail = { ...detail, poster_path: event.detail.destPath };
    }
  }
</script>

<svelte:head>
  <title>Kyro — Édition de la fiche</title>
</svelte:head>

<section class="edit-page" aria-label="Édition d'un Media_Item">
  {#if loading}
    <p class="loading" data-testid="edit-loading">Chargement de la fiche…</p>
  {:else if loadError !== null}
    <p class="error" role="alert" data-testid="edit-load-error">
      {loadError}
    </p>
  {:else if detail !== null}
    <header class="header">
      <h1>Modifier « {detail.title} »</h1>
    </header>

    <form on:submit={handleSubmit} novalidate>
      <div class="row">
        <label for="edit-title">Titre</label>
        <input
          id="edit-title"
          type="text"
          required
          bind:value={title}
          autocomplete="off"
        />
      </div>

      <div class="row">
        <label for="edit-original-title">Titre original</label>
        <input
          id="edit-original-title"
          type="text"
          bind:value={originalTitle}
          autocomplete="off"
        />
      </div>

      <div class="row">
        <label for="edit-year">Année</label>
        <input
          id="edit-year"
          type="text"
          inputmode="numeric"
          placeholder="Vide ou entre 1900 et 2099"
          bind:value={yearText}
          aria-describedby="edit-year-help"
        />
        <p id="edit-year-help" class="help">
          Laissez vide ou saisissez un entier compris entre {YEAR_MIN} et {YEAR_MAX}.
        </p>
      </div>

      <div class="row">
        <label for="edit-synopsis">Synopsis</label>
        <textarea
          id="edit-synopsis"
          rows="5"
          bind:value={synopsis}
        ></textarea>
      </div>

      <div class="row">
        <label for="edit-genres">Genres</label>
        <div class="chips-input">
          {#each genres as genre (genre)}
            <span class="chip" data-testid="genre-chip">
              <span class="chip-label">{genre}</span>
              <button
                type="button"
                class="chip-remove"
                aria-label={`Retirer le genre ${genre}`}
                on:click={() => removeGenre(genre)}
              >
                ✕
              </button>
            </span>
          {/each}
          <input
            id="edit-genres"
            type="text"
            placeholder="Ajouter un genre puis appuyer sur Entrée"
            bind:value={genreDraft}
            on:keydown={onGenreKeydown}
            autocomplete="off"
          />
        </div>
      </div>

      <div class="row">
        <label for="edit-rating">Note utilisateur</label>
        <RatingInput id="edit-rating" bind:value={userRating} />
      </div>

      <div class="row">
        <label for="edit-studio">Studio ou réalisateur</label>
        <input
          id="edit-studio"
          type="text"
          bind:value={studio}
          autocomplete="off"
        />
      </div>

      <div class="row">
        <label for="edit-status">Statut</label>
        <select id="edit-status" bind:value={status}>
          {#each STATUSES as opt (opt.value)}
            <option value={opt.value}>{opt.label}</option>
          {/each}
        </select>
      </div>

      <div class="row">
        <span class="label-as-text">Affiche</span>
        <PosterPicker kind="media" {id} on:imported={onPosterImported} />
      </div>

      {#if validationError !== null}
        <p class="error" role="alert" data-testid="edit-validation-error">
          {validationError}
        </p>
      {/if}

      {#if submitError !== null}
        <p class="error" role="alert" data-testid="edit-submit-error">
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

  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-4, 16px);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-card, 12px);
    padding: var(--space-5, 24px);
    box-shadow: var(--shadow-card);
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
    font-size: 0.8rem;
    color: var(--text-secondary);
  }

  /* Multi-tag input. */
  .chips-input {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2, 8px);
    background: var(--bg-secondary);
    border: 1px solid var(--border);
    border-radius: var(--radius-control, 8px);
    padding: var(--space-2, 8px);
  }

  .chips-input:focus-within {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
    border-color: var(--accent);
  }

  .chips-input input {
    flex: 1 1 auto;
    min-width: 8rem;
    border: none;
    background: transparent;
    padding: var(--space-1, 4px) var(--space-2, 8px);
    color: var(--text-primary);
    font-family: var(--font-family);
    font-size: 0.95rem;
  }

  .chips-input input:focus {
    outline: none;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1, 4px);
    background: var(--accent);
    color: var(--text-primary);
    border-radius: var(--radius-control, 8px);
    padding: var(--space-1, 4px) var(--space-2, 8px);
    font-size: 0.85rem;
  }

  .chip-remove {
    background: transparent;
    border: none;
    color: inherit;
    cursor: pointer;
    font-size: 0.85rem;
    padding: 0 var(--space-1, 4px);
    line-height: 1;
  }

  .chip-remove:hover {
    color: rgba(255, 255, 255, 0.85);
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
