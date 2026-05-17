<script lang="ts">
  /**
   * `CategoryGrid` — grille adaptive d'une catégorie de la
   * bibliothèque (tâche 15.2).
   *
   * Rendue par les routes `/films`, `/series`, `/animes`. Le
   * composant est paramétré par la `Category` à charger ; cela
   * évite de dupliquer la logique de mount + grille dans trois
   * pages quasi-identiques.
   *
   * UX additions :
   *   - **Skeleton loaders** pendant le fetch initial : 12 cartes
   *     fantômes affichées avec un shimmer animé (`@keyframes
   *     kyro-shimmer` défini dans `theme.css`). Donne un feedback
   *     immédiat de structure, plus agréable qu'un texte
   *     « Chargement… » et plus pro qu'un blanc.
   *   - **Stagger d'apparition** : chaque carte de la grille apparaît
   *     avec un léger décalage temporel (animation-delay basé sur
   *     l'index), produisant une cascade visuelle douce.
   *   - **Empty state illustré** : icône SVG inline + message,
   *     remplace le simple `<p>` du précédent design.
   *
   * _Requirements: 7.1, 7.2, 15.1, 15.3_
   */
  import { onMount } from 'svelte';
  import { writable, type Writable } from 'svelte/store';

  import MediaCard from './MediaCard.svelte';
  import { listByCategory, setLastCategory } from '$lib/api/commands';
  import { items } from '$lib/stores/library';
  import { filteredItems } from '$lib/stores/filtered';
  import type { Category } from '$lib/api/types';

  /** Catégorie à charger (`Films`, `Series` ou `Animes`). */
  export let category: Category;

  /**
   * Titre affiché en tête de page. La forme française
   * (« Films », « Séries », « Animes ») diffère de la valeur
   * `Category` (toujours en anglais côté backend / SQL CHECK), donc
   * on laisse l'appelant fournir le libellé.
   */
  export let title: string;

  /**
   * Message à afficher quand la grille est vide. Permet aux pages
   * d'utiliser un libellé adapté (« Aucun film », « Aucune série »,
   * « Aucun anime ») cohérent avec la catégorie.
   */
  export let emptyMessage: string;

  /**
   * Identifiant data-testid utilisé pour les éléments racines
   * (grille, vide, erreur, loading) — facilite l'écriture de tests
   * unitaires par catégorie sans collision dans le DOM.
   */
  export let testIdPrefix: string;

  /**
   * État de chargement local. `null` tant que `listByCategory` n'a
   * pas répondu, ce qui distingue « en attente » de « répondu mais
   * liste vide ».
   */
  const loaded: Writable<boolean> = writable<boolean>(false);

  /** Message d'erreur backend, le cas échéant. */
  const errorMessage: Writable<string | null> = writable<string | null>(null);

  /**
   * Nombre de squelettes affichés pendant le chargement initial. 12
   * couvre une rangée typique en 1080p (~6 colonnes × 2 rangées).
   */
  const SKELETON_COUNT = 12;

  /**
   * Borne supérieure du décalage cumulé pour le stagger. Au-delà,
   * l'animation se sentirait laggy : un grand jeu d'items continuera
   * de s'animer mais les premiers seront déjà installés quand les
   * derniers commenceront leur fade-in. C'est volontaire — on
   * privilégie la fluidité au stagger parfait.
   */
  const STAGGER_MAX_INDEX = 20;
  const STAGGER_STEP_MS = 25;

  function staggerDelay(idx: number): string {
    const i = Math.min(idx, STAGGER_MAX_INDEX);
    return `${i * STAGGER_STEP_MS}ms`;
  }

  onMount(async () => {
    // Mémorisation de la catégorie courante : permet à `/` de
    // restaurer la dernière vue au prochain démarrage. On effectue
    // l'appel sans bloquer le chargement de la liste — un échec
    // de persistance ne doit pas empêcher l'utilisateur de
    // consulter ses médias.
    try {
      await setLastCategory(category);
    } catch (err) {
      console.warn('Kyro: échec setLastCategory', err);
    }

    try {
      const result = await listByCategory(category);
      items.set(result);
    } catch (err) {
      console.warn(`Kyro: échec listByCategory(${category})`, err);
      errorMessage.set(
        err instanceof Error ? err.message : String(err ?? 'erreur inconnue')
      );
      // Reset à liste vide pour ne pas afficher la liste de la
      // catégorie précédemment consultée si l'IPC échoue.
      items.set([]);
    } finally {
      loaded.set(true);
    }
  });
</script>

