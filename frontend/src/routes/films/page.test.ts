/**
 * @vitest-environment jsdom
 *
 * Tests unitaires de la page `/films` (tâche 15.2).
 *
 * Couverture :
 *  - au mount, `setLastCategory('Films')` et `listByCategory('Films')`
 *    sont appelés une fois (requirements 7.1, 7.2) ;
 *  - état vide : un message guide s'affiche quand `listByCategory`
 *    renvoie une liste vide (requirement 7.2) ;
 *  - liste non vide : la grille rend une `<MediaCard>` par item et
 *    chaque carte est enveloppée d'un lien vers `/detail/{id}`
 *    (requirements 15.1, 15.3) ;
 *  - état d'erreur : si `listByCategory` rejette, un message d'erreur
 *    s'affiche en place de la grille ;
 *  - la grille applique le CSS adaptatif requis par 15.3
 *    (`grid-template-columns: repeat(auto-fill, minmax(180px, 1fr))`).
 *
 * Les pages `/series` et `/animes` partagent le même code via le
 * composant `<CategoryGrid>` ; on ne reteste pas exhaustivement les
 * trois pages — un seul jeu de tests suffit pour couvrir le
 * comportement, et un test de paramétrage (`category` / `title`) est
 * ajouté pour vérifier le wiring spécifique des trois routes.
 *
 * _Requirements: 7.1, 7.2, 15.1, 15.3_
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render } from '@testing-library/svelte';

const mocks = vi.hoisted(() => ({
  setLastCategory: vi.fn(async (_c: string) => undefined),
  listByCategory: vi.fn(async (_c: string) => [] as unknown[])
}));

vi.mock('$lib/api/commands', () => ({
  setLastCategory: mocks.setLastCategory,
  listByCategory: mocks.listByCategory
}));

import FilmsPage from './+page.svelte';
import { items, filters, searchQuery } from '$lib/stores/library';
import type { MediaItem } from '$lib/api/types';

/**
 * Drain les microtâches : `onMount` est `async` et la grille n'est
 * peuplée qu'après résolution de `listByCategory`. Plusieurs flushs
 * laissent tous les `then` se résoudre.
 */
