/**
 * @vitest-environment jsdom
 *
 * Tests unitaires du composant `<SearchBar>`.
 *
 * Couvre les exigences :
 * - 13.1 : champ accessible (placeholder / aria-label) ;
 * - 13.2 : la saisie alimente le store `searchQuery` ;
 * - 13.3 : debounce 50 ms (le store n'est pas mis à jour avant) ;
 * - 13.4 : champ vidé ou `Escape` ⇒ store remis à `''`
 *   immédiatement (sans attendre les 50 ms).
 *
 * On utilise `vi.useFakeTimers()` pour piloter le debounce de
 * façon déterministe — sans dépendre du temps réel de l'OS.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { get } from 'svelte/store';

import { searchQuery } from '$lib/stores/library';
import SearchBar from './SearchBar.svelte';

beforeEach(() => {
  vi.useFakeTimers();
  searchQuery.set('');
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  searchQuery.set('');
});

describe('SearchBar — debounce 50 ms', () => {
  it('met à jour `searchQuery` à 50 ms après la frappe (requirement 13.2 / 13.3)', async () => {
    const { getByRole } = render(SearchBar);
    const input = getByRole('searchbox') as HTMLInputElement;

    await fireEvent.input(input, { target: { value: 'test' } });

    // Le store ne doit pas avoir été mis à jour tant que le timer
    // n'a pas expiré.
    expect(get(searchQuery)).toBe('');

    // Juste avant 50 ms : toujours rien.
    vi.advanceTimersByTime(49);
    expect(get(searchQuery)).toBe('');

    // À 50 ms : la valeur est propagée.
    vi.advanceTimersByTime(1);
    expect(get(searchQuery)).toBe('test');
  });

  it('seule la dernière saisie est propagée si plusieurs frappes se succèdent dans la fenêtre debounce', async () => {
    const { getByRole } = render(SearchBar);
    const input = getByRole('searchbox') as HTMLInputElement;

    await fireEvent.input(input, { target: { value: 'te' } });
    vi.advanceTimersByTime(20);
    await fireEvent.input(input, { target: { value: 'test' } });
    vi.advanceTimersByTime(49);
    expect(get(searchQuery)).toBe('');

    vi.advanceTimersByTime(1);
    expect(get(searchQuery)).toBe('test');
  });
});

describe('SearchBar — effacement (requirement 13.4)', () => {
  it("vider le champ remet `searchQuery` à '' immédiatement", async () => {
    const { getByRole } = render(SearchBar);
    const input = getByRole('searchbox') as HTMLInputElement;

    // D'abord, saisir et flusher une valeur.
    await fireEvent.input(input, { target: { value: 'test' } });
    vi.advanceTimersByTime(50);
    expect(get(searchQuery)).toBe('test');

    // Puis vider le champ : flush immédiat, pas d'attente.
    await fireEvent.input(input, { target: { value: '' } });
    expect(get(searchQuery)).toBe('');
  });

  it('appuyer sur Escape vide le champ et remet le store à \'\' immédiatement', async () => {
    const { getByRole } = render(SearchBar);
    const input = getByRole('searchbox') as HTMLInputElement;

    await fireEvent.input(input, { target: { value: 'test' } });
    vi.advanceTimersByTime(50);
    expect(get(searchQuery)).toBe('test');

    await fireEvent.keyDown(input, { key: 'Escape' });
    expect(get(searchQuery)).toBe('');
    expect(input.value).toBe('');
  });

  it("annule un debounce en attente quand l'utilisateur vide le champ avant 50 ms", async () => {
    const { getByRole } = render(SearchBar);
    const input = getByRole('searchbox') as HTMLInputElement;

    await fireEvent.input(input, { target: { value: 'test' } });
    vi.advanceTimersByTime(20);
    await fireEvent.input(input, { target: { value: '' } });

    expect(get(searchQuery)).toBe('');

    // Même après expiration du timer initial, le store reste vide
    // (le timer a été annulé).
    vi.advanceTimersByTime(100);
    expect(get(searchQuery)).toBe('');
  });
});

describe('SearchBar — accessibilité (requirement 13.1)', () => {
  it('expose un placeholder « Rechercher... » et un aria-label', () => {
    const { getByRole, getByPlaceholderText } = render(SearchBar);
    const input = getByRole('searchbox') as HTMLInputElement;

    expect(input).toBe(getByPlaceholderText('Rechercher...'));
    expect(input.getAttribute('aria-label')).toBe('Rechercher');
    expect(input.type).toBe('search');
  });
});
