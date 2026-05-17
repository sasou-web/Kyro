/**
 * Tests unitaires de `setupEventListeners`.
 *
 * Stratégie : on mocke `@tauri-apps/api/event` pour exposer une
 * `listen` espionnable qui mémorise tous les `(eventName, handler)`
 * souscrits, sans toucher à la couche IPC réelle. Chaque test vérifie
 * qu'un événement émis (en appelant directement le handler enregistré)
 * met bien à jour le store correspondant.
 *
 * On mocke également `@tauri-apps/api/core` pour neutraliser le
 * `invoke` utilisé par `listByCategory` (refresh post-scan / playback).
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';

/**
 * Force la résolution complète des chaînes de microtâches en attente.
 * `await Promise.resolve()` ne flush qu'un niveau ; on enchaîne
 * plusieurs `setTimeout(0)` pour drainer les `then` chaînés (le
 * refresh post-scan : `invoke().then(items.set)`).
 */
async function flushMicrotasks(): Promise<void> {
  for (let i = 0; i < 5; i += 1) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

vi.mock('@tauri-apps/api/event', () => {
  const handlers = new Map<string, (event: { payload: unknown }) => void>();
  return {
    handlers,
    listen: vi.fn(
      async (eventName: string, handler: (event: { payload: unknown }) => void) => {
        handlers.set(eventName, handler);
        return () => {
          handlers.delete(eventName);
        };
      }
    )
  };
});

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn()
}));

import { invoke } from '@tauri-apps/api/core';
import * as eventModule from '@tauri-apps/api/event';
import { setupEventListeners } from './events';
import { flags, items, scanProgress } from '$lib/stores/library';
import type { MediaItem, ScanProgress } from './types';

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;
// Cast vers le shape réel exposé par notre stub — `handlers` est
// ajouté à l'objet du module pour permettre aux tests de récupérer
// les handlers enregistrés.
const handlers = (eventModule as unknown as {
  handlers: Map<string, (event: { payload: unknown }) => void>;
}).handlers;

function emit(eventName: string, payload: unknown): void {
  const handler = handlers.get(eventName);
  if (!handler) {
    throw new Error(`Aucun handler enregistré pour ${eventName}`);
  }
  handler({ payload });
}

beforeEach(() => {
  // Reset stores entre chaque test pour ne pas polluer l'état.
  items.set([]);
  scanProgress.set(null);
  flags.set({ ffprobeMissing: false, backendDegraded: false });
  handlers.clear();
  invokeMock.mockReset();
});

afterEach(() => {
  invokeMock.mockReset();
});

describe('setupEventListeners', () => {
  it("souscrit aux six événements canoniques", async () => {
    await setupEventListeners();

    expect(handlers.has('scan-progress')).toBe(true);
    expect(handlers.has('scan-completed')).toBe(true);
    expect(handlers.has('ffprobe-missing')).toBe(true);
    expect(handlers.has('backend-degraded')).toBe(true);
    expect(handlers.has('external-player-missing')).toBe(true);
    expect(handlers.has('playback-ended')).toBe(true);
  });

  it('scan-progress met à jour le store scanProgress', async () => {
    await setupEventListeners();

    const payload: ScanProgress = {
      processed: 12,
      total: 100,
      current_file: 'C:/Films/Inception.mkv'
    };
    emit('scan-progress', payload);

    expect(get(scanProgress)).toEqual(payload);
  });

  it('scan-completed remet scanProgress à null et déclenche un refresh', async () => {
    const refreshed: MediaItem[] = [
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
    invokeMock.mockResolvedValueOnce(refreshed);

    await setupEventListeners('Films');
    scanProgress.set({ processed: 99, total: 100, current_file: 'x.mkv' });

    emit('scan-completed', null);

    expect(get(scanProgress)).toBeNull();
    // Laisse la microtâche du refresh se résoudre.
    await flushMicrotasks();
    expect(invokeMock).toHaveBeenCalledWith('list_by_category', { category: 'Films' });
    expect(get(items)).toEqual(refreshed);
  });

  it('scan-completed sans refreshCategory ne déclenche pas list_by_category', async () => {
    await setupEventListeners(null);
    emit('scan-completed', null);

    await flushMicrotasks();
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it('ffprobe-missing positionne flags.ffprobeMissing à true', async () => {
    await setupEventListeners();
    expect(get(flags).ffprobeMissing).toBe(false);

    emit('ffprobe-missing', null);

    expect(get(flags)).toEqual({ ffprobeMissing: true, backendDegraded: false });
  });

  it('backend-degraded positionne flags.backendDegraded à true', async () => {
    await setupEventListeners();
    expect(get(flags).backendDegraded).toBe(false);

    emit('backend-degraded', 'pool sqlite verrouillé');

    expect(get(flags)).toEqual({ ffprobeMissing: false, backendDegraded: true });
  });

  it('external-player-missing logue sans modifier les stores', async () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    await setupEventListeners();

    emit('external-player-missing', 'C:/missing/mpv.exe');

    expect(warnSpy).toHaveBeenCalled();
    expect(get(flags)).toEqual({ ffprobeMissing: false, backendDegraded: false });
    warnSpy.mockRestore();
  });

  it('playback-ended déclenche un refresh des items', async () => {
    const refreshed: MediaItem[] = [];
    invokeMock.mockResolvedValueOnce(refreshed);

    await setupEventListeners('Series');
    emit('playback-ended', 1234);

    await flushMicrotasks();
    expect(invokeMock).toHaveBeenCalledWith('list_by_category', { category: 'Series' });
  });

  it('la cleanup retournée détache tous les listeners', async () => {
    const cleanup = await setupEventListeners();
    expect(handlers.size).toBe(7);

    cleanup();

    expect(handlers.size).toBe(0);
  });
});
