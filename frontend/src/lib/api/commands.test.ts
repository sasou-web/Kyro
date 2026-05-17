/**
 * Tests unitaires des wrappers `commands.ts`.
 *
 * On mocke `@tauri-apps/api/core` avant l'import de `commands.ts`
 * (`vi.mock` est hissé en haut du module par Vitest) afin que les
 * wrappers utilisent une `invoke` espionnable. Chaque test vérifie :
 *   1. le **nom de la commande** passé à `invoke` (chaîne snake_case
 *      identique au nom Rust),
 *   2. l'**objet d'arguments** transmis (clés en camelCase, valeurs
 *      transmises telles quelles),
 *   3. la **valeur de retour** est bien propagée par le wrapper.
 *
 * Couverture intentionnellement minimale : 5 wrappers représentatifs
 * — un par couche (library, scan, playback, settings, logs) — plutôt
 * qu'un test par commande, ce qui validerait la même mécanique
 * d'`invoke` 30+ fois.
 */

import { afterEach, describe, expect, it, vi } from 'vitest';

// Mock du runtime Tauri. Le hoisting de `vi.mock` garantit que
// l'import suivant de `commands.ts` reçoit bien notre stub.
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn()
}));

import { invoke } from '@tauri-apps/api/core';
import {
  listByCategory,
  playMedia,
  readLatestLog,
  setExternalPlayerPath,
  startScan,
  updateMediaItem
} from './commands';
import type { MediaItem, UpdateMediaItemInput } from './types';

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

afterEach(() => {
  invokeMock.mockReset();
});

describe('library wrappers', () => {
  it('listByCategory calls invoke with the right command and category arg', async () => {
    const fixture: MediaItem[] = [
      {
        id: 1,
        title: 'Inception',
        original_title: null,
        year: 2010,
        type: 'Film',
        status: 'non_vu',
        genres: ['Sci-Fi']
      }
    ];
    invokeMock.mockResolvedValueOnce(fixture);

    const result = await listByCategory('Films');

    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenCalledWith('list_by_category', {
      category: 'Films'
    });
    expect(result).toEqual(fixture);
  });

  it('updateMediaItem forwards id and the full input payload', async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    const input: UpdateMediaItemInput = {
      title: 'Inception',
      original_title: null,
      year: 2010,
      synopsis: null,
      studio: null,
      user_rating: 9,
      status: 'vu'
    };

    await updateMediaItem(42, input);

    expect(invokeMock).toHaveBeenCalledWith('update_media_item', {
      id: 42,
      input
    });
  });
});

describe('scan wrapper', () => {
  it('startScan calls invoke with no arguments', async () => {
    invokeMock.mockResolvedValueOnce(undefined);

    await startScan();

    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenCalledWith('start_scan');
  });
});

describe('playback wrapper', () => {
  it('playMedia forwards both targetId (camelCase) and kind', async () => {
    invokeMock.mockResolvedValueOnce(undefined);

    await playMedia(7, 'episode');

    expect(invokeMock).toHaveBeenCalledWith('play_media', {
      targetId: 7,
      kind: 'episode'
    });
  });
});

describe('settings wrapper', () => {
  it('setExternalPlayerPath forwards the path argument', async () => {
    invokeMock.mockResolvedValueOnce(undefined);

    await setExternalPlayerPath('C:/Program Files/mpv.net/mpv.net.exe');

    expect(invokeMock).toHaveBeenCalledWith('set_external_player_path', {
      path: 'C:/Program Files/mpv.net/mpv.net.exe'
    });
  });
});

describe('logs wrapper', () => {
  it('readLatestLog returns the string payload as-is', async () => {
    invokeMock.mockResolvedValueOnce('line1\nline2\n');

    const out = await readLatestLog();

    expect(invokeMock).toHaveBeenCalledWith('read_latest_log');
    expect(out).toBe('line1\nline2\n');
  });
});
