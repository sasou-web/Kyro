/**
 * @vitest-environment jsdom
 *
 * Tests unitaires du layout racine `+layout.svelte` (tâche 15.1).
 *
 * Couverture :
 *  - les bandeaux apparaissent en fonction des stores (`flags`,
 *    `scanProgress`) et disparaissent quand les drapeaux retombent ;
 *  - le bandeau `ffprobe-missing` est dismissable (le badge TopBar
 *    reste affiché par ailleurs) ;
 *  - le bandeau `backend-degraded` expose un bouton « Relancer » ;
 *  - au mount, les fonctions d'init (`loadTheme`,
 *    `setupEventListeners`, `initSettings`) sont appelées dans
 *    l'ordre attendu.
 *
 * Stratégie de mock :
 *   - `$lib/stores/theme` → `loadTheme` espion résolvant void ;
 *   - `$lib/api/events` → `setupEventListeners` espion retournant
 *     une cleanup espionnée ;
 *   - `$lib/stores/init` → `initSettings` espion résolvant void ;
 *   - `$app/stores` → store `page` minimal (consommé par Sidebar).
 *
 * Note : on utilise `vi.hoisted` pour déclarer les espions avant les
 * factories `vi.mock` (elles-mêmes hoistées par Vitest), évitant la
 * `ReferenceError: Cannot access X before initialization` qui
 * surviendrait si on déclarait les `vi.fn()` avec un simple `const`.
 *
 * _Requirements: 5.4, 18.4, 16.1_
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render } from '@testing-library/svelte';
import { tick } from 'svelte';

const mocks = vi.hoisted(() => {
  return {
    loadTheme: vi.fn(async () => undefined),
    setupEventListeners: vi.fn(),
    cleanupListeners: vi.fn(() => undefined),
    initSettings: vi.fn(async () => undefined)
  };
});

vi.mock('$app/stores', async () => {
  const { writable } = await import('svelte/store');
  const pageStore = writable<{ url: { pathname: string } }>({
    url: { pathname: '/films' }
  });
  return {
    page: { subscribe: pageStore.subscribe }
  };
});

vi.mock('$lib/stores/theme', () => ({
  loadTheme: mocks.loadTheme,
  toggleTheme: vi.fn(),
  theme: { subscribe: () => () => undefined }
}));

vi.mock('$lib/api/events', () => ({
  setupEventListeners: mocks.setupEventListeners
}));

vi.mock('$lib/stores/init', () => ({
  initSettings: mocks.initSettings
}));

import Layout from './+layout.svelte';
import { flags, scanProgress } from '$lib/stores/library';

/**
 * Drain les microtâches en attente : `loadTheme`, puis
 * `setupEventListeners`, puis `initSettings` sont chaînés via
 * `await` dans le `onMount` du layout. Plusieurs flushs successifs
 * laissent tous les `then` se résoudre.
 */
