<script lang="ts">
  /**
   * Layout racine de Kyro (tâche 15.1).
   *
   * Rôle :
   *   1. Importer la feuille de tokens Adwaita Dark (`theme.css`)
   *      pour qu'elle soit appliquée à l'ensemble de l'application,
   *      y compris au tout premier rendu (requirement 16.1) ;
   *   2. Monter la chrome globale : `<Sidebar>` à gauche, colonne
   *      principale avec `<TopBar>` en haut puis les bandeaux
   *      d'état dégradé (`ffprobe-missing` jaune, `backend-degraded`
   *      rouge, `scan-progress` bleu) et enfin le `<slot />` du
   *      routeur SvelteKit ;
   *   3. Au montage : appliquer le thème persisté (`loadTheme`),
   *      brancher les écouteurs d'événements Tauri
   *      (`setupEventListeners`) avant que le moindre événement ne
   *      puisse être manqué, puis amorcer le store `settings`
   *      (`initSettings`) avec l'agrégat persistant.
   *
   * Le `<ToastContainer>` est monté ici (une seule fois pour toute
   * l'app) afin que n'importe quel composant puisse pousser un toast
   * via `import { toast } from '$lib/stores/toast'`.
   *
   * Les changements de route déclenchent une transition de fade
   * subtile sur le `<main>` (animation CSS sur l'attribut `key`
   * synthétisé depuis `$page.url.pathname`).
   *
   * _Requirements: 5.4, 18.4, 16.1_
   */
  import { onDestroy, onMount } from 'svelte';
  import type { UnlistenFn } from '@tauri-apps/api/event';
  import { fade } from 'svelte/transition';
  import { page } from '$app/stores';

  import Sidebar from '$lib/components/Sidebar.svelte';
  import TopBar from '$lib/components/TopBar.svelte';
  import ToastContainer from '$lib/components/ToastContainer.svelte';
  import EmbeddedPlayer from '$lib/components/EmbeddedPlayer.svelte';
  import { setupEventListeners } from '$lib/api/events';
  import { initSettings } from '$lib/stores/init';
  import { flags, scanProgress } from '$lib/stores/library';
  import { loadTheme } from '$lib/stores/theme';

  // Import statique de la feuille de tokens Adwaita Dark : SvelteKit
  // l'inclut dans le bundle final pour qu'elle soit appliquée à
  // l'ensemble de l'application dès le premier rendu (requirement 16.1).
  import '../styles/theme.css';

  /** Cleanup retourné par `setupEventListeners` ; appelé au destroy. */
  let unlistenAll: UnlistenFn | null = null;

  /**
   * Permet à l'utilisateur de masquer le bandeau `ffprobe-missing`
   * pour la session courante. Le store `flags.ffprobeMissing` reste
   * à `true` (utilisé par TopBar pour le badge persistant) ; seul
   * le bandeau d'information est dissimulé. Le bandeau rouge
   * `backend-degraded` n'est volontairement pas dismissable :
   * l'utilisateur doit voir la proposition de relance tant que
   * l'état n'est pas résolu.
   */
  let ffprobeBannerDismissed = false;

  onMount(async () => {
    // Initialisation séquentielle :
    //   1. `loadTheme` applique le thème persisté (évite un FOUC).
    //   2. `setupEventListeners` est attendu pour ne perdre aucun
    //      événement émis pendant la lecture des settings.
    //   3. `initSettings` lit l'agrégat persistant.
    try {
      await loadTheme();
    } catch (err) {
      console.warn('Kyro: échec loadTheme', err);
    }

    try {
      unlistenAll = await setupEventListeners();
    } catch (err) {
      console.warn('Kyro: échec setupEventListeners', err);
    }

    await initSettings();
  });

  onDestroy(() => {
    if (unlistenAll !== null) {
      try {
        unlistenAll();
      } catch (err) {
        console.warn('Kyro: échec cleanup listeners', err);
      }
      unlistenAll = null;
    }
  });

  /**
   * Recharge la fenêtre Tauri pour laisser le backend retenter une
   * initialisation propre. Volontairement basique : un `location.reload()`
   * suffit côté WebView (Tauri ne tue pas le backend). L'utilisateur
   * conserve toujours la possibilité de fermer / relancer
   * manuellement l'application.
   */
  function reloadApp(): void {
    if (typeof window !== 'undefined') {
      window.location.reload();
    }
  }

  /**
   * `routeKey` est utilisé comme clé de transition pour le `<main>`.
   * Quand le pathname change, Svelte détruit puis recrée le slot,
   * déclenchant l'animation `transition:fade`. On strippe les query
   * params pour ne pas re-déclencher la transition à chaque update
   * du `?seriesId=…` ou autres mutations sans changement de page.
   */
  $: routeKey = $page.url.pathname;
</script>

