/**
 * @vitest-environment jsdom
 *
 * Tests unitaires de la page détail Media_Item — tâche 15.4
 * (Requirements 7.3, 7.4, 8.5, 12.2, 12.3, 12.4, 15.4).
 *
 * Stratégie de mock :
 *   - `$lib/api/commands` : `getMediaItem` retourne une fiche
 *     synthétique au mount, `listEpisodes` retourne la liste
 *     d'épisodes attendue (déjà triée), `playMedia`, `playResume`,
 *     `playRewatchFromStart`, `toggleFavorite`, `toggleToWatch`
 *     sont espionnés pour vérifier les appels.
 *   - `$app/stores` : on injecte un `page` store minimal portant le
 *     paramètre `id` de la route SvelteKit `[id]`.
 *
 * On vérifie :
 *   1. au mount, `getMediaItem(id)` est appelé ;
 *   2. la fiche est rendue (titre, métadonnées, synopsis, genres) ;
 *   3. pour un Film, le bouton `Lire` appelle `playMedia(id, 'media')` ;
 *   4. pour une Série avec épisodes mixtes, le bouton `Reprendre`
 *      appelle `playResume(id)` ;
 *   5. pour une Série dont tous les épisodes sont vus, le bouton
 *      `Revoir depuis le début` appelle `playRewatchFromStart(id)` ;
 *   6. la liste d'épisodes est rendue groupée par saison, et le
 *      compteur `X / N épisodes vus` est correct ;
 *   7. cliquer sur `Lire` d'un épisode appelle
 *      `playMedia(episodeId, 'episode')`.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { writable } from 'svelte/store';

import type { EpisodeItem, MediaItemDetail } from '$lib/api/types';

// ---------------------------------------------------------------------------
// Mocks hoistés.
// ---------------------------------------------------------------------------

const mocks = vi.hoisted(() => {
  return {
    getMediaItem: vi.fn(),
    listEpisodes: vi.fn(),
    launchPlayback: vi.fn(async () => undefined),
    launchResume: vi.fn(async () => undefined),
    launchRewatchFromStart: vi.fn(async () => undefined),
    toggleFavorite: vi.fn(async () => true),
    toggleToWatch: vi.fn(async () => true),
    pageStore: undefined as
      | ReturnType<typeof writable<{ params: { id: string } }>>
      | undefined
  };
});

vi.mock('$lib/api/commands', () => ({
  getMediaItem: mocks.getMediaItem,
  listEpisodes: mocks.listEpisodes,
  toggleFavorite: mocks.toggleFavorite,
  toggleToWatch: mocks.toggleToWatch
}));

vi.mock('$lib/api/player-window', () => ({
  launchPlayback: mocks.launchPlayback,
  launchResume: mocks.launchResume,
  launchRewatchFromStart: mocks.launchRewatchFromStart
}));

vi.mock('$app/stores', async () => {
  const { writable } = await import('svelte/store');
  const store = writable<{ params: { id: string } }>({ params: { id: '42' } });
  mocks.pageStore = store;
  return { page: { subscribe: store.subscribe } };
});

import DetailPage from './+page.svelte';

// ---------------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------------

const FILM_FIXTURE: MediaItemDetail = {
  id: 42,
  type: 'Film',
  title: 'Inception',
  original_title: null,
  year: 2010,
  synopsis: 'Un voleur entre dans les rêves.',
  studio: 'Warner',
  user_rating: 9,
  poster_path: null,
  quality: '1080p',
  duration_seconds: 8880,
  video_codec: 'h264',
  width: 1920,
  height: 1080,
  watch_progress_seconds: 0,
  status: 'non_vu',
  favorite: false,
  to_watch: false,
  available: true,
  numbering_unknown: false,
  genres: ['Science-fiction', 'Thriller']
};

const SERIES_FIXTURE: MediaItemDetail = {
  id: 7,
  type: 'Series',
  title: 'Breaking Bad',
  original_title: null,
  year: 2008,
  synopsis: 'Un prof de chimie devient producteur.',
  studio: 'AMC',
  user_rating: 10,
  poster_path: null,
  quality: null,
  duration_seconds: null,
  video_codec: null,
  width: null,
  height: null,
  watch_progress_seconds: 0,
  status: 'en_cours',
  favorite: false,
  to_watch: false,
  available: true,
  numbering_unknown: false,
  genres: ['Drame']
};

const SERIES_EPISODES_MIXED: EpisodeItem[] = [
  { id: 100, season_number: 1, episode_number: 1, status: 'vu', available: true },
  { id: 101, season_number: 1, episode_number: 2, status: 'en_cours', available: true },
  { id: 102, season_number: 1, episode_number: 3, status: 'non_vu', available: true },
  { id: 200, season_number: 2, episode_number: 1, status: 'non_vu', available: true }
];

const SERIES_EPISODES_ALL_WATCHED: EpisodeItem[] = [
  { id: 100, season_number: 1, episode_number: 1, status: 'vu', available: true },
  { id: 101, season_number: 1, episode_number: 2, status: 'vu', available: true }
];

async function flushAsync(): Promise<void> {
  for (let i = 0; i < 10; i += 1) {
    await Promise.resolve();
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

beforeEach(() => {
  mocks.getMediaItem.mockReset();
  mocks.listEpisodes.mockReset();
  mocks.launchPlayback.mockReset();
  mocks.launchPlayback.mockResolvedValue(undefined);
  mocks.launchResume.mockReset();
  mocks.launchResume.mockResolvedValue(undefined);
  mocks.launchRewatchFromStart.mockReset();
  mocks.launchRewatchFromStart.mockResolvedValue(undefined);
  mocks.toggleFavorite.mockReset();
  mocks.toggleFavorite.mockResolvedValue(true);
  mocks.toggleToWatch.mockReset();
  mocks.toggleToWatch.mockResolvedValue(true);
  mocks.pageStore?.set({ params: { id: '42' } });
});

afterEach(() => {
  cleanup();
});

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

describe('/detail/[id] +page.svelte — Film', () => {
  beforeEach(() => {
    mocks.pageStore?.set({ params: { id: '42' } });
  });

  it('appelle getMediaItem au mount avec l\'id de la route', async () => {
    mocks.getMediaItem.mockResolvedValue(FILM_FIXTURE);

    render(DetailPage);
    await flushAsync();

    expect(mocks.getMediaItem).toHaveBeenCalledTimes(1);
    expect(mocks.getMediaItem).toHaveBeenCalledWith(42);
    // Pas de listEpisodes pour un Film.
    expect(mocks.listEpisodes).not.toHaveBeenCalled();
  });

  it('rend titre, métadonnées et synopsis', async () => {
    mocks.getMediaItem.mockResolvedValue(FILM_FIXTURE);

    const { queryByTestId } = render(DetailPage);
    await flushAsync();

    expect(queryByTestId('detail-title')?.textContent).toBe('Inception');
    expect(queryByTestId('detail-year')?.textContent).toBe('2010');
    expect(queryByTestId('detail-synopsis')?.textContent).toContain(
      'Un voleur'
    );
    // Genres rendus en chips.
    const chips = document.querySelectorAll('[data-testid="detail-genre-chip"]');
    expect(chips).toHaveLength(2);
  });

  it('affiche le bouton "Lire" et appelle launchPlayback(id, "media")', async () => {
    mocks.getMediaItem.mockResolvedValue(FILM_FIXTURE);

    const { queryByTestId } = render(DetailPage);
    await flushAsync();

    const playBtn = queryByTestId('detail-play-button') as HTMLButtonElement;
    expect(playBtn).not.toBeNull();
    expect(playBtn.textContent?.trim()).toBe('Lire');

    await fireEvent.click(playBtn);
    await waitFor(() => {
      expect(mocks.launchPlayback).toHaveBeenCalledTimes(1);
    });
    expect(mocks.launchPlayback).toHaveBeenCalledWith(42, 'media');

    // Pas de bouton Reprendre / Revoir pour un film.
    expect(queryByTestId('detail-resume-button')).toBeNull();
    expect(queryByTestId('detail-rewatch-button')).toBeNull();
  });

  it('ne rend pas la section épisodes pour un Film', async () => {
    mocks.getMediaItem.mockResolvedValue(FILM_FIXTURE);

    const { queryByTestId } = render(DetailPage);
    await flushAsync();

    expect(queryByTestId('detail-episodes-counter')).toBeNull();
    expect(queryByTestId('detail-episode-list')).toBeNull();
  });
});

describe('/detail/[id] +page.svelte — Série avec épisodes mixtes', () => {
  beforeEach(() => {
    mocks.pageStore?.set({ params: { id: '7' } });
  });

  it('appelle getMediaItem puis listEpisodes au mount', async () => {
    mocks.getMediaItem.mockResolvedValue(SERIES_FIXTURE);
    mocks.listEpisodes.mockResolvedValue(SERIES_EPISODES_MIXED);

    render(DetailPage);
    await flushAsync();

    expect(mocks.getMediaItem).toHaveBeenCalledWith(7);
    expect(mocks.listEpisodes).toHaveBeenCalledWith(7);
  });

  it('affiche le bouton "Reprendre" et appelle launchResume(id)', async () => {
    mocks.getMediaItem.mockResolvedValue(SERIES_FIXTURE);
    mocks.listEpisodes.mockResolvedValue(SERIES_EPISODES_MIXED);

    const { queryByTestId } = render(DetailPage);
    await flushAsync();

    const resumeBtn = queryByTestId('detail-resume-button') as HTMLButtonElement;
    expect(resumeBtn).not.toBeNull();
    expect(resumeBtn.textContent?.trim()).toBe('Reprendre');
    expect(queryByTestId('detail-rewatch-button')).toBeNull();

    await fireEvent.click(resumeBtn);
    await waitFor(() => {
      expect(mocks.launchResume).toHaveBeenCalledTimes(1);
    });
    expect(mocks.launchResume).toHaveBeenCalledWith(7);
  });

  it('rend le compteur "1 / 4 épisodes vus" (Requirement 8.5)', async () => {
    mocks.getMediaItem.mockResolvedValue(SERIES_FIXTURE);
    mocks.listEpisodes.mockResolvedValue(SERIES_EPISODES_MIXED);

    const { queryByTestId } = render(DetailPage);
    await flushAsync();

    const counter = queryByTestId('detail-episodes-counter');
    expect(counter).not.toBeNull();
    expect(counter?.textContent).toContain('1 / 4 épisodes vus');
  });

  it('rend la liste d\'épisodes groupée par saison (Requirements 7.3, 7.4)', async () => {
    mocks.getMediaItem.mockResolvedValue(SERIES_FIXTURE);
    mocks.listEpisodes.mockResolvedValue(SERIES_EPISODES_MIXED);

    render(DetailPage);
    await flushAsync();

    const groups = document.querySelectorAll(
      '[data-testid="detail-season-group"]'
    );
    expect(groups).toHaveLength(2);
    expect(groups[0].getAttribute('data-season')).toBe('1');
    expect(groups[1].getAttribute('data-season')).toBe('2');

    const rows = document.querySelectorAll(
      '[data-testid="detail-episode-row"]'
    );
    expect(rows).toHaveLength(4);
    // Ordre attendu : S1E1, S1E2, S1E3, S2E1.
    expect(rows[0].getAttribute('data-episode-id')).toBe('100');
    expect(rows[1].getAttribute('data-episode-id')).toBe('101');
    expect(rows[2].getAttribute('data-episode-id')).toBe('102');
    expect(rows[3].getAttribute('data-episode-id')).toBe('200');
  });

  it('cliquer sur "Lire" d\'un épisode appelle launchPlayback(epId, "episode")', async () => {
    mocks.getMediaItem.mockResolvedValue(SERIES_FIXTURE);
    mocks.listEpisodes.mockResolvedValue(SERIES_EPISODES_MIXED);

    render(DetailPage);
    await flushAsync();

    const playButtons = document.querySelectorAll(
      '[data-testid="detail-episode-play"]'
    );
    expect(playButtons.length).toBeGreaterThan(0);

    await fireEvent.click(playButtons[1] as HTMLButtonElement);
    await waitFor(() => {
      expect(mocks.launchPlayback).toHaveBeenCalledTimes(1);
    });
    expect(mocks.launchPlayback).toHaveBeenCalledWith(101, 'episode');
  });
});

describe('/detail/[id] +page.svelte — Série tous épisodes vus', () => {
  beforeEach(() => {
    mocks.pageStore?.set({ params: { id: '7' } });
  });

  it('affiche "Revoir depuis le début" et appelle launchRewatchFromStart(id) (Requirement 12.3)', async () => {
    mocks.getMediaItem.mockResolvedValue(SERIES_FIXTURE);
    mocks.listEpisodes.mockResolvedValue(SERIES_EPISODES_ALL_WATCHED);

    const { queryByTestId } = render(DetailPage);
    await flushAsync();

    const rewatchBtn = queryByTestId(
      'detail-rewatch-button'
    ) as HTMLButtonElement;
    expect(rewatchBtn).not.toBeNull();
    expect(rewatchBtn.textContent?.trim()).toBe('Revoir depuis le début');
    expect(queryByTestId('detail-resume-button')).toBeNull();

    await fireEvent.click(rewatchBtn);
    await waitFor(() => {
      expect(mocks.launchRewatchFromStart).toHaveBeenCalledTimes(1);
    });
    expect(mocks.launchRewatchFromStart).toHaveBeenCalledWith(7);
  });

  it('affiche le compteur "2 / 2 épisodes vus" quand tous sont vus', async () => {
    mocks.getMediaItem.mockResolvedValue(SERIES_FIXTURE);
    mocks.listEpisodes.mockResolvedValue(SERIES_EPISODES_ALL_WATCHED);

    const { queryByTestId } = render(DetailPage);
    await flushAsync();

    expect(queryByTestId('detail-episodes-counter')?.textContent).toContain(
      '2 / 2 épisodes vus'
    );
  });
});

describe('/detail/[id] +page.svelte — toggles favoris / à regarder', () => {
  beforeEach(() => {
    mocks.pageStore?.set({ params: { id: '42' } });
  });

  it('toggleFavorite est appelé et l\'état local est mis à jour', async () => {
    mocks.getMediaItem.mockResolvedValue(FILM_FIXTURE);
    mocks.toggleFavorite.mockResolvedValue(true);

    const { queryByTestId } = render(DetailPage);
    await flushAsync();

    const btn = queryByTestId('detail-toggle-favorite') as HTMLButtonElement;
    expect(btn).not.toBeNull();
    expect(btn.getAttribute('aria-pressed')).toBe('false');

    await fireEvent.click(btn);
    await waitFor(() => {
      expect(mocks.toggleFavorite).toHaveBeenCalledWith(42);
    });
    await waitFor(() => {
      expect(btn.getAttribute('aria-pressed')).toBe('true');
    });
  });

  it('toggleToWatch est appelé', async () => {
    mocks.getMediaItem.mockResolvedValue(FILM_FIXTURE);
    mocks.toggleToWatch.mockResolvedValue(true);

    const { queryByTestId } = render(DetailPage);
    await flushAsync();

    const btn = queryByTestId('detail-toggle-to-watch') as HTMLButtonElement;
    await fireEvent.click(btn);
    await waitFor(() => {
      expect(mocks.toggleToWatch).toHaveBeenCalledWith(42);
    });
  });
});

describe('/detail/[id] +page.svelte — gestion erreur', () => {
  it('affiche un message d\'erreur si getMediaItem rejette', async () => {
    mocks.getMediaItem.mockRejectedValue(new Error('IPC failure'));
    mocks.pageStore?.set({ params: { id: '42' } });

    const { queryByTestId } = render(DetailPage);
    await flushAsync();

    const error = queryByTestId('detail-load-error');
    expect(error).not.toBeNull();
    expect(error?.textContent).toContain('IPC failure');
  });

  it('refuse un id non parseable sans appeler le backend', async () => {
    mocks.pageStore?.set({ params: { id: 'abc' } });

    const { queryByTestId } = render(DetailPage);
    await flushAsync();

    expect(mocks.getMediaItem).not.toHaveBeenCalled();
    const error = queryByTestId('detail-load-error');
    expect(error).not.toBeNull();
    expect(error?.textContent).toContain('Identifiant invalide');
  });
});

describe('/detail/[id] +page.svelte — liens d\'édition', () => {
  it('contient un lien Éditer vers /edit/{id}', async () => {
    mocks.getMediaItem.mockResolvedValue(FILM_FIXTURE);

    const { queryByTestId } = render(DetailPage);
    await flushAsync();

    const link = queryByTestId('detail-edit-link') as HTMLAnchorElement;
    expect(link).not.toBeNull();
    expect(link.getAttribute('href')).toBe('/edit/42');
  });

  it('chaque épisode a un lien Éditer vers /edit-episode/{id}?seriesId={seriesId}', async () => {
    mocks.getMediaItem.mockResolvedValue(SERIES_FIXTURE);
    mocks.listEpisodes.mockResolvedValue(SERIES_EPISODES_MIXED);
    mocks.pageStore?.set({ params: { id: '7' } });

    render(DetailPage);
    await flushAsync();

    const links = document.querySelectorAll<HTMLAnchorElement>(
      '[data-testid="detail-episode-edit"]'
    );
    expect(links).toHaveLength(4);
    expect(links[0].getAttribute('href')).toBe('/edit-episode/100?seriesId=7');
    expect(links[3].getAttribute('href')).toBe('/edit-episode/200?seriesId=7');
  });
});