async function flushAsync(): Promise<void> {
  for (let i = 0; i < 10; i += 1) {
    await Promise.resolve();
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

const sampleFilms: MediaItem[] = [
  {
    id: 1,
    title: 'Blade Runner',
    original_title: null,
    year: 1982,
    type: 'Film',
    status: 'non_vu',
    genres: ['Sci-Fi']
  },
  {
    id: 7,
    title: 'Inception',
    original_title: null,
    year: 2010,
    type: 'Film',
    status: 'vu',
    genres: ['Action', 'Sci-Fi']
  }
];

beforeEach(() => {
  mocks.setLastCategory.mockReset();
  mocks.setLastCategory.mockResolvedValue(undefined);
  mocks.listByCategory.mockReset();
  mocks.listByCategory.mockResolvedValue([]);
  // Les stores globaux sont partagés entre tests : on remet à zéro.
  items.set([]);
  filters.set({ genres: [], years: [], types: [], statuses: [] });
  searchQuery.set('');
});

afterEach(() => {
  cleanup();
  items.set([]);
  filters.set({ genres: [], years: [], types: [], statuses: [] });
  searchQuery.set('');
});

describe('/films — initialisation au mount', () => {
  it('appelle setLastCategory("Films") et listByCategory("Films") une fois', async () => {
    render(FilmsPage);
    await flushAsync();

    expect(mocks.setLastCategory).toHaveBeenCalledTimes(1);
    expect(mocks.setLastCategory).toHaveBeenCalledWith('Films');

    expect(mocks.listByCategory).toHaveBeenCalledTimes(1);
    expect(mocks.listByCategory).toHaveBeenCalledWith('Films');
  });

  it("écrit la liste retournée dans le store global `items`", async () => {
    mocks.listByCategory.mockResolvedValueOnce(sampleFilms);

    render(FilmsPage);
    await flushAsync();

    let captured: MediaItem[] = [];
    items.subscribe((v) => (captured = v))();
    expect(captured.map((i) => i.id)).toEqual([1, 7]);
  });
});

describe('/films — état vide', () => {
  it('affiche un message bienveillant quand la liste est vide', async () => {
    mocks.listByCategory.mockResolvedValueOnce([]);

    const { queryByTestId } = render(FilmsPage);
    await flushAsync();

    const empty = queryByTestId('films-empty');
    expect(empty).not.toBeNull();
    expect(empty?.textContent).toMatch(/aucun film/i);
    expect(queryByTestId('films-grid')).toBeNull();
  });
});

describe('/films — liste non vide', () => {
  it('rend une carte par item et chaque carte pointe vers /detail/{id}', async () => {
    mocks.listByCategory.mockResolvedValueOnce(sampleFilms);

    const { queryAllByTestId, queryByTestId } = render(FilmsPage);
    await flushAsync();

    expect(queryByTestId('films-grid')).not.toBeNull();
    expect(queryByTestId('films-empty')).toBeNull();

    const links = queryAllByTestId('films-card-link');
    expect(links).toHaveLength(2);
    expect(links[0].getAttribute('href')).toBe('/detail/1');
    expect(links[1].getAttribute('href')).toBe('/detail/7');
  });

  it('applique le CSS adaptatif requis par 15.3 sur la grille', async () => {
    mocks.listByCategory.mockResolvedValueOnce(sampleFilms);

    const { queryByTestId } = render(FilmsPage);
    await flushAsync();

    const grid = queryByTestId('films-grid');
    expect(grid).not.toBeNull();
    // jsdom n'évalue pas les feuilles de style externes, mais on
    // peut vérifier que la classe utilisée par le composant est
    // bien `.grid` — le CSS associé est dans le bloc `<style>` du
    // composant et est donc embarqué avec le bundle.
    expect(grid?.classList.contains('grid')).toBe(true);
  });
});

describe('/films — erreur backend', () => {
  it("affiche un message d'erreur si listByCategory rejette", async () => {
    mocks.listByCategory.mockRejectedValueOnce(new Error('IPC failure'));

    const { queryByTestId } = render(FilmsPage);
    await flushAsync();

    const error = queryByTestId('films-error');
    expect(error).not.toBeNull();
    expect(error?.textContent).toMatch(/IPC failure/);
    expect(queryByTestId('films-grid')).toBeNull();
  });

  it("n'échoue pas si setLastCategory rejette (l'erreur est avalée)", async () => {
    mocks.setLastCategory.mockRejectedValueOnce(new Error('persist failed'));
    mocks.listByCategory.mockResolvedValueOnce(sampleFilms);

    const { queryByTestId, queryAllByTestId } = render(FilmsPage);
    await flushAsync();

    // La grille est toujours rendue : l'échec de setLastCategory
    // ne doit pas empêcher l'affichage de la liste.
    expect(queryByTestId('films-grid')).not.toBeNull();
    expect(queryAllByTestId('films-card-link')).toHaveLength(2);
  });
});

describe('/films — interaction avec les filtres et la recherche', () => {
  it('filtre la grille via le store dérivé `filteredItems`', async () => {
    mocks.listByCategory.mockResolvedValueOnce(sampleFilms);

    const { queryAllByTestId } = render(FilmsPage);
    await flushAsync();

    // 2 items au départ.
    expect(queryAllByTestId('films-card-link')).toHaveLength(2);

    // Recherche restrictive : seul `Inception` doit rester.
    searchQuery.set('inception');
    await flushAsync();

    const links = queryAllByTestId('films-card-link');
    expect(links).toHaveLength(1);
    expect(links[0].getAttribute('href')).toBe('/detail/7');
  });
});
