<!--
  FilterPanel — bandeau de filtrage de la bibliothèque (requirement 14).

  Affiche quatre sections :
    1. Genres   — sélection MULTIPLE, sémantique d'UNION (au moins un
                  genre commun suffit pour qu'un Media_Item passe le
                  filtre — requirement 14.3).
    2. Années   — sélection UNIQUE (replace), combinée par
                  INTERSECTION avec les autres dimensions (14.2).
    3. Types    — sélection UNIQUE (Film / Série / Anime).
    4. Statuts  — sélection UNIQUE (non_vu / en_cours / vu).

  Le compteur en tête (« X éléments visibles ») reflète en permanence
  le nombre de Media_Item visibles après filtrage et recherche
  combinés, conformément au requirement 14.5.

  Les valeurs disponibles pour `Genres` et `Années` sont dérivées
  dynamiquement du store `items` ; `Types` et `Statuts` sont des
  ensembles fixes alignés sur les types DTO `MediaType` et `Status`.

  Le bouton « Réinitialiser » remet les quatre dimensions à `[]`
  (requirement 14.4).

  Le compteur d'éléments visibles est branché sur le store dérivé
  `filteredItems` exposé par `$lib/stores/filtered` (tâche 13.3) :
  il reflète automatiquement l'état combiné `items` × `filters` ×
  `searchQuery`.

  _Requirements: 14.1, 14.2, 14.3, 14.4, 14.5_
-->
<script lang="ts">
  import { derived } from 'svelte/store';

  import FilterChip from './FilterChip.svelte';
  import { filters, items } from '$lib/stores/library';
  import { filteredItems } from '$lib/stores/filtered';
  import type { MediaType, Status } from '$lib/api/types';

  // ---------------------------------------------------------------------
  // Constantes — dimensions à valeurs fixes.
  // ---------------------------------------------------------------------

  const TYPE_VALUES: readonly MediaType[] = ['Film', 'Series', 'Anime'];
  const TYPE_LABELS: Record<MediaType, string> = {
    Film: 'Film',
    Series: 'Série',
    Anime: 'Anime'
  };

  const STATUS_VALUES: readonly Status[] = ['non_vu', 'en_cours', 'vu'];
  const STATUS_LABELS: Record<Status, string> = {
    non_vu: 'Non vu',
    en_cours: 'En cours',
    vu: 'Vu'
  };

  // ---------------------------------------------------------------------
  // Dérivés UI : valeurs disponibles depuis `items`.
  // ---------------------------------------------------------------------

  /** Liste triée des genres présents dans la bibliothèque courante. */
  const availableGenres = derived(items, ($items) => {
    const seen = new Set<string>();
    for (const it of $items) {
      for (const g of it.genres) {
        seen.add(g);
      }
    }
    return Array.from(seen).sort((a, b) => a.localeCompare(b));
  });

  /** Liste triée (décroissante) des années connues parmi `items`. */
  const availableYears = derived(items, ($items) => {
    const seen = new Set<number>();
    for (const it of $items) {
      if (it.year !== null) {
        seen.add(it.year);
      }
    }
    return Array.from(seen).sort((a, b) => b - a);
  });

  // ---------------------------------------------------------------------
  // Compteur (requirement 14.5) — branché sur `filteredItems` (13.3).
  // ---------------------------------------------------------------------

  const visibleCount = derived(filteredItems, ($items) => $items.length);

  // ---------------------------------------------------------------------
  // Mutations du store `filters`.
  // ---------------------------------------------------------------------

  function toggleGenre(genre: string): void {
    filters.update((f) => {
      const has = f.genres.includes(genre);
      return {
        ...f,
        genres: has ? f.genres.filter((g) => g !== genre) : [...f.genres, genre]
      };
    });
  }

  function toggleYear(year: number): void {
    // Sélection unique : on bascule entre [year] et [].
    filters.update((f) => ({
      ...f,
      years: f.years.length === 1 && f.years[0] === year ? [] : [year]
    }));
  }

  function toggleType(type: MediaType): void {
    filters.update((f) => ({
      ...f,
      types: f.types.length === 1 && f.types[0] === type ? [] : [type]
    }));
  }

  function toggleStatus(status: Status): void {
    filters.update((f) => ({
      ...f,
      statuses:
        f.statuses.length === 1 && f.statuses[0] === status ? [] : [status]
    }));
  }

  function resetFilters(): void {
    filters.set({ genres: [], years: [], types: [], statuses: [] });
  }
</script>

<section class="filter-panel" aria-label="Filtres de la bibliothèque">
  <header class="header">
    <p class="count" data-testid="visible-count">
      {$visibleCount}
      {$visibleCount === 1 ? 'élément visible' : 'éléments visibles'}
    </p>
    <button type="button" class="reset" on:click={resetFilters}>
      Réinitialiser
    </button>
  </header>

  {#if $availableGenres.length > 0}
    <fieldset class="group" data-testid="group-genres">
      <legend>Genres</legend>
      <div class="chips">
        {#each $availableGenres as genre (genre)}
          <FilterChip
            label={genre}
            selected={$filters.genres.includes(genre)}
            on:toggle={() => toggleGenre(genre)}
          />
        {/each}
      </div>
    </fieldset>
  {/if}

  {#if $availableYears.length > 0}
    <fieldset class="group" data-testid="group-years">
      <legend>Années</legend>
      <div class="chips">
        {#each $availableYears as year (year)}
          <FilterChip
            label={String(year)}
            selected={$filters.years.includes(year)}
            on:toggle={() => toggleYear(year)}
          />
        {/each}
      </div>
    </fieldset>
  {/if}

  <fieldset class="group" data-testid="group-types">
    <legend>Types</legend>
    <div class="chips">
      {#each TYPE_VALUES as type (type)}
        <FilterChip
          label={TYPE_LABELS[type]}
          selected={$filters.types.includes(type)}
          on:toggle={() => toggleType(type)}
        />
      {/each}
    </div>
  </fieldset>

  <fieldset class="group" data-testid="group-statuses">
    <legend>Statuts</legend>
    <div class="chips">
      {#each STATUS_VALUES as status (status)}
        <FilterChip
          label={STATUS_LABELS[status]}
          selected={$filters.statuses.includes(status)}
          on:toggle={() => toggleStatus(status)}
        />
      {/each}
    </div>
  </fieldset>
</section>

<style>
  .filter-panel {
    display: flex;
    flex-direction: column;
    gap: var(--space-4, 1rem);
    padding: var(--space-4, 1rem);
    background: var(--bg-secondary);
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
    color: var(--text-primary);
    font-family: var(--font-family);
  }

  .header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3, 0.5rem);
  }

  .count {
    margin: 0;
    font-size: 0.95rem;
    color: var(--text-primary);
  }

  .reset {
    padding: var(--space-2, 0.25rem) var(--space-3, 0.5rem);
    background: var(--surface);
    color: var(--text-primary);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    cursor: pointer;
  }

  .reset:hover {
    border-color: var(--accent);
  }

  .group {
    border: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 0.25rem);
  }

  .group legend {
    padding: 0;
    margin: 0;
    font-size: 0.85rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-secondary);
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2, 0.25rem);
  }
</style>
