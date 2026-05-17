/**
 * @vitest-environment jsdom
 *
 * Tests unitaires de la page « Favoris » (`/favoris`, tâche 15.3).
 *
 * Couverture :
 *  - au mount, `listFavorites` est appelé une fois ;
 *  - état vide : un message guide s'affiche quand la liste est vide ;
 *  - liste non vide : la grille rend une `<MediaCard>` par item et
 *    chaque carte est enveloppée d'un lien vers `/detail/{id}` ;
 *  - état d'erreur : si `listFavorites` rejette, un message d'erreur
 *    s'affiche en place de la grille.
 *
 * _Requirements: 11.4_
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render } from '@testing-library/svelte';

const mocks = vi.hoisted(() => ({
  listFavorites: vi.fn(async () => [] as unknown[])
}));

vi.mock('$lib/api/commands', () => ({
  listFavorites: mocks.listFavorites
}));

import FavorisPage from './+page.svelte';
import type { MediaItem } from '$lib/api/types';

/**
 * Drain les microtâches : `onMount` est `async` et la grille n'est
 * peuplée qu'après résolution de `listFavorites`. Plusieurs flushs
 * laissent tous les `then` se résoudre.
 */
async function flushAsync(): Promise<void> {
  for (let i = 0; i < 10; i += 1) {
    await Promise.resolve();
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

const sampleItems: MediaItem[] = [
  {
    id: 1,
    title: 'Blade Runner',
    original_title: null,
    year: 1982,
    type: 'Film',
    status: 'non_vu',
    genres: []
  },
  {
    id: 2,
    title: 'Cowboy Bebop',
    original_title: 'カウボーイビバップ',
    year: 1998,
    type: 'Anime',
    status: 'en_cours',
    genres: ['Action']
  }
];

beforeEach(() => {
  mocks.listFavorites.mockReset();
  mocks.listFavorites.mockResolvedValue([]);
});

afterEach(() => {
  cleanup();
});

describe('/favoris — appel `listFavorites`', () => {
  it('appelle listFavorites une fois au mount', async () => {
    render(FavorisPage);
    await flushAsync();

    expect(mocks.listFavorites).toHaveBeenCalledTimes(1);
  });
});

describe('/favoris — état vide', () => {
  it('affiche un message bienveillant quand la liste est vide', async () => {
    mocks.listFavorites.mockResolvedValueOnce([]);

    const { queryByTestId } = render(FavorisPage);
    await flushAsync();

    const empty = queryByTestId('favoris-empty');
    expect(empty).not.toBeNull();
    expect(empty?.textContent).toMatch(/favori/i);
    expect(queryByTestId('favoris-grid')).toBeNull();
  });
});

describe('/favoris — liste non vide', () => {
  it('rend une carte par item et chaque carte pointe vers /detail/{id}', async () => {
    mocks.listFavorites.mockResolvedValueOnce(sampleItems);

    const { queryAllByTestId, queryByTestId } = render(FavorisPage);
    await flushAsync();

    expect(queryByTestId('favoris-grid')).not.toBeNull();
    expect(queryByTestId('favoris-empty')).toBeNull();

    const links = queryAllByTestId('favoris-card-link');
    expect(links).toHaveLength(2);
    expect(links[0].getAttribute('href')).toBe('/detail/1');
    expect(links[1].getAttribute('href')).toBe('/detail/2');
  });
});

describe('/favoris — erreur backend', () => {
  it('affiche un message d\'erreur si listFavorites rejette', async () => {
    mocks.listFavorites.mockRejectedValueOnce(new Error('IPC failure'));

    const { queryByTestId } = render(FavorisPage);
    await flushAsync();

    const error = queryByTestId('favoris-error');
    expect(error).not.toBeNull();
    expect(error?.textContent).toMatch(/IPC failure/);
    expect(queryByTestId('favoris-grid')).toBeNull();
  });
});