<div class="app">
  <Sidebar />

  <div class="main">
    <TopBar />

    <div class="banners" role="region" aria-label="Bandeaux d'état">
      {#if $flags.ffprobeMissing && !ffprobeBannerDismissed}
        <div
          class="banner banner-warning"
          role="status"
          data-testid="banner-ffprobe-missing"
        >
          <span class="banner-text">
            ffprobe est introuvable. Les métadonnées techniques (durée,
            codec, résolution) ne peuvent pas être extraites tant qu'un
            chemin valide n'est pas configuré dans
            <a href="/settings">Paramètres</a>.
          </span>
          <button
            type="button"
            class="banner-close"
            aria-label="Masquer ce bandeau"
            on:click={() => (ffprobeBannerDismissed = true)}
          >
            ✕
          </button>
        </div>
      {/if}

      {#if $flags.backendDegraded}
        <div
          class="banner banner-error"
          role="alert"
          data-testid="banner-backend-degraded"
        >
          <span class="banner-text">
            Kyro est en état dégradé : le backend ne répond plus
            correctement. Relancez l'application pour réessayer.
          </span>
          <button
            type="button"
            class="banner-action"
            on:click={reloadApp}
          >
            Relancer
          </button>
        </div>
      {/if}

      {#if $scanProgress !== null}
        <div
          class="banner banner-info"
          role="status"
          aria-live="polite"
          data-testid="banner-scan-progress"
        >
          <span class="banner-text">
            Scan en cours : {$scanProgress.processed} / {$scanProgress.total}
            — <span class="banner-file">{$scanProgress.current_file}</span>
          </span>
          <div
            class="scan-progress-bar"
            aria-hidden="true"
            style="--progress: {$scanProgress.total > 0
              ? ($scanProgress.processed / $scanProgress.total) * 100
              : 0}%"
          ></div>
        </div>
      {/if}
    </div>

    <main class="content">
      {#key routeKey}
        <div in:fade={{ duration: 180 }}>
          <slot />
        </div>
      {/key}
    </main>
  </div>

  <ToastContainer />
  <EmbeddedPlayer />
</div>

<style>
  .app {
    display: flex;
    height: 100vh;
    overflow: hidden;
    background: var(--bg-primary);
    color: var(--text-primary);
    font-family: var(--font-family);
  }

  .main {
    display: flex;
    flex-direction: column;
    flex: 1 1 auto;
    min-width: 0;
    /* Autorise le child `.content` à devenir le seul scroller : sans
       `min-height: 0`, le flex item refuserait de rétrécir et c'est
       la fenêtre qui scrollerait à la place. */
    min-height: 0;
    overflow: hidden;
  }

  .banners {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .banner {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-4);
    border-bottom: 1px solid var(--border);
    font-size: 0.9rem;
    line-height: 1.4;
    animation: kyro-fade-in var(--duration-base) var(--ease-standard);
  }

  .banner-text {
    flex: 1 1 auto;
    min-width: 0;
  }

  /* Tronque le chemin du fichier en cours pour éviter de pousser le
     bouton hors champ ; même règle que `<TopBar>` pour cohérence. */
  .banner-file {
    display: inline-block;
    max-width: 480px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    vertical-align: bottom;
  }

  .banner a {
    color: inherit;
    text-decoration: underline;
  }

  .banner-close,
  .banner-action {
    background: transparent;
    border: 1px solid currentColor;
    color: inherit;
    border-radius: var(--radius-control);
    padding: var(--space-1) var(--space-2);
    cursor: pointer;
    font-size: 0.85rem;
    transition: background-color var(--duration-fast) var(--ease-standard);
  }

  .banner-close:hover,
  .banner-action:hover {
    background: rgba(255, 255, 255, 0.15);
  }

  /* Jaune Adwaita pour ffprobe manquant (avertissement). */
  .banner-warning {
    background: var(--color-warning);
    color: #1e1e2e;
  }

  /* Rouge Adwaita pour backend dégradé (erreur). */
  .banner-error {
    background: var(--color-error);
    color: #ffffff;
  }

  /* Bleu d'accent pour la progression du scan (information). */
  .banner-info {
    background: var(--accent);
    color: var(--text-primary);
    overflow: hidden;
  }

  /*
   * Barre de progression visuelle subtile en pied du bandeau scan-
   * progress : s'étend de 0 % à 100 % au fur et à mesure que
   * `processed / total` augmente, transition douce.
   */
  .scan-progress-bar {
    position: absolute;
    left: 0;
    bottom: 0;
    height: 2px;
    width: var(--progress, 0%);
    background: rgba(255, 255, 255, 0.6);
    transition: width var(--duration-base) var(--ease-out);
  }

  .content {
    flex: 1 1 auto;
    min-width: 0;
    /* Indispensable pour que `overflow-y: auto` prenne effet dans un
       parent flex column : sans ça, le child garde sa hauteur intrinsèque
       et c'est la fenêtre qui scrolle. */
    min-height: 0;
    padding: var(--space-4);
    overflow-y: auto;
    /* Réserve la place de la scrollbar même quand elle est absente :
       élimine le micro-décalage horizontal entre les pages courtes
       (Animes, Séries) et la page Films plus haute qui fait apparaître
       la barre. */
    scrollbar-gutter: stable;
  }
</style>