<section class="page" aria-labelledby="{testIdPrefix}-title">
  <h1 id="{testIdPrefix}-title" class="page-title">{title}</h1>

  {#if $errorMessage !== null}
    <div
      class="empty-state error"
      role="alert"
      data-testid="{testIdPrefix}-error"
    >
      <svg viewBox="0 0 24 24" class="empty-icon" aria-hidden="true">
        <circle cx="12" cy="12" r="10" fill="none" stroke="currentColor" stroke-width="1.5" />
        <path d="M12 7v6" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
        <circle cx="12" cy="17" r="1.1" fill="currentColor" />
      </svg>
      <p>Impossible de charger la liste : {$errorMessage}</p>
    </div>
  {:else if !$loaded}
    <!--
      Skeleton grid : on utilise la même structure que la grille
      réelle pour que la mise en page ne change pas brutalement
      quand les vraies cartes arrivent.
    -->
    <div
      class="grid"
      data-testid="{testIdPrefix}-loading"
      aria-busy="true"
      aria-label="Chargement en cours"
    >
      {#each Array(SKELETON_COUNT) as _, i}
        <div class="skeleton-card" style="--stagger-delay: {staggerDelay(i)}">
          <div class="skeleton-poster shimmer"></div>
          <div class="skeleton-line shimmer skeleton-line--title"></div>
          <div class="skeleton-line shimmer skeleton-line--year"></div>
        </div>
      {/each}
    </div>
  {:else if $filteredItems.length === 0}
    <div class="empty-state" data-testid="{testIdPrefix}-empty">
      <svg viewBox="0 0 24 24" class="empty-icon" aria-hidden="true">
        <rect x="3" y="5" width="18" height="14" rx="2" ry="2"
              fill="none" stroke="currentColor" stroke-width="1.5" />
        <path d="M3 10h18" stroke="currentColor" stroke-width="1.2" opacity="0.5" />
        <circle cx="8" cy="14.5" r="1" fill="currentColor" opacity="0.6" />
        <circle cx="12" cy="14.5" r="1" fill="currentColor" opacity="0.4" />
        <circle cx="16" cy="14.5" r="1" fill="currentColor" opacity="0.3" />
      </svg>
      <p>{emptyMessage}</p>
    </div>
  {:else}
    <div
      class="grid"
      data-testid="{testIdPrefix}-grid"
    >
      {#each $filteredItems as item, idx (item.id)}
        <a
          class="card-link"
          href={`/detail/${item.id}`}
          aria-label={item.title}
          data-testid="{testIdPrefix}-card-link"
          style="--stagger-delay: {staggerDelay(idx)}"
        >
          <MediaCard {item} />
        </a>
      {/each}
    </div>
  {/if}
</section>

<style>
  .page {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    animation: kyro-fade-in var(--duration-base) var(--ease-standard);
  }

  .page-title {
    margin: 0;
    font-size: 1.5rem;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  /*
    Grille adaptive (requirement 15.3) — `auto-fill` ajuste le
    nombre de colonnes à la largeur de la fenêtre, chaque colonne
    fait au minimum 180 px de large et au maximum 1 fr (partage
    équitable de l'espace).
  */
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: var(--space-4);
  }

  /*
    Lien englobant : neutre visuellement mais conservant un focus
    visible (accessibilité clavier). Le styling de la carte elle-
    même est intégralement délégué à `<MediaCard>`.
  */
  .card-link {
    color: inherit;
    text-decoration: none;
    border-radius: var(--radius-card);
    outline: none;
    /* Stagger d'apparition : chaque carte fade-in avec un retard
       proportionnel à son index dans la grille. */
    animation: kyro-fade-in-up var(--duration-slow) var(--ease-out) backwards;
    animation-delay: var(--stagger-delay, 0ms);
  }

  /*
    Skeletons — placeholders visuellement cohérents avec la vraie
    carte (mêmes proportions, même radius, même padding).
  */
  .skeleton-card {
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
    box-shadow: var(--shadow-card);
    overflow: hidden;
    animation: kyro-fade-in var(--duration-base) var(--ease-standard) backwards;
    animation-delay: var(--stagger-delay, 0ms);
  }

  .skeleton-poster {
    width: 100%;
    aspect-ratio: 2 / 3;
  }

  .skeleton-line {
    height: 14px;
    margin: var(--space-2) var(--space-3);
    border-radius: 6px;
  }

  .skeleton-line--title {
    margin-top: var(--space-3);
    width: 80%;
  }

  .skeleton-line--year {
    width: 30%;
    height: 10px;
    margin-bottom: var(--space-4);
  }

  /*
    Shimmer — gradient horizontal qui défile de gauche à droite
    grâce à l'animation `kyro-shimmer` définie dans theme.css.
  */
  .shimmer {
    background: linear-gradient(
      90deg,
      var(--skeleton-base) 0%,
      var(--skeleton-highlight) 50%,
      var(--skeleton-base) 100%
    );
    background-size: 200% 100%;
    animation: kyro-shimmer 1.6s linear infinite;
  }

  /* Empty state illustré (vide ou erreur). */
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    padding: var(--space-6) var(--space-4);
    color: var(--text-secondary);
    text-align: center;
    animation: kyro-fade-in-up var(--duration-slow) var(--ease-out);
  }

  .empty-state p {
    margin: 0;
    font-size: 0.95rem;
    line-height: 1.5;
    max-width: 32rem;
  }

  .empty-state.error {
    color: var(--text-primary);
  }

  .empty-state.error .empty-icon {
    color: var(--color-error);
  }

  .empty-icon {
    width: 64px;
    height: 64px;
    color: var(--text-secondary);
    opacity: 0.7;
  }
</style>
