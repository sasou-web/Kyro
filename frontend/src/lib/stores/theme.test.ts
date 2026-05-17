/**
 * @vitest-environment jsdom
 *
 * Tests unitaires du store `theme`.
 *
 * On mocke `$lib/api/commands` afin d'isoler la logique du store
 * du runtime Tauri. Pour que les modifications successives du store
 * (entre tests) n'altèrent pas l'attribut `data-theme` du DOM
 * partagé, on `resetModules` après chaque test : cela force un
 * nouvel import du module et donc une nouvelle souscription
 * indépendante.
 *
 * _Requirements: 16.1, 16.14_
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';

vi.mock('$lib/api/commands', () => ({
  getTheme: vi.fn(),
  setTheme: vi.fn()
}));

import * as commands from '$lib/api/commands';

const getThemeMock = commands.getTheme as unknown as ReturnType<typeof vi.fn>;
const setThemeMock = commands.setTheme as unknown as ReturnType<typeof vi.fn>;

beforeEach(() => {
  // Réinitialiser le DOM et les mocks avant chaque test.
  document.documentElement.removeAttribute('data-theme');
  getThemeMock.mockReset();
  setThemeMock.mockReset();
  vi.resetModules();
});

afterEach(() => {
  document.documentElement.removeAttribute('data-theme');
});

describe('theme store — initial value', () => {
  it('default store value is adwaita-dark before loadTheme is called', async () => {
    const { theme } = await import('./theme');
    expect(get(theme)).toBe('adwaita-dark');
  });

  it('subscribing applies data-theme to document.documentElement immediately', async () => {
    await import('./theme');
    expect(document.documentElement.getAttribute('data-theme')).toBe(
      'adwaita-dark'
    );
  });
});

describe('loadTheme', () => {
  it('reads get_theme() and applies the value to the DOM', async () => {
    getThemeMock.mockResolvedValueOnce('adwaita-light');
    const { loadTheme, theme } = await import('./theme');

    await loadTheme();

    expect(getThemeMock).toHaveBeenCalledTimes(1);
    expect(get(theme)).toBe('adwaita-light');
    expect(document.documentElement.getAttribute('data-theme')).toBe(
      'adwaita-light'
    );
  });

  it('falls back to adwaita-dark when backend returns an unknown value', async () => {
    // Cas défensif : valeur corrompue dans setting_value.
    getThemeMock.mockResolvedValueOnce('solarized');
    const { loadTheme, theme } = await import('./theme');

    await loadTheme();

    expect(get(theme)).toBe('adwaita-dark');
    expect(document.documentElement.getAttribute('data-theme')).toBe(
      'adwaita-dark'
    );
  });

  it('keeps adwaita-dark on first launch when backend returns the default', async () => {
    // Couvre le critère 16.1 : premier démarrage ⇒ adwaita-dark.
    getThemeMock.mockResolvedValueOnce('adwaita-dark');
    const { loadTheme, theme } = await import('./theme');

    await loadTheme();

    expect(get(theme)).toBe('adwaita-dark');
    expect(document.documentElement.getAttribute('data-theme')).toBe(
      'adwaita-dark'
    );
  });
});

describe('toggleTheme', () => {
  it('flips dark → light, persists via set_theme and updates the DOM', async () => {
    setThemeMock.mockResolvedValueOnce(undefined);
    const { toggleTheme, theme } = await import('./theme');

    await toggleTheme();

    expect(get(theme)).toBe('adwaita-light');
    expect(setThemeMock).toHaveBeenCalledWith('adwaita-light');
    expect(document.documentElement.getAttribute('data-theme')).toBe(
      'adwaita-light'
    );
  });

  it('flips light → dark on a second toggle', async () => {
    getThemeMock.mockResolvedValueOnce('adwaita-light');
    setThemeMock.mockResolvedValue(undefined);
    const { loadTheme, toggleTheme, theme } = await import('./theme');

    await loadTheme();
    await toggleTheme();

    expect(get(theme)).toBe('adwaita-dark');
    expect(setThemeMock).toHaveBeenLastCalledWith('adwaita-dark');
    expect(document.documentElement.getAttribute('data-theme')).toBe(
      'adwaita-dark'
    );
  });
});

describe('direct store mutation', () => {
  it('calling theme.set(...) reapplies the value to the DOM via subscription', async () => {
    const { theme } = await import('./theme');

    theme.set('adwaita-light');

    expect(document.documentElement.getAttribute('data-theme')).toBe(
      'adwaita-light'
    );
  });
});
