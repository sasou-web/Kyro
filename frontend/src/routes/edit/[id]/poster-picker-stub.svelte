<script lang="ts">
  /**
   * Stub Svelte minimal substitué à `<PosterPicker>` dans les tests
   * de la page d'édition (cf. `page.test.ts`).
   *
   * Le vrai composant ouvre une boîte de dialogue Tauri natif et
   * appelle des commandes IPC qui ne sont pas disponibles dans
   * l'environnement jsdom. Ce stub se contente de rendre un bouton
   * inerte qui ne fait rien — suffisant pour ne pas casser le
   * rendu de la page.
   */
  export let kind: 'media' | 'episode' = 'media';
  export let id = 0;
  // Référence inerte des props pour éviter à la fois :
  //   - l'avertissement svelte-check « Component has unused export
  //     property » si on les déclare sans les utiliser ;
  //   - le warning runtime « unknown prop » côté Svelte quand le
  //     parent passe ces props.
  $: void kind;
  $: void id;
</script>

<button type="button" data-testid="poster-picker-stub" disabled>
  PosterPicker stub
</button>
