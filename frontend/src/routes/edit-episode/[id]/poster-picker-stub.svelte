<script lang="ts">
  /**
   * Stub Svelte minimal substitué à `<PosterPicker>` dans les tests
   * de la page d'édition Episode_Item (cf. `page.test.ts`).
   *
   * Le vrai composant ouvre une boîte de dialogue Tauri natif et
   * appelle des commandes IPC qui ne sont pas disponibles dans
   * l'environnement jsdom. Ce stub se contente de rendre un bouton
   * inerte qui ne fait rien — suffisant pour ne pas casser le
   * rendu de la page.
   */
  export let kind: 'media' | 'episode' = 'episode';
  export let id = 0;
  // Référence inerte des props pour éviter les warnings
  // svelte-check / runtime — même schéma que le stub utilisé par
  // `routes/edit/[id]/poster-picker-stub.svelte`.
  $: void kind;
  $: void id;
</script>

<button type="button" data-testid="poster-picker-stub" disabled>
  PosterPicker stub
</button>
