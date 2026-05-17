<script lang="ts">
  /**
   * SearchBar — champ de recherche global accessible depuis toutes
   * les vues principales (placé dans la `TopBar` à la tâche 14.5).
   *
   * UX :
   *   - Icône loupe intégrée à gauche du champ.
   *   - Quand l'utilisateur a tapé du texte, un bouton ✕ apparaît à
   *     droite pour effacer instantanément (clic ou Escape).
   *   - Le champ s'élargit au focus avec une transition fluide.
   *
   * _Requirements: 13.1, 13.2, 13.3, 13.4_
   */

  import { onDestroy } from 'svelte';

  import { searchQuery } from '$lib/stores/library';

  /** Délai de debounce (ms) — aligné avec la cible 13.3. */
  const DEBOUNCE_MS = 50;

  /** Valeur courante du champ, mise à jour à chaque frappe. */
  let value = '';

  /** Identifiant du `setTimeout` en attente, ou `null` si aucun. */
  let pending: ReturnType<typeof setTimeout> | null = null;

  /** Référence DOM utilisée pour focus au clic sur le bouton clear. */
  let inputEl: HTMLInputElement | null = null;

  function cancelPending(): void {
    if (pending !== null) {
      clearTimeout(pending);
      pending = null;
    }
  }

  function flush(next: string): void {
    cancelPending();
    searchQuery.set(next);
  }

  function handleInput(event: Event): void {
    const target = event.currentTarget as HTMLInputElement;
    value = target.value;

    if (value === '') {
      flush('');
      return;
    }

    cancelPending();
    pending = setTimeout(() => {
      pending = null;
      searchQuery.set(value);
    }, DEBOUNCE_MS);
  }

  function handleKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
      value = '';
      flush('');
    }
  }

  function clearSearch(): void {
    value = '';
    flush('');
    inputEl?.focus();
  }

  onDestroy(cancelPending);
</script>

<div class="search-bar-wrapper">
  <svg
    class="search-icon"
    viewBox="0 0 24 24"
    width="16"
    height="16"
    aria-hidden="true"
  >
    <circle cx="11" cy="11" r="7" fill="none" stroke="currentColor" stroke-width="1.8" />
    <path d="M16.2 16.2L21 21" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
  </svg>

  <input
    bind:this={inputEl}
    type="search"
    class="search-bar"
    placeholder="Rechercher..."
    aria-label="Rechercher"
    bind:value
    on:input={handleInput}
    on:keydown={handleKeydown}
  />

  {#if value.length > 0}
    <button
      type="button"
      class="clear-btn"
      aria-label="Effacer la recherche"
      on:click={clearSearch}
    >
      <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true">
        <circle cx="12" cy="12" r="9" fill="currentColor" opacity="0.18" />
        <path d="M9 9l6 6M15 9l-6 6" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
      </svg>
    </button>
  {/if}
</div>

<style>
  .search-bar-wrapper {
    position: relative;
    display: flex;
    align-items: center;
    width: 100%;
    max-width: 360px;
    transition: max-width var(--duration-base) var(--ease-emphasized);
  }

  .search-bar-wrapper:focus-within {
    max-width: 460px;
  }

  .search-icon {
    position: absolute;
    left: var(--space-3);
    color: var(--text-secondary);
    pointer-events: none;
    transition: color var(--duration-fast) var(--ease-standard);
  }

  .search-bar-wrapper:focus-within .search-icon {
    color: var(--accent);
  }

  .search-bar {
    width: 100%;
    padding: var(--space-2) calc(var(--space-3) + 22px) var(--space-2)
      calc(var(--space-4) + 16px);
    background: var(--surface);
    color: var(--text-primary);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    font-family: var(--font-family);
    font-size: 14px;
    outline: none;
    transition:
      border-color var(--duration-fast) var(--ease-standard),
      box-shadow var(--duration-fast) var(--ease-standard);
  }

  .search-bar::placeholder {
    color: var(--text-secondary);
  }

  .search-bar:hover {
    border-color: var(--text-secondary);
  }

  .search-bar:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px rgba(53, 132, 228, 0.15);
  }

  .search-bar::-webkit-search-cancel-button {
    appearance: none;
  }

  .clear-btn {
    position: absolute;
    right: var(--space-2);
    background: transparent;
    border: none;
    padding: var(--space-1);
    color: var(--text-secondary);
    border-radius: 50%;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transition:
      background-color var(--duration-fast) var(--ease-standard),
      color var(--duration-fast) var(--ease-standard);
  }

  .clear-btn:hover {
    color: var(--text-primary);
  }

  .clear-btn:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
</style>
