<script lang="ts">
  /**
   * Page « À regarder » — liste filtrée des Media_Item dont
   * `à_regarder = true` (tâche 15.3).
   *
   * Comportement :
   *   1. Au montage, appelle `listToWatch()` (qui invoque la
   *      commande Rust `list_to_watch`, requirement 11.5) et stocke
   *      le résultat dans un writable local. Le store est local à la
   *      page pour ne pas écraser la liste de la dernière catégorie
   *      consultée si l'utilisateur revient sur `/films`, `/series`
   *      ou `/animes`.
   *   2. Rend la même grille adaptive que les pages catégorie
   *      (`grid-template-columns: repeat(auto-fill, minmax(180px, 1fr))`,
   *      `gap: var(--space-4)`) — cohérence visuelle exigée par le
   *      design (requirement 15.3).
   *   3. Chaque `<MediaCard>` est enveloppée dans un lien
   *      `<a href="/detail/{id}">` qui amène vers la fiche détail.
   *   4. État vide : message bienveillant invitant l'utilisateur à
   *      ajouter des médias à sa liste à regarder depuis la grille
   *      ou la vue détail (requirement 11.3).
   *   5. Erreur : message d'erreur affiché en place de la grille
   *      sans crasher la page.
   *
   * _Requirements: 11.5_
   */
  import { onMount } from 'svelte';
  import { writable, type Writable } from 'svelte/store';

  import MediaCard from '$lib/components/MediaCard.svelte';
  import { listToWatch } from '$lib/api/commands';
  import type { MediaItem } from '$lib/api/types';

  /**
   * Liste locale des items à regarder. `null` tant que la première
   * lecture n'a pas abouti — permet de distinguer « en cours de
   * chargement » de « chargé mais vide » (état vide affiché
   * uniquement après succès de l'appel).
   */
  const toWatch: Writable<MediaItem[] | null> = writable<MediaItem[] | null>(null);

  /** Message d'erreur renvoyé par le backend, le cas échéant. */
  const errorMessage: Writable<string | null> = writable<string | null>(null);

  onMount(async () => {
    try {
      const items = await listToWatch();
      toWatch.set(items);
    } catch (err) {
      console.warn('Kyro: échec listToWatch', err);
      errorMessage.set(
        err instanceof Error ? err.message : String(err ?? 'erreur inconnue')
      );
      toWatch.set([]);
    }
  });
</script>

<section class="page" aria-labelledby="a-regarder-title">
  <h1 id="a-regarder-title" class="page-title">À regarder</h1>

  {#if $errorMessage !== null}
    <p class="error" role="alert" data-testid="a-regarder-error">
      Impossible de charger la liste : {$errorMessage}
    </p>
  {:else if $toWatch === null}
    <p class="loading" data-testid="a-regarder-loading">Chargement…</p>
  {:else if $toWatch.length === 0}
    <p class="empty" data-testid="a-regarder-empty">
      Votre liste « À regarder » est vide. Ajoutez un média à votre liste
      depuis sa fiche ou la grille pour le retrouver ici.
    </p>
  {:else}
    <div class="grid" data-testid="a-regarder-grid">
      {#each $toWatch as item (item.id)}
        <a
          class="card-link"
          href={`/detail/${item.id}`}
          aria-label={item.title}
          data-testid="a-regarder-card-link"
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
  }

  .page-title {
    margin: 0;
    font-size: 1.5rem;
    font-weight: 600;
    color: var(--text-primary);
  }

  /*
    Grille adaptive identique aux pages catégorie : `auto-fill` +
    `minmax(180px, 1fr)` permet à la grille de se réorganiser au
    redimensionnement de la fenêtre (requirement 15.3).
  */
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: var(--space-4);
  }

  /*
    Le lien englobant doit rester visuellement neutre (pas de
    soulignement, pas de couleur de lien) et conserver l'accessibilité
    clavier (focus visible). On délègue toute la mise en forme à
    `<MediaCard>` lui-même.
  */
  .card-link {
    color: inherit;
    text-decoration: none;
    border-radius: var(--radius-card);
    outline: none;
  }

  .card-link:focus-visible {
    box-shadow: 0 0 0 2px var(--accent);
  }

  .empty,
  .loading,
  .error {
    margin: 0;
    color: var(--text-secondary);
    font-size: 0.95rem;
    line-height: 1.5;
  }

  .error {
    color: var(--text-primary);
  }
</style>
