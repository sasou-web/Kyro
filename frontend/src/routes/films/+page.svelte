<script lang="ts">
  /**
   * Page `/films` — grille des Media_Item de catégorie `Films`
   * (tâche 15.2).
   *
   * Toute la logique (mount, chargement IPC, persistance de la
   * dernière catégorie, grille adaptive, état vide) est mutualisée
   * dans `<CategoryGrid>` — on délègue ici en passant simplement
   * la `Category` cible et les libellés français.
   *
   * _Requirements: 7.1, 7.2, 15.1, 15.3_
   */
  import CategoryGrid from '$lib/components/CategoryGrid.svelte';
</script>

<CategoryGrid
  category="Films"
  title="Films"
  emptyMessage="Aucun film à afficher pour le moment. Ajoutez un dossier de bibliothèque depuis Paramètres ou ajustez vos filtres."
  testIdPrefix="films"
/>
