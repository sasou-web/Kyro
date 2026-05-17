<script lang="ts">
  /**
   * Page `/series` — grille des Media_Item de catégorie `Series`
   * (tâche 15.2).
   *
   * Délègue à `<CategoryGrid>` pour la logique de chargement et
   * la grille adaptive ; cette page se contente de passer la
   * `Category` cible et les libellés français.
   *
   * _Requirements: 7.1, 7.2, 15.1, 15.3_
   */
  import CategoryGrid from '$lib/components/CategoryGrid.svelte';
</script>

<CategoryGrid
  category="Series"
  title="Séries"
  emptyMessage="Aucune série à afficher pour le moment. Ajoutez un dossier de bibliothèque depuis Paramètres ou ajustez vos filtres."
  testIdPrefix="series"
/>
