<script lang="ts">
  /**
   * MediaCard — carte d'affichage d'un Media_Item dans la grille de
   * la bibliothèque (requirement 15.1, 15.2).
   *
   * - Affiche l'affiche locale via `tauri://localhost/posters/<filename>`
   *   (le backend copie les affiches importées dans
   *   `%APPDATA%\Kyro\posters\<id>.<ext>` et persiste `poster_path` dans
   *   `media_item` ; cf. tâche 7.7).
   * - Si aucune affiche n'est associée (`poster_path` null/absent), on
   *   affiche un visuel SVG intégré (requirement 15.2) — aucune requête
   *   réseau, aucun asset externe.
   * - Affiche le titre et l'année (si définie).
   *
   * Le hover overlay (note, durée/épisodes, statut) est implémenté
   * dans un composant séparé `HoverOverlay.svelte` (tâche 14.2).
   *
   * Style : Adwaita Dark via les variables CSS du thème
   * (`frontend/src/styles/theme.css`).
   *
   * UX additions :
   *   - Fade-in de l'image à la fin du load (`onload`) pour éviter
   *     le saut visuel quand l'affiche arrive un cycle après le DOM.
   *   - Hover lift (-4px), élévation accrue, tilt latéral discret du
   *     poster pour un feeling « tangible ».
   *   - Bordure de la carte qui s'éclaire à l'accent au focus du
   *     lien parent (cf. `.card-link:focus-visible` dans CategoryGrid).
   *
   * _Requirements: 15.1, 15.2_
   */
  import type { MediaItem } from '$lib/api/types';

  /**
   * Type structurel attendu par la carte. On part du DTO
   * `MediaItem` (listing) et on rajoute `poster_path` en optionnel :
   * dans le DTO de listing actuel ce champ n'est pas projeté, mais
   * la fiche détail (`MediaItemDetail`) le contient. Le composant
   * accepte donc indistinctement les deux (et tout objet structurellement
   * compatible utilisé dans les tests).
   */
  type MediaCardItem = MediaItem & { poster_path?: string | null };

  export let item: MediaCardItem;

  /**
   * URL Tauri locale vers l'affiche. Le custom protocol `tauri://`
   * est exposé par `tauri.conf.json` ; le préfixe `posters/` est
   * mappé vers `%APPDATA%\Kyro\posters\` côté backend.
   *
   * `poster_path` contient typiquement un chemin absolu Windows
   * (`C:\Users\…\AppData\Roaming\app.kyro\posters\media-<id>.<ext>`)
   * tel que persisté par le backend `import_poster`. On le convertit
   * en URL `asset://localhost/...` consommable par la WebView via
   * `convertFileSrc` du SDK Tauri v2 — cette fonction encode le
   * chemin et applique le préfixe attendu par le custom protocol
   * `asset` configuré dans `tauri.conf.json` (`security.assetProtocol`).
   */
  import { toAssetUrl } from '$lib/api/poster-url';
  $: posterUrl = toAssetUrl(item.poster_path);

  /**
   * `true` quand l'image a fini de se charger. On applique alors un
   * `opacity: 1` qui produit un fade-in. Tant que `false`, l'image
   * est invisible — le fond du conteneur (`bg-secondary`) reste
   * visible et évite un flash blanc avant rendu.
   */
  let imgLoaded = false;

  function onImgLoad() {
    imgLoaded = true;
  }

  function onImgError() {
    // L'image n'a pas pu être chargée (fichier supprimé / corrompu).
    // On ne fait rien ici : `imgLoaded` reste `false` donc l'image
    // demeure invisible et le fond du conteneur fait office de
    // placeholder. Pas de placeholder SVG ici car on ne veut pas
    // double-empiler les visuels avec l'`<img>` qui occupe la place.
  }

  // Réinitialise `imgLoaded` quand la prop `posterUrl` change (par
  // exemple après un import d'affiche pour cet item). Sans ce reset
  // le fade-in se rejouerait sur la nouvelle image au moment du
  // `onload`, mais on partirait d'un état déjà à `true` et donc pas
  // d'animation visible.
  let lastUrl: string | null = null;
  $: if (posterUrl !== lastUrl) {
    imgLoaded = false;
    lastUrl = posterUrl;
  }
</script>

<article class="media-card" data-testid="media-card">
  <div class="poster">
    {#if posterUrl}
      <img
        src={posterUrl}
        alt={item.title}
        loading="lazy"
        class:loaded={imgLoaded}
        on:load={onImgLoad}
        on:error={onImgError}
      />
    {:else}
      <!--
        Visuel par défaut intégré (requirement 15.2). SVG inline pour
        éviter toute requête réseau et garder le composant autonome.
        Icône générique « film/écran » neutre, teintée via les
        couleurs du thème.
      -->
      <svg
        class="poster-placeholder"
        xmlns="http://www.w3.org/2000/svg"
        viewBox="0 0 24 24"
        role="img"
        aria-label={item.title}
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
    <h3 class="title" title={item.title}>{item.title}</h3>
    {#if item.year != null}
      <span class="year">{item.year}</span>
    {/if}
  </div>
</article>

<style>
  .media-card {
    position: relative;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
    box-shadow: var(--shadow-card);
    overflow: hidden;
    color: var(--text-primary);
    font-family: var(--font-family);
    transition:
      transform var(--duration-base) var(--ease-out),
      box-shadow var(--duration-base) var(--ease-out),
      border-color var(--duration-base) var(--ease-out);
    will-change: transform;
  }

  .media-card:hover {
    transform: translateY(-4px);
    box-shadow: var(--shadow-card-hover);
    border-color: var(--accent);
  }

  /*
    L'effet hover doit aussi répondre quand le lien parent est
    focusé au clavier — on utilise un sélecteur global pour
    remonter au .card-link.
  */
  :global(.card-link:focus-visible) .media-card {
    transform: translateY(-2px);
    border-color: var(--accent);
    box-shadow: var(--shadow-card-hover);
  }

  .poster {
    position: relative;
    width: 100%;
    aspect-ratio: 2 / 3;
    background: var(--bg-secondary);
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
  }

  .poster img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
    opacity: 0;
    transform: scale(1.02);
    transition:
      opacity var(--duration-slow) var(--ease-out),
      transform var(--duration-slow) var(--ease-out);
  }

  .poster img.loaded {
    opacity: 1;
    transform: scale(1);
  }

  /* Léger zoom au hover pour rendre la carte « vivante ». */
  .media-card:hover .poster img.loaded {
    transform: scale(1.04);
  }

  .poster-placeholder {
    width: 40%;
    height: 40%;
    color: var(--text-secondary);
    transition: color var(--duration-base) var(--ease-out);
  }

  .media-card:hover .poster-placeholder {
    color: var(--accent);
  }

  .meta {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-3) var(--space-3) var(--space-4);
  }

  .title {
    margin: 0;
    font-size: 0.95rem;
    font-weight: 600;
    color: var(--text-primary);
    line-height: 1.3;
    overflow: hidden;
    text-overflow: ellipsis;
    display: -webkit-box;
    line-clamp: 2;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
  }

  .year {
    font-size: 0.85rem;
    color: var(--text-secondary);
    transition: color var(--duration-fast) var(--ease-out);
  }

  .media-card:hover .year {
    color: var(--text-primary);
  }
</style>
