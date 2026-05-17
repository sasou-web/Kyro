/**
 * @vitest-environment jsdom
 *
 * Tests unitaires de `<PosterPicker>` (Tâche 14.7, Requirements
 * 6.2 / 6.3).
 *
 * On mocke :
 *   - `@tauri-apps/plugin-dialog` (le `open()` natif est remplacé par
 *     une fonction espionnable) ;
 *   - `@tauri-apps/api/path` (pour ne pas toucher au FS pendant les
 *     tests) ;
 *   - `$lib/api/commands` (les wrappers `importPosterForMedia` /
 *     `importPosterForEpisode` n'invoquent rien côté backend).
 *
 * On vérifie le contrat fonctionnel :
 *   1. clic ⇒ `open()` est appelé avec le bon filtre image ;
 *   2. fichier sélectionné ⇒ la bonne commande backend est appelée
 *      selon `kind` (media vs episode) avec `(id, sourcePath, postersDir)` ;
 *   3. annulation utilisateur (open retourne `null`) ⇒ aucune commande
 *      backend appelée, aucun statut affiché ;
 *   4. erreur backend ⇒ message d'erreur affiché en `role="alert"` ;
 *   5. succès ⇒ message succès + événement `imported` émis avec le
 *      chemin retourné.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';

vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: vi.fn()
}));

vi.mock('@tauri-apps/api/path', () => ({
  appDataDir: vi.fn(async () => 'C:/Users/test/AppData/Roaming/Kyro'),
  join: vi.fn(async (...parts: string[]) => parts.join('/'))
}));

vi.mock('$lib/api/commands', () => ({
  importPosterForMedia: vi.fn(),
  importPosterForEpisode: vi.fn()
}));

import { open } from '@tauri-apps/plugin-dialog';
import {
  importPosterForEpisode,
  importPosterForMedia
} from '$lib/api/commands';
import PosterPicker from './PosterPicker.svelte';

beforeEach(() => {
  vi.mocked(open).mockReset();
  vi.mocked(importPosterForMedia).mockReset();
  vi.mocked(importPosterForEpisode).mockReset();
});

afterEach(() => {
  cleanup();
});

describe('PosterPicker', () => {
  it('ouvre le dialogue avec un filtre image au clic (requirement 6.2)', async () => {
    vi.mocked(open).mockResolvedValue(null);

    const { getByRole } = render(PosterPicker, { kind: 'media', id: 1 });
    const button = getByRole('button');

    await fireEvent.click(button);

    expect(open).toHaveBeenCalledTimes(1);
    const args = vi.mocked(open).mock.calls[0][0] as {
      multiple: boolean;
      directory: boolean;
      filters: Array<{ name: string; extensions: string[] }>;
    };
    expect(args.multiple).toBe(false);
    expect(args.directory).toBe(false);
    expect(args.filters).toHaveLength(1);
    expect(args.filters[0].extensions).toEqual([
      'png',
      'jpg',
      'jpeg',
      'webp',
      'bmp'
    ]);
  });

  it('appelle import_poster_for_media avec (id, source, postersDir) quand kind=media (requirement 6.3)', async () => {
    vi.mocked(open).mockResolvedValue('C:/Pictures/cover.jpg');
    vi.mocked(importPosterForMedia).mockResolvedValue(
      'C:/Users/test/AppData/Roaming/Kyro/posters/media-42.jpg'
    );

    const { getByRole } = render(PosterPicker, { kind: 'media', id: 42 });

    await fireEvent.click(getByRole('button'));

    await waitFor(() => {
      expect(importPosterForMedia).toHaveBeenCalledTimes(1);
    });
    expect(importPosterForMedia).toHaveBeenCalledWith(
      42,
      'C:/Pictures/cover.jpg',
      'C:/Users/test/AppData/Roaming/Kyro/posters'
    );
    expect(importPosterForEpisode).not.toHaveBeenCalled();
  });

  it('appelle import_poster_for_episode quand kind=episode (requirement 6.5)', async () => {
    vi.mocked(open).mockResolvedValue('C:/Pictures/ep.png');
    vi.mocked(importPosterForEpisode).mockResolvedValue(
      'C:/Users/test/AppData/Roaming/Kyro/posters/episode-7.png'
    );

    const { getByRole } = render(PosterPicker, { kind: 'episode', id: 7 });

    await fireEvent.click(getByRole('button'));

    await waitFor(() => {
      expect(importPosterForEpisode).toHaveBeenCalledTimes(1);
    });
    expect(importPosterForEpisode).toHaveBeenCalledWith(
      7,
      'C:/Pictures/ep.png',
      'C:/Users/test/AppData/Roaming/Kyro/posters'
    );
    expect(importPosterForMedia).not.toHaveBeenCalled();
  });

  it("n'appelle aucune commande quand l'utilisateur annule le dialogue", async () => {
    vi.mocked(open).mockResolvedValue(null);

    const { getByRole, queryByRole } = render(PosterPicker, {
      kind: 'media',
      id: 1
    });

    await fireEvent.click(getByRole('button'));

    await waitFor(() => {
      // Le bouton n'est plus en cours (busy=false).
      expect((getByRole('button') as HTMLButtonElement).disabled).toBe(false);
    });
    expect(importPosterForMedia).not.toHaveBeenCalled();
    expect(importPosterForEpisode).not.toHaveBeenCalled();
    expect(queryByRole('alert')).toBeNull();
    expect(queryByRole('status')).toBeNull();
  });

  it('affiche un message de succès et dispatch `imported` après un import réussi', async () => {
    vi.mocked(open).mockResolvedValue('C:/Pictures/cover.jpg');
    vi.mocked(importPosterForMedia).mockResolvedValue(
      'C:/Users/test/AppData/Roaming/Kyro/posters/media-3.jpg'
    );

    const dispatched: Array<{ kind: 'media' | 'episode'; id: number; destPath: string }> = [];
    const { getByRole, component } = render(PosterPicker, {
      kind: 'media',
      id: 3
    });
    component.$on('imported', (e: CustomEvent) => dispatched.push(e.detail));

    await fireEvent.click(getByRole('button'));

    await waitFor(() => {
      expect(getByRole('status')).not.toBeNull();
    });
    expect(getByRole('status').textContent).toContain('Affiche importée.');
    expect(dispatched).toHaveLength(1);
    expect(dispatched[0]).toEqual({
      kind: 'media',
      id: 3,
      destPath: 'C:/Users/test/AppData/Roaming/Kyro/posters/media-3.jpg'
    });
  });

  it("affiche un message d'erreur quand le backend rejette l'import", async () => {
    vi.mocked(open).mockResolvedValue('C:/Pictures/cover.svg');
    vi.mocked(importPosterForMedia).mockRejectedValue(
      'Extension non supportée'
    );

    const { getByRole } = render(PosterPicker, { kind: 'media', id: 9 });

    await fireEvent.click(getByRole('button'));

    await waitFor(() => {
      expect(getByRole('alert')).not.toBeNull();
    });
    expect(getByRole('alert').textContent).toContain(
      "Échec de l'import : Extension non supportée"
    );
  });
});
