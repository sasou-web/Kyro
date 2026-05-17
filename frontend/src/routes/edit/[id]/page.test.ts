/**
 * @vitest-environment jsdom
 *
 * Tests unitaires de la page d'édition Media_Item — tâche 15.5
 * (Requirements 6.1, 6.2, 6.3).
 *
 * Stratégie de mock :
 *   - `$lib/api/commands` : `getMediaItem` retourne une fiche
 *     synthétique au mount, `updateMediaItem` est espionné pour
 *     vérifier le payload envoyé.
 *   - `$app/navigation` : `goto` est espionné pour vérifier la
 *     redirection vers `/detail/{id}` après une soumission réussie.
 *   - `$app/stores` : on injecte un `page` store minimal portant le
 *     paramètre `id` de la route SvelteKit `[id]`.
 *   - `<PosterPicker>` est un composant qui ouvrirait un dialogue
 *     natif et appellerait des commandes Tauri ; on en mock un stub
 *     inerte pour ne pas casser le rendu.
 *
 * On vérifie :
 *   1. au mount, le formulaire est pré-rempli avec les données
 *      retournées par `getMediaItem` ;
 *   2. l'année hors [1900, 2099] déclenche le message de validation
 *      attendu et empêche tout appel `updateMediaItem` ;
 *   3. l'année vide est acceptée et envoyée comme `null` ;
 *   4. une soumission valide envoie le bon payload puis navigue vers
 *      `/detail/{id}` ;
 *   5. le multi-tag « genres » : Entrée pousse un chip, ✕ retire le
 *      chip ciblé ;
 *   6. le titre vide est refusé.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { writable } from 'svelte/store';

import type { MediaItemDetail } from '$lib/api/types';

// ---------------------------------------------------------------------------
// Mocks hoistés.
// ---------------------------------------------------------------------------

const mocks = vi.hoisted(() => {
  return {
    getMediaItem: vi.fn(),
    updateMediaItem: vi.fn(),
    goto: vi.fn(async () => undefined),
    pageStore: undefined as
      | ReturnType<typeof writable<{ params: { id: string } }>>
      | undefined
  };
});

vi.mock('$lib/api/commands', () => ({
  getMediaItem: mocks.getMediaItem,
  updateMediaItem: mocks.updateMediaItem
}));

vi.mock('$app/navigation', () => ({
  goto: mocks.goto
}));

vi.mock('$app/stores', async () => {
  const { writable } = await import('svelte/store');
  const store = writable<{ params: { id: string } }>({ params: { id: '42' } });
  mocks.pageStore = store;
  return { page: { subscribe: store.subscribe } };
});

// `<PosterPicker>` est mocké pour éviter de charger
// `@tauri-apps/plugin-dialog` et les chemins `appDataDir`. On
// rend un bouton inerte pour préserver une structure DOM simple.
vi.mock('$lib/components/PosterPicker.svelte', async () => {
  // Utiliser un stub Svelte minimal — un composant Svelte exporté
  // par défaut qui rend juste un `<button>` avec un libellé fixe.
  const Stub = (await import('./poster-picker-stub.svelte')).default;
  return { default: Stub };
});

import EditPage from './+page.svelte';

// ---------------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------------

const FIXTURE: MediaItemDetail = {
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

async function flushAsync(): Promise<void> {
  for (let i = 0; i < 10; i += 1) {
    await Promise.resolve();
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

beforeEach(() => {
  mocks.getMediaItem.mockReset();
  mocks.updateMediaItem.mockReset();
  mocks.goto.mockReset();
  mocks.goto.mockResolvedValue(undefined);
  mocks.pageStore?.set({ params: { id: '42' } });
});

afterEach(() => {
  cleanup();
});

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

describe('/edit/[id] +page.svelte', () => {
  it('charge la fiche et pré-remplit le formulaire au mount (requirement 6.1)', async () => {
    mocks.getMediaItem.mockResolvedValue(FIXTURE);

    const { container, getByLabelText } = render(EditPage);
    await flushAsync();

    expect(mocks.getMediaItem).toHaveBeenCalledWith(42);

    const titleInput = getByLabelText('Titre') as HTMLInputElement;
    expect(titleInput.value).toBe('Inception');

    const yearInput = getByLabelText('Année') as HTMLInputElement;
    expect(yearInput.value).toBe('2010');

    const synopsisInput = getByLabelText('Synopsis') as HTMLTextAreaElement;
    expect(synopsisInput.value).toBe('Un voleur entre dans les rêves.');

    const studioInput = getByLabelText('Studio ou réalisateur') as HTMLInputElement;
    expect(studioInput.value).toBe('Warner');

    // Les chips de genres sont rendus.
    const chips = container.querySelectorAll('[data-testid="genre-chip"]');
    expect(chips).toHaveLength(2);
    expect(chips[0].textContent).toContain('Science-fiction');
    expect(chips[1].textContent).toContain('Thriller');
  });

  it('rejette une année hors [1900, 2099] avec le message attendu (requirement 6.1)', async () => {
    mocks.getMediaItem.mockResolvedValue(FIXTURE);

    const { getByLabelText, getByText, queryByTestId } = render(EditPage);
    await flushAsync();

    const yearInput = getByLabelText('Année') as HTMLInputElement;
    await fireEvent.input(yearInput, { target: { value: '2200' } });

    const submitBtn = getByText('Enregistrer') as HTMLButtonElement;
    await fireEvent.click(submitBtn);
    await tick();

    const error = queryByTestId('edit-validation-error');
    expect(error).not.toBeNull();
    expect(error?.textContent).toContain(
      "L'année doit être comprise entre 1900 et 2099 ou laissée vide."
    );
    expect(mocks.updateMediaItem).not.toHaveBeenCalled();
  });

  it('accepte une année vide et envoie `null`', async () => {
    mocks.getMediaItem.mockResolvedValue(FIXTURE);
    mocks.updateMediaItem.mockResolvedValue(undefined);

    const { getByLabelText, getByText } = render(EditPage);
    await flushAsync();

    const yearInput = getByLabelText('Année') as HTMLInputElement;
    await fireEvent.input(yearInput, { target: { value: '' } });

    await fireEvent.click(getByText('Enregistrer'));
    await waitFor(() => {
      expect(mocks.updateMediaItem).toHaveBeenCalledTimes(1);
    });

    const [calledId, payload] = mocks.updateMediaItem.mock.calls[0] as [
      number,
      Record<string, unknown>
    ];
    expect(calledId).toBe(42);
    expect(payload.year).toBeNull();
  });

  it('rejette une année non entière (« 1995.5 »)', async () => {
    mocks.getMediaItem.mockResolvedValue(FIXTURE);

    const { getByLabelText, getByText, queryByTestId } = render(EditPage);
    await flushAsync();

    const yearInput = getByLabelText('Année') as HTMLInputElement;
    await fireEvent.input(yearInput, { target: { value: '1995.5' } });

    await fireEvent.click(getByText('Enregistrer'));
    await tick();

    expect(queryByTestId('edit-validation-error')).not.toBeNull();
    expect(mocks.updateMediaItem).not.toHaveBeenCalled();
  });

  it('refuse un titre vide', async () => {
    mocks.getMediaItem.mockResolvedValue(FIXTURE);

    const { getByLabelText, getByText, queryByTestId } = render(EditPage);
    await flushAsync();

    const titleInput = getByLabelText('Titre') as HTMLInputElement;
    await fireEvent.input(titleInput, { target: { value: '   ' } });

    await fireEvent.click(getByText('Enregistrer'));
    await tick();

    const error = queryByTestId('edit-validation-error');
    expect(error?.textContent).toContain('Le titre est obligatoire.');
    expect(mocks.updateMediaItem).not.toHaveBeenCalled();
  });

  it('soumet un payload valide puis redirige vers /detail/{id}', async () => {
    mocks.getMediaItem.mockResolvedValue(FIXTURE);
    mocks.updateMediaItem.mockResolvedValue(undefined);

    const { getByLabelText, getByText } = render(EditPage);
    await flushAsync();

    // Modification d'un champ pour s'assurer qu'on persiste bien la
    // saisie utilisateur (et pas seulement la fiche initiale).
    const titleInput = getByLabelText('Titre') as HTMLInputElement;
    await fireEvent.input(titleInput, { target: { value: 'Inception (édité)' } });

    const studioInput = getByLabelText('Studio ou réalisateur') as HTMLInputElement;
    await fireEvent.input(studioInput, { target: { value: 'Warner Bros.' } });

    await fireEvent.click(getByText('Enregistrer'));
    await waitFor(() => {
      expect(mocks.updateMediaItem).toHaveBeenCalledTimes(1);
    });

    const [calledId, payload] = mocks.updateMediaItem.mock.calls[0] as [
      number,
      Record<string, unknown>
    ];
    expect(calledId).toBe(42);
    expect(payload).toMatchObject({
      title: 'Inception (édité)',
      original_title: null,
      year: 2010,
      synopsis: 'Un voleur entre dans les rêves.',
      studio: 'Warner Bros.',
      user_rating: 9,
      status: 'non_vu'
    });

    await waitFor(() => {
      expect(mocks.goto).toHaveBeenCalledWith('/detail/42');
    });
  });

  it('ajoute un genre via Entrée et le retire via le bouton ✕', async () => {
    mocks.getMediaItem.mockResolvedValue({ ...FIXTURE, genres: [] });

    const { container, getByLabelText, getByLabelText: byLabel } = render(
      EditPage
    );
    await flushAsync();

    const genreInput = getByLabelText('Genres') as HTMLInputElement;
    await fireEvent.input(genreInput, { target: { value: 'Drame' } });
    await fireEvent.keyDown(genreInput, { key: 'Enter' });
    await tick();

    let chips = container.querySelectorAll('[data-testid="genre-chip"]');
    expect(chips).toHaveLength(1);
    expect(chips[0].textContent).toContain('Drame');
    // Le tampon doit avoir été vidé.
    expect((genreInput as HTMLInputElement).value).toBe('');

    // ',' déclenche aussi l'ajout.
    await fireEvent.input(genreInput, { target: { value: 'Comédie' } });
    await fireEvent.keyDown(genreInput, { key: ',' });
    await tick();

    chips = container.querySelectorAll('[data-testid="genre-chip"]');
    expect(chips).toHaveLength(2);

    // Suppression d'un chip via ✕.
    const removeBtn = byLabel('Retirer le genre Drame') as HTMLButtonElement;
    await fireEvent.click(removeBtn);
    await tick();

    chips = container.querySelectorAll('[data-testid="genre-chip"]');
    expect(chips).toHaveLength(1);
    expect(chips[0].textContent).toContain('Comédie');
  });

  it("affiche une erreur backend si updateMediaItem échoue (sans navigation)", async () => {
    mocks.getMediaItem.mockResolvedValue(FIXTURE);
    mocks.updateMediaItem.mockRejectedValue('DB lock');

    const { getByText, queryByTestId } = render(EditPage);
    await flushAsync();

    await fireEvent.click(getByText('Enregistrer'));
    await waitFor(() => {
      expect(queryByTestId('edit-submit-error')).not.toBeNull();
    });

    expect(queryByTestId('edit-submit-error')?.textContent).toContain(
      "Échec de l'enregistrement : DB lock"
    );
    expect(mocks.goto).not.toHaveBeenCalled();
  });
});
