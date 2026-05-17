/**
 * @vitest-environment jsdom
 *
 * Tests unitaires de la page `/animes` (tâche 15.2).
 *
 * Le comportement détaillé (état vide, erreur, filtrage, lien
 * `/detail/{id}`) est couvert par `films/page.test.ts` puisque les
 * trois routes partagent le composant `<CategoryGrid>`. On vérifie
 * ici simplement que le wiring spécifique à la route est correct :
 *  - `setLastCategory('Animes')` est appelé au mount ;
 *  - `listByCategory('Animes')` est appelé au mount ;
 *  - le titre `Animes` est rendu.
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

import AnimesPage from './+page.svelte';
import { items, filters, searchQuery } from '$lib/stores/library';

async function flushAsync(): Promise<void> {
  for (let i = 0; i < 10; i += 1) {
    await Promise.resolve();
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

beforeEach(() => {
  mocks.setLastCategory.mockReset();
  mocks.setLastCategory.mockResolvedValue(undefined);
  mocks.listByCategory.mockReset();
  mocks.listByCategory.mockResolvedValue([]);
  items.set([]);
  filters.set({ genres: [], years: [], types: [], statuses: [] });
  searchQuery.set('');
});

afterEach(() => {
  cleanup();
  items.set([]);
});

describe('/animes — wiring de la catégorie', () => {
  it("appelle setLastCategory('Animes') au mount", async () => {
    render(AnimesPage);
    await flushAsync();

    expect(mocks.setLastCategory).toHaveBeenCalledTimes(1);
    expect(mocks.setLastCategory).toHaveBeenCalledWith('Animes');
  });

  it("appelle listByCategory('Animes') au mount", async () => {
    render(AnimesPage);
    await flushAsync();

    expect(mocks.listByCategory).toHaveBeenCalledTimes(1);
    expect(mocks.listByCategory).toHaveBeenCalledWith('Animes');
  });

  it("rend le titre 'Animes'", async () => {
    const { getByText } = render(AnimesPage);
    await flushAsync();

    expect(getByText('Animes')).toBeTruthy();
  });
});
