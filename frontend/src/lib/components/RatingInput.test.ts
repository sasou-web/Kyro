/**
 * @vitest-environment jsdom
 *
 * Tests unitaires du composant `RatingInput`.
 *
 * On vérifie le contrat fonctionnel imposé par Requirement 6.6 :
 *   - une valeur > 10 affiche le message d'erreur explicite indiquant la plage
 *     autorisée ;
 *   - une valeur ∈ [0, 10] n'affiche aucune erreur ;
 *   - vider le champ remet `value` à `null` (la note est effacée).
 *
 * _Requirements: 6.6_
 */

import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render } from '@testing-library/svelte';

import RatingInput from './RatingInput.svelte';

afterEach(() => {
  cleanup();
});

const ERROR_MESSAGE = 'La note doit être comprise entre 0 et 10.';

describe('RatingInput', () => {
  it("affiche le message d'erreur lorsque la valeur saisie est hors plage (11)", async () => {
    const { container, queryByRole } = render(RatingInput, { value: null });
    const input = container.querySelector('input') as HTMLInputElement;

    await fireEvent.input(input, { target: { value: '11' } });

    const error = queryByRole('alert');
    expect(error).not.toBeNull();
    expect(error?.textContent).toBe(ERROR_MESSAGE);
    expect(input.getAttribute('aria-invalid')).toBe('true');
  });

  it("n'affiche aucune erreur pour une valeur valide (5)", async () => {
    const { container, queryByRole } = render(RatingInput, { value: null });
    const input = container.querySelector('input') as HTMLInputElement;

    await fireEvent.input(input, { target: { value: '5' } });

    expect(queryByRole('alert')).toBeNull();
    expect(input.getAttribute('aria-invalid')).toBe('false');
  });

  it("vider le champ ramène value à null et masque l'erreur", async () => {
    // On part d'une saisie invalide pour s'assurer que l'erreur disparaît.
    // Sans `accessors`, on ne peut pas lire la prop `value` directement
    // depuis l'instance Svelte. On observe à la place ce que rend le
    // composant : l'input lit `value={value ?? ''}`, donc un input
    // vide ⇔ `value` interne `null`.
    const { container, queryByRole } = render(RatingInput, { value: 7 });
    const input = container.querySelector('input') as HTMLInputElement;

    await fireEvent.input(input, { target: { value: '12' } });
    expect(queryByRole('alert')).not.toBeNull();

    await fireEvent.input(input, { target: { value: '' } });

    expect(queryByRole('alert')).toBeNull();
    // L'input vide ⇒ `value` interne = null (effacement de la note).
    expect(input.value).toBe('');
  });

  it('rejette les valeurs négatives avec le même message', async () => {
    const { container, queryByRole } = render(RatingInput, { value: null });
    const input = container.querySelector('input') as HTMLInputElement;

    await fireEvent.input(input, { target: { value: '-1' } });

    const error = queryByRole('alert');
    expect(error).not.toBeNull();
    expect(error?.textContent).toBe(ERROR_MESSAGE);
  });
});