async function flushAsync(): Promise<void> {
  for (let i = 0; i < 10; i += 1) {
    await Promise.resolve();
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

beforeEach(() => {
  scanProgress.set(null);
  flags.set({ ffprobeMissing: false, backendDegraded: false });
  mocks.loadTheme.mockClear();
  mocks.setupEventListeners.mockReset();
  mocks.setupEventListeners.mockImplementation(async () => mocks.cleanupListeners);
  mocks.initSettings.mockClear();
  mocks.cleanupListeners.mockClear();
});

afterEach(() => {
  cleanup();
  scanProgress.set(null);
  flags.set({ ffprobeMissing: false, backendDegraded: false });
});

describe('+layout.svelte — initialisation au mount', () => {
  it('appelle loadTheme, setupEventListeners et initSettings au mount', async () => {
    render(Layout);
    await flushAsync();

    expect(mocks.loadTheme).toHaveBeenCalledTimes(1);
    expect(mocks.setupEventListeners).toHaveBeenCalledTimes(1);
    expect(mocks.initSettings).toHaveBeenCalledTimes(1);
  });

  it('appelle setupEventListeners avant initSettings (pas de race)', async () => {
    const order: string[] = [];
    mocks.setupEventListeners.mockImplementationOnce(async () => {
      order.push('setupEventListeners');
      return mocks.cleanupListeners;
    });
    mocks.initSettings.mockImplementationOnce(async () => {
      order.push('initSettings');
    });

    render(Layout);
    await flushAsync();

    expect(order).toEqual(['setupEventListeners', 'initSettings']);
  });
});

describe('+layout.svelte — bandeaux', () => {
  it("n'affiche aucun bandeau quand tous les drapeaux sont à false", async () => {
    const { queryByTestId } = render(Layout);
    await flushAsync();

    expect(queryByTestId('banner-ffprobe-missing')).toBeNull();
    expect(queryByTestId('banner-backend-degraded')).toBeNull();
    expect(queryByTestId('banner-scan-progress')).toBeNull();
  });

  it('affiche le bandeau ffprobe-missing quand flags.ffprobeMissing est true', async () => {
    const { queryByTestId } = render(Layout);
    flags.set({ ffprobeMissing: true, backendDegraded: false });
    await tick();

    const banner = queryByTestId('banner-ffprobe-missing');
    expect(banner).not.toBeNull();
    expect(banner?.textContent).toMatch(/ffprobe/i);
  });

  it('le bandeau ffprobe-missing est dismissable', async () => {
    const { queryByTestId, getByLabelText } = render(Layout);
    flags.set({ ffprobeMissing: true, backendDegraded: false });
    await tick();
    expect(queryByTestId('banner-ffprobe-missing')).not.toBeNull();

    const closeBtn = getByLabelText('Masquer ce bandeau') as HTMLButtonElement;
    closeBtn.click();
    await tick();

    expect(queryByTestId('banner-ffprobe-missing')).toBeNull();
  });

  it('affiche le bandeau backend-degraded avec un bouton Relancer', async () => {
    const { queryByTestId, getByText } = render(Layout);
    flags.set({ ffprobeMissing: false, backendDegraded: true });
    await tick();

    const banner = queryByTestId('banner-backend-degraded');
    expect(banner).not.toBeNull();
    expect(getByText('Relancer')).toBeTruthy();
  });

  it('affiche le bandeau scan-progress avec processed/total et fichier courant', async () => {
    const { queryByTestId } = render(Layout);
    scanProgress.set({
      processed: 7,
      total: 42,
      current_file: 'D:/Films/Inception.2010.1080p.mkv'
    });
    await tick();

    const banner = queryByTestId('banner-scan-progress');
    expect(banner).not.toBeNull();
    const text = banner?.textContent ?? '';
    expect(text).toContain('7');
    expect(text).toContain('42');
    expect(text).toContain('Inception.2010.1080p.mkv');
  });

  it('peut afficher les trois bandeaux simultanément', async () => {
    const { queryByTestId } = render(Layout);
    flags.set({ ffprobeMissing: true, backendDegraded: true });
    scanProgress.set({ processed: 1, total: 10, current_file: 'a.mkv' });
    await tick();

    expect(queryByTestId('banner-ffprobe-missing')).not.toBeNull();
    expect(queryByTestId('banner-backend-degraded')).not.toBeNull();
    expect(queryByTestId('banner-scan-progress')).not.toBeNull();
  });

  it('retire le bandeau scan-progress quand scanProgress repasse à null', async () => {
    scanProgress.set({ processed: 5, total: 10, current_file: 'x.mkv' });

    const { queryByTestId } = render(Layout);
    await tick();
    expect(queryByTestId('banner-scan-progress')).not.toBeNull();

    scanProgress.set(null);
    await tick();
    expect(queryByTestId('banner-scan-progress')).toBeNull();
  });
});
