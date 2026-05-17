<script lang="ts">
  /**
   * Champ de saisie de la note utilisateur (Requirement 6.6).
   *
   * - `value` est `null` quand le champ est vide (la note est effacée).
   * - Toute valeur hors de l'intervalle [0, 10] (entiers ou non) est
   *   refusée et un message d'erreur explicite indiquant la plage
   *   autorisée est affiché.
   * - L'erreur n'est visible que lorsque la valeur courante est invalide.
   *
   * Ce composant est un contrôle d'entrée pur : il ne persiste rien.
   * La validation côté backend (Requirement 6.6, Property 12) reste la
   * source de vérité ; ce garde-fou UI évite simplement les allers-retours
   * inutiles.
   */

  export let value: number | null = null;
  export let disabled = false;
  export let id: string | undefined = undefined;

  const ERROR_MESSAGE = 'La note doit être comprise entre 0 et 10.';
  const MIN = 0;
  const MAX = 10;

  let errorMessage = '';

  function isValid(n: number): boolean {
    return Number.isFinite(n) && n >= MIN && n <= MAX;
  }

  function onInput(event: Event): void {
    const input = event.currentTarget as HTMLInputElement;
    const raw = input.value;

    if (raw.trim() === '') {
      value = null;
      errorMessage = '';
      return;
    }

    const parsed = Number(raw);
    if (!Number.isFinite(parsed) || !isValid(parsed)) {
      // On reflète quand même la saisie courante pour que l'erreur
      // corresponde à ce que l'utilisateur voit, mais on n'expose pas
      // une valeur invalide en dehors du composant.
      value = null;
      errorMessage = ERROR_MESSAGE;
      return;
    }

    value = parsed;
    errorMessage = '';
  }
</script>

<div class="rating-input">
  <input
    {id}
    type="number"
    min={MIN}
    max={MAX}
    step="1"
    {disabled}
    value={value ?? ''}
    aria-invalid={errorMessage !== ''}
    aria-describedby={errorMessage !== '' ? `${id ?? 'rating'}-error` : undefined}
    on:input={onInput}
  />
  {#if errorMessage !== ''}
    <p
      class="error"
      id={`${id ?? 'rating'}-error`}
      role="alert"
    >
      {errorMessage}
    </p>
  {/if}
</div>

<style>
  .rating-input {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 0.25rem);
  }

  input {
    width: 5rem;
  }

  .error {
    margin: 0;
    color: #e01b24; /* rouge Adwaita destructive */
    font-size: 0.875rem;
  }
</style>
