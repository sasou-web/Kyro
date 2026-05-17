/**
 * @vitest-environment jsdom
 *
 * Tests unitaires de la page racine `/` (tâche 15.2).
 *
 * Couverture :
 *  - au mount, `getLastCategory()` est appelé une fois ;
 *  - quand `getLastCategory` renvoie `null`, on redirige vers
 *    `/films` (défaut) avec `replaceState: true` ;
 *  - quand `getLastCategory` renvoie `Films`, `Series` ou `Animes`,
 *    la redirection cible respectivement `/films`, `/series`,
 *    `/animes` ;
 *  - en cas d'erreur IPC, on retombe sur `/films` plutôt que de
 *    laisser l'utilisateur sur la racine.
 *
 * _Requirements: 7.1_
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render } from '@testing-library/svelte';

const mocks = vi.hoisted(() => ({
  getLastCategory: vi.fn(async () => null as string | null),
  goto: vi.fn(async (_path: string, _opts?: { replaceState?: boolean }) => undefined)
}));

vi.mock('$lib/api/commands', () => ({
  getLastCategory: mocks.getLastCategory
}));

vi.mock('$app/navigation', () => ({
  goto: mocks.goto
}));

import IndexPage from './+page.svelte';

/**
 * Drain les microtâches : `onMount` est `async` et la redirection
 * passe par un `await goto()`. Plusieurs flushs laissent tous les
 * `then` se résoudre.
 */
async function flushAsync(): Promise<void> {
  for (let i = 0; i < 10; i += 1) {
    await Promise.resolve();
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

beforeEach(() => {
  mocks.getLastCategory.mockReset();
  mocks.goto.mockReset();
  mocks.getLastCategory.mockResolvedValue(null);
});

afterEach(() => {
  cleanup();
});

describe('/ (page racine) — redirection vers la dernière catégorie', () => {
  it('appelle getLastCategory une fois au mount', async () => {
    render(IndexPage);
    await flushAsync();

    expect(mocks.getLastCategory).toHaveBeenCalledTimes(1);
  });

  it('redirige vers /films par défaut quand last_category est null', async () => {
    mocks.getLastCategory.mockResolvedValueOnce(null);

    render(IndexPage);
    await flushAsync();

    expect(mocks.goto).toHaveBeenCalledTimes(1);
    expect(mocks.goto).toHaveBeenCalledWith('/films', { replaceState: true });
  });

  it('redirige vers /films quand last_category = Films', async () => {
    mocks.getLastCategory.mockResolvedValueOnce('Films');

    render(IndexPage);
    await flushAsync();

    expect(mocks.goto).toHaveBeenCalledWith('/films', { replaceState: true });
  });

  it('redirige vers /series quand last_category = Series', async () => {
    mocks.getLastCategory.mockResolvedValueOnce('Series');

    render(IndexPage);
    await flushAsync();

    expect(mocks.goto).toHaveBeenCalledWith('/series', { replaceState: true });
  });

  it('redirige vers /animes quand last_category = Animes', async () => {
    mocks.getLastCategory.mockResolvedValueOnce('Animes');

    render(IndexPage);
    await flushAsync();

    expect(mocks.goto).toHaveBeenCalledWith('/animes', { replaceState: true });
  });

  it('retombe sur /films quand getLastCategory rejette', async () => {
    mocks.getLastCategory.mockRejectedValueOnce(new Error('IPC failure'));

    render(IndexPage);
    await flushAsync();

    expect(mocks.goto).toHaveBeenCalledTimes(1);
    expect(mocks.goto).toHaveBeenCalledWith('/films', { replaceState: true });
  });
});
