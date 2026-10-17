/**
 * Tests unitaires de `initSettings` (tâche 15.1).
 *
 * Couverture :
 *  - succès : la commande Tauri `get_settings_summary` est appelée
 *    et son résultat est écrit dans le store `settings` ;
 *  - échec : une erreur lors de la lecture des settings positionne
 *    `flags.backendDegraded` à `true` (mode dégradé,
 *    requirement 18.4) sans propager l'exception.
 *
 * On mocke `@tauri-apps/api/core` pour neutraliser l'IPC réel ;
 * `getSettingsSummary` (wrapper dans `commands.ts`) appelle
 * directement `invoke('get_settings_summary')`.
 *
 * _Requirements: 5.4, 18.4, 16.1_
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn()
}));

import { invoke } from '@tauri-apps/api/core';
import { initSettings } from './init';
import { flags, settings } from './library';
import type { SettingsSummary } from '$lib/api/types';

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

beforeEach(() => {
  settings.set(null);
  flags.set({ ffprobeMissing: false, backendDegraded: false });
  invokeMock.mockReset();
});

afterEach(() => {
  invokeMock.mockReset();
});

describe('initSettings', () => {
  it('écrit le SettingsSummary retourné dans le store settings', async () => {
    const summary: SettingsSummary = {
      library_folders: [
        { id: 1, path: 'D:/Films', category: 'Films' }
      ],
      external_player_path: 'C:/Program Files/mpv.net/mpvnet.exe',
      ffprobe_path: null,
      scan_at_startup: true,
      theme: 'adwaita-dark',
      last_category: 'Films',
      playback_mode: 'embedded'
    };
    invokeMock.mockResolvedValueOnce(summary);

    await initSettings();

    expect(invokeMock).toHaveBeenCalledWith('get_settings_summary');
    expect(get(settings)).toEqual(summary);
    expect(get(flags)).toEqual({ ffprobeMissing: false, backendDegraded: false });
  });

  it('positionne flags.backendDegraded en cas d\'échec sans propager l\'erreur', async () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    invokeMock.mockRejectedValueOnce(new Error('pool sqlite verrouillé'));

    await expect(initSettings()).resolves.toBeUndefined();

    expect(get(settings)).toBeNull();
    expect(get(flags)).toEqual({ ffprobeMissing: false, backendDegraded: true });
    expect(warnSpy).toHaveBeenCalled();
    warnSpy.mockRestore();
  });
});
