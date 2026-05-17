<!--
  FilterChip — chip réutilisable pour les filtres de la bibliothèque.

  Composant atomique sans logique métier : il se contente d'afficher
  un libellé, de refléter visuellement l'état `selected`, et de
  déclencher un événement `toggle` au clic. Le composant parent
  (`<FilterPanel>`) est seul responsable de mettre à jour le store
  `filters` (`$lib/stores/library`) en fonction de la sémantique de
  la dimension (union pour les genres, intersection pour les autres).

  Props :
    - `label`: texte affiché à l'utilisateur.
    - `selected`: vrai si ce chip représente une valeur active dans
      le filtre courant ; détermine la mise en valeur visuelle
      (couleur d'accent vs. surface neutre).

  Événements :
    - `toggle` (sans payload) : émis sur clic ou activation clavier
      (Entrée / Espace via le comportement natif du `<button>`).

  Accessibilité : on s'appuie sur un `<button>` natif pour l'activation
  clavier et le focus visible. `aria-pressed` reflète l'état pour les
  technologies d'assistance.

  _Requirements: 14.1, 14.2, 14.3_
-->
<script lang="ts">
  import { createEventDispatcher } from 'svelte';

  export let label: string;
  export let selected: boolean = false;

  const dispatch = createEventDispatcher<{ toggle: void }>();

  function handleClick(): void {
    dispatch('toggle');
  }
</script>

<button
  type="button"
  class="chip"
  class:selected
  aria-pressed={selected}
  on:click={handleClick}
>
  {label}
</button>

<style>
  .chip {
    display: inline-flex;
    align-items: center;
    padding: var(--space-2, 0.25rem) var(--space-3, 0.5rem);
    background: var(--surface);
    color: var(--text-primary);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    font-family: var(--font-family);
    font-size: 0.875rem;
    cursor: pointer;
    transition: background-color 120ms ease, border-color 120ms ease,
      color 120ms ease;
  }

  .chip:hover {
    border-color: var(--accent);
  }

  .chip:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .chip.selected {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }

  .chip.selected:hover {
    background: var(--accent-hover, var(--accent));
    border-color: var(--accent-hover, var(--accent));
  }
</style>
