/**
 * @vitest-environment jsdom
 *
 * Tests unitaires de la page d'édition Episode_Item — tâche 15.6
 * (Requirement 6.5).
 *
 * Stratégie de mock — calquée sur `routes/edit/[id]/page.test.ts` :
 *   - `$lib/api/commands` : `listEpisodes` retourne une liste
 *     synthétique au mount, `updateEpisodeItem` est espionné pour
 *     vérifier le payload envoyé.
 *   - `$app/navigation` : `goto` est espionné pour vérifier les
 *     redirections vers `/detail/{seriesId}` et `/edit/{seriesId}`.
 *   - `$app/stores` : on injecte un `page` store minimal portant
 *     `params.id` et `url` (avec `searchParams.get('seriesId')`).
 *   - `<PosterPicker>` est mocké par un stub inerte (cf.
 *     `poster-picker-stub.svelte`).
 *
 * On vérifie :
 *   1. au mount, l'épisode est chargé via `listEpisodes(seriesId)`,
 *      et le statut courant est pré-rempli ;
 *   2. l'absence du paramètre `seriesId` produit un message d'erreur
 *      explicite et n'appelle pas `listEpisodes` ;
 *   3. un id absent de la liste produit un message « introuvable » ;
 *   4. les onglets « Série / Saison / Épisode » sont présents,
 *      le panneau « Épisode » est actif par défaut, et cliquer sur
 *      « Éditer la fiche série » navigue vers `/edit/{seriesId}` ;
 *   5. une soumission valide envoie le bon payload (`updateEpisodeItem`)
 *      puis navigue vers `/detail/{seriesId}` ;
 *   6. les chaînes vides (titre, synopsis) sont envoyées comme `null` ;
 *   7. une erreur backend lors de la soumission s'affiche sans
 *      navigation.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { writable } from 'svelte/store';

import type { EpisodeItem } from '$lib/api/types';

// ---------------------------------------------------------------------------
// Mocks hoistés.
// ---------------------------------------------------------------------------

type PageState = {
  params: { id: string };
  url: URL;
};

const mocks = vi.hoisted(() => {
  return {
    listEpisodes: vi.fn(),
    listByCategory: vi.fn(),
    updateEpisodeItem: vi.fn(),
    goto: vi.fn(async () => undefined),
    pageStore: undefined as ReturnType<typeof writable<PageState>> | undefined
  };
});

vi.mock('$lib/api/commands', () => ({
  listEpisodes: mocks.listEpisodes,
  listByCategory: mocks.listByCategory,
  updateEpisodeItem: mocks.updateEpisodeItem
}));

vi.mock('$app/navigation', () => ({
  goto: mocks.goto
}));

vi.mock('$app/stores', async () => {
  const { writable } = await import('svelte/store');
  const store = writable<PageState>({
    params: { id: '7' },
    url: new URL('http://localhost/edit-episode/7?seriesId=3')
  });
  mocks.pageStore = store;
  return { page: { subscribe: store.subscribe } };
});

// `<PosterPicker>` est mocké pour éviter de charger
// `@tauri-apps/plugin-dialog` et `appDataDir`.
vi.mock('$lib/components/PosterPicker.svelte', async () => {
  const Stub = (await import('./poster-picker-stub.svelte')).default;
  return { default: Stub };
});

import EditEpisodePage from './+page.svelte';

// ---------------------------------------------------------------------------
// Helpers / fixtures.
// ---------------------------------------------------------------------------

const FIXTURE_EPISODES: EpisodeItem[] = [
  {
    id: 5,
    season_number: 1,
    episode_number: 1,
    status: 'vu',
    available: true
  },
  {
    id: 7,
    season_number: 1,
    episode_number: 2,
    status: 'en_cours',
    available: true
  },
  {
    id: 9,
    season_number: 2,
    episode_number: 1,
    status: 'non_vu',
    available: true
  }
];

async function flushAsync(): Promise<void> {
  for (let i = 0; i < 10; i += 1) {
    await Promise.resolve();
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

beforeEach(() => {
  mocks.listEpisodes.mockReset();
  mocks.listByCategory.mockReset();
  mocks.updateEpisodeItem.mockReset();
  mocks.goto.mockReset();
  mocks.goto.mockResolvedValue(undefined);
  mocks.pageStore?.set({
    params: { id: '7' },
    url: new URL('http://localhost/edit-episode/7?seriesId=3')
  });
});

afterEach(() => {
  cleanup();
});

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

describe('/edit-episode/[id] +page.svelte', () => {
  it("charge l'épisode via listEpisodes(seriesId) et pré-remplit le statut (Requirement 6.5)", async () => {
    mocks.listEpisodes.mockResolvedValue(FIXTURE_EPISODES);

    const { getByLabelText, getByRole } = render(EditEpisodePage);
    await flushAsync();

    expect(mocks.listEpisodes).toHaveBeenCalledWith(3);

    const statusSelect = getByLabelText('Statut') as HTMLSelectElement;
    expect(statusSelect.value).toBe('en_cours');

    // Titre de la page : « Modifier l'épisode S01E02 ».
    const heading = getByRole('heading', { level: 1 });
    expect(heading.textContent).toContain('S01E02');
  });

  it("résout seriesId automatiquement quand il est absent de l'URL", async () => {
    // Sans `?seriesId=…`, la page interroge `listByCategory` puis
    // `listEpisodes` pour retrouver la série parente de l'épisode 7.
    mocks.pageStore?.set({
      params: { id: '7' },
      url: new URL('http://localhost/edit-episode/7')
    });
    mocks.listByCategory.mockResolvedValueOnce([
      { id: 3, title: 'Bleach Kai', original_title: null, year: null,
        type: 'Anime', status: 'non_vu', genres: [] }
    ]);
    mocks.listByCategory.mockResolvedValueOnce([]);
    mocks.listEpisodes
      .mockResolvedValueOnce(FIXTURE_EPISODES) // pour la résolution
      .mockResolvedValueOnce(FIXTURE_EPISODES); // pour le pré-remplissage

    const { getByLabelText } = render(EditEpisodePage);
    await flushAsync();

    expect(mocks.listByCategory).toHaveBeenCalledWith('Series');
    expect(mocks.listByCategory).toHaveBeenCalledWith('Animes');
    const statusSelect = getByLabelText('Statut') as HTMLSelectElement;
    expect(statusSelect.value).toBe('en_cours');
  });

  it("affiche un message d'erreur quand l'épisode est introuvable dans toutes les séries", async () => {
    mocks.pageStore?.set({
      params: { id: '999' },
      url: new URL('http://localhost/edit-episode/999')
    });
    mocks.listByCategory.mockResolvedValue([]);

    const { queryByTestId } = render(EditEpisodePage);
    await flushAsync();

    const err = queryByTestId('edit-episode-load-error');
    expect(err).not.toBeNull();
    expect(err?.textContent).toContain('introuvable');
  });

  it("affiche un message si l'épisode est absent de la liste", async () => {
    mocks.listEpisodes.mockResolvedValue([
      // Pas d'épisode avec id = 7.
      { id: 1, season_number: 1, episode_number: 1, status: 'non_vu', available: true }
    ] satisfies EpisodeItem[]);

    const { queryByTestId } = render(EditEpisodePage);
    await flushAsync();

    const err = queryByTestId('edit-episode-load-error');
    expect(err).not.toBeNull();
    expect(err?.textContent).toContain('introuvable');
  });

  it("affiche les trois onglets et navigue vers /edit/{seriesId} depuis l'onglet série", async () => {
    mocks.listEpisodes.mockResolvedValue(FIXTURE_EPISODES);

    const { getByTestId, getByText, queryByTestId } = render(EditEpisodePage);
    await flushAsync();

    // Les trois onglets sont présents.
    expect(getByTestId('tab-serie')).not.toBeNull();
    expect(getByTestId('tab-saison')).not.toBeNull();
    expect(getByTestId('tab-episode')).not.toBeNull();

    // Le panneau « Épisode » est actif par défaut.
    expect(queryByTestId('panel-episode')).not.toBeNull();
    expect(queryByTestId('panel-serie')).toBeNull();

    // Bascule vers « Série ».
    await fireEvent.click(getByTestId('tab-serie'));
    await tick();
    expect(queryByTestId('panel-serie')).not.toBeNull();

    // Le bouton « Éditer la fiche série » navigue.
    await fireEvent.click(getByText('Éditer la fiche série'));
    await waitFor(() => {
      expect(mocks.goto).toHaveBeenCalledWith('/edit/3');
    });
  });

  it('soumet un payload valide puis redirige vers /detail/{seriesId}', async () => {
    mocks.listEpisodes.mockResolvedValue(FIXTURE_EPISODES);
    mocks.updateEpisodeItem.mockResolvedValue(undefined);

    const { getByLabelText, getByText } = render(EditEpisodePage);
    await flushAsync();

    const titleInput = getByLabelText('Titre') as HTMLInputElement;
    await fireEvent.input(titleInput, { target: { value: 'Le pilote' } });

    const synopsisInput = getByLabelText('Synopsis') as HTMLTextAreaElement;
    await fireEvent.input(synopsisInput, {
      target: { value: 'Tout commence ici.' }
    });

    const statusSelect = getByLabelText('Statut') as HTMLSelectElement;
    await fireEvent.change(statusSelect, { target: { value: 'vu' } });

    await fireEvent.click(getByText('Enregistrer'));
    await waitFor(() => {
      expect(mocks.updateEpisodeItem).toHaveBeenCalledTimes(1);
    });

    const [calledId, payload] = mocks.updateEpisodeItem.mock.calls[0] as [
      number,
      Record<string, unknown>
    ];
    expect(calledId).toBe(7);
    expect(payload).toEqual({
      title: 'Le pilote',
      synopsis: 'Tout commence ici.',
      status: 'vu'
    });

    await waitFor(() => {
      expect(mocks.goto).toHaveBeenCalledWith('/detail/3');
    });
  });

  it("envoie `null` pour les champs texte vides", async () => {
    mocks.listEpisodes.mockResolvedValue(FIXTURE_EPISODES);
    mocks.updateEpisodeItem.mockResolvedValue(undefined);

    const { getByText } = render(EditEpisodePage);
    await flushAsync();

    await fireEvent.click(getByText('Enregistrer'));
    await waitFor(() => {
      expect(mocks.updateEpisodeItem).toHaveBeenCalledTimes(1);
    });

    const [, payload] = mocks.updateEpisodeItem.mock.calls[0] as [
      number,
      Record<string, unknown>
    ];
    expect(payload.title).toBeNull();
    expect(payload.synopsis).toBeNull();
    // Statut non modifié → reste celui de la fiture.
    expect(payload.status).toBe('en_cours');
  });

  it("affiche une erreur backend si updateEpisodeItem échoue (sans navigation)", async () => {
    mocks.listEpisodes.mockResolvedValue(FIXTURE_EPISODES);
    mocks.updateEpisodeItem.mockRejectedValue('DB lock');

    const { getByText, queryByTestId } = render(EditEpisodePage);
    await flushAsync();

    await fireEvent.click(getByText('Enregistrer'));
    await waitFor(() => {
      expect(queryByTestId('edit-episode-submit-error')).not.toBeNull();
    });

    expect(queryByTestId('edit-episode-submit-error')?.textContent).toContain(
      "Échec de l'enregistrement : DB lock"
    );
    expect(mocks.goto).not.toHaveBeenCalled();
  });

  it("annule sans persister et redirige vers /detail/{seriesId}", async () => {
    mocks.listEpisodes.mockResolvedValue(FIXTURE_EPISODES);

    const { getByText } = render(EditEpisodePage);
    await flushAsync();

    await fireEvent.click(getByText('Annuler'));
    await waitFor(() => {
      expect(mocks.goto).toHaveBeenCalledWith('/detail/3');
    });
    expect(mocks.updateEpisodeItem).not.toHaveBeenCalled();
  });
});
