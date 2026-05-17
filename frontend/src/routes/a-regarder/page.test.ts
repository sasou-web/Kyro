/**
 * @vitest-environment jsdom
 *
 * Tests unitaires de la page « À regarder » (`/a-regarder`, tâche 15.3).
 *
 * Couverture :
 *  - au mount, `listToWatch` est appelé une fois ;
 *  - état vide : un message guide s'affiche quand la liste est vide ;
 *  - liste non vide : la grille rend une `<MediaCard>` par item et
 *    chaque carte est enveloppée d'un lien vers `/detail/{id}` ;
 *  - état d'erreur : si `listToWatch` rejette, un message d'erreur
 *    s'affiche en place de la grille.
 *
 * _Requirements: 11.5_
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render } from '@testing-library/svelte';

const mocks = vi.hoisted(() => ({
  listToWatch: vi.fn(async () => [] as unknown[])
}));

vi.mock('$lib/api/commands', () => ({
  listToWatch: mocks.listToWatch
}));

import AregarderPage from './+page.svelte';
import type { MediaItem } from '$lib/api/types';

/**
 * Drain les microtâches : `onMount` est `async` et la grille n'est
 * peuplée qu'après résolution de `listToWatch`.
 */
async function flushAsync(): Promise<void> {
  for (let i = 0; i < 10; i += 1) {
    await Promise.resolve();
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

const sampleItems: MediaItem[] = [
  {
    id: 10,
    title: 'Inception',
    original_title: null,
    year: 2010,
    type: 'Film',
    status: 'non_vu',
    genres: []
  },
  {
    id: 11,
    title: 'Breaking Bad',
    original_title: null,
    year: 2008,
    type: 'Series',
    status: 'en_cours',
    genres: ['Drame']
  }
];

beforeEach(() => {
  mocks.listToWatch.mockReset();
  mocks.listToWatch.mockResolvedValue([]);
});

afterEach(() => {
  cleanup();
});

describe('/a-regarder — appel `listToWatch`', () => {
  it('appelle listToWatch une fois au mount', async () => {
    render(AregarderPage);
    await flushAsync();

    expect(mocks.listToWatch).toHaveBeenCalledTimes(1);
  });
});

describe('/a-regarder — état vide', () => {
  it('affiche un message bienveillant quand la liste est vide', async () => {
    mocks.listToWatch.mockResolvedValueOnce([]);

    const { queryByTestId } = render(AregarderPage);
    await flushAsync();

    const empty = queryByTestId('a-regarder-empty');
    expect(empty).not.toBeNull();
    expect(empty?.textContent).toMatch(/regarder/i);
    expect(queryByTestId('a-regarder-grid')).toBeNull();
  });
});

describe('/a-regarder — liste non vide', () => {
  it('rend une carte par item et chaque carte pointe vers /detail/{id}', async () => {
    mocks.listToWatch.mockResolvedValueOnce(sampleItems);

    const { queryAllByTestId, queryByTestId } = render(AregarderPage);
    await flushAsync();

    expect(queryByTestId('a-regarder-grid')).not.toBeNull();
    expect(queryByTestId('a-regarder-empty')).toBeNull();

    const links = queryAllByTestId('a-regarder-card-link');
    expect(links).toHaveLength(2);
    expect(links[0].getAttribute('href')).toBe('/detail/10');
    expect(links[1].getAttribute('href')).toBe('/detail/11');
  });
});

describe('/a-regarder — erreur backend', () => {
  it('affiche un message d\'erreur si listToWatch rejette', async () => {
    mocks.listToWatch.mockRejectedValueOnce(new Error('IPC failure'));

    const { queryByTestId } = render(AregarderPage);
    await flushAsync();

    const error = queryByTestId('a-regarder-error');
    expect(error).not.toBeNull();
    expect(error?.textContent).toMatch(/IPC failure/);
    expect(queryByTestId('a-regarder-grid')).toBeNull();
  });
});
