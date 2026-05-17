<script lang="ts">
  /**
   * Page racine `/` (tâche 15.2).
   *
   * Rôle : rediriger l'utilisateur au démarrage vers la dernière
   * catégorie consultée (requirement 7.1). Au tout premier
   * démarrage, `get_last_category` renvoie `null` ; on retombe alors
   * sur `/films` par défaut comme spécifié dans `design.md` § Routes
   * / Pages.
   *
   * Implémentation :
   *   - `onMount` (CSR uniquement, le mode SPA garantit que le code
   *     ne tourne jamais côté serveur) appelle la commande Tauri
   *     `getLastCategory` puis appelle `goto()` du routeur SvelteKit
   *     avec `replaceState: true` pour ne pas polluer l'historique
   *     (l'utilisateur ne doit pas pouvoir « revenir » à la racine
   *     par le bouton précédent du navigateur).
   *   - En cas d'erreur IPC (backend dégradé, premier boot), on
   *     redirige vers `/films` — la même valeur que celle utilisée
   *     quand `last_category` est `null` ; on évite ainsi qu'un
   *     échec backend laisse l'utilisateur sur une page racine vide.
   *
   * Mapping `Category → route` aligné avec les routes SvelteKit
   * existantes (`/films`, `/series`, `/animes`, cf. `<Sidebar>`) :
   *
   * | Category | Route   |
   * | -------- | ------- |
   * | Films    | /films  |
   * | Series   | /series |
   * | Animes   | /animes |
   *
   * _Requirements: 7.1_
   */
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';

  import { getLastCategory } from '$lib/api/commands';
  import type { Category } from '$lib/api/types';

  /**
   * Mapping littéral `Category → route` exposé pour les tests.
   * Toute évolution des routes catégorie doit être répercutée ici
   * en parallèle de la sidebar (`<Sidebar>`).
   */
  export const CATEGORY_ROUTES: Readonly<Record<Category, string>> = {
    Films: '/films',
    Series: '/series',
    Animes: '/animes'
  };

  onMount(async () => {
    let target = '/films';
    try {
      const last = await getLastCategory();
      if (last !== null) {
        target = CATEGORY_ROUTES[last];
      }
    } catch (err) {
      console.warn('Kyro: échec getLastCategory, redirection vers /films', err);
    }
    // `replaceState: true` : la racine `/` ne doit pas figurer dans
    // l'historique de navigation — l'utilisateur revient sinon à une
    // page de redirection vide quand il appuie sur « précédent ».
    await goto(target, { replaceState: true });
  });
</script>

<!--
  Pas de rendu visible : la page n'est qu'un trampoline vers la bonne
  route catégorie. Un fallback minimaliste accessible est tout de même
  exposé pour les lecteurs d'écran et le cas (très court) où la
  redirection prend quelques dizaines de millisecondes.
-->
<p class="redirect-fallback" role="status" aria-live="polite">
  Chargement de Kyro…
</p>

<style>
  .redirect-fallback {
    margin: 0;
    padding: var(--space-4, 16px);
    color: var(--text-secondary);
    font-family: var(--font-family);
    font-size: 0.95rem;
  }
</style>
