/**
 * @vitest-environment jsdom
 *
 * Tests unitaires de `<Sidebar>`.
 *
 * Couverture :
 *  - chacune des six entrées de navigation est rendue avec le bon
 *    libellé et le bon `href` (requirements 7.1, 11.4, 11.5) ;
 *  - le lien correspondant à la route courante reçoit la classe
 *    `.active` et `aria-current="page"` ;
 *  - le changement de route met à jour la mise en évidence sans
 *    recréer le composant.
 *
 * Le store `page` de `$app/stores` est remplacé par un writable
 * local : SvelteKit n'instancie son contexte que pendant un cycle
 * SSR / mount complet, ce qu'on ne peut pas reproduire dans
 * `@testing-library/svelte`. La substitution se fait via
 * `vi.mock('$app/stores', ...)` — c'est la stratégie recommandée
 * par la documentation SvelteKit pour les tests unitaires de
 * composants qui ne sortent pas de leur propre périmètre.
 *
 * _Requirements: 7.1, 11.4, 11.5_
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render } from '@testing-library/svelte';
import { tick } from 'svelte';

// Le store `page` mocké est un writable simple — on ne renseigne
// que `url.pathname`, seul champ consommé par le composant. On
// construit le store *à l'intérieur* du factory de `vi.mock`
// (lui-même hoisté) puis on le récupère via un import statique :
// les imports du test sont analysés après le hoisting du mock,
// donc la référence pointe bien vers le même writable.
vi.mock('$app/stores', async () => {
  const { writable } = await import('svelte/store');
  const pageStore = writable<{ url: { pathname: string } }>({
    url: { pathname: '/films' }
  });
  return {
    page: { subscribe: pageStore.subscribe },
    __pageStore: pageStore
  };
});

import * as appStores from '$app/stores';
const pageStore = (
  appStores as unknown as { __pageStore: import('svelte/store').Writable<{ url: { pathname: string } }> }
).__pageStore;

import Sidebar from './Sidebar.svelte';

beforeEach(() => {
  pageStore.set({ url: { pathname: '/films' } });
});

afterEach(() => {
  cleanup();
});

describe('<Sidebar> — rendu des liens', () => {
  it('rend les six entrées de navigation avec leur libellé et leur href', () => {
    const { getByRole } = render(Sidebar);

    const expected: ReadonlyArray<{ label: string; href: string }> = [
      { label: 'Films', href: '/films' },
      { label: 'Séries', href: '/series' },
      { label: 'Animes', href: '/animes' },
      { label: 'Favoris', href: '/favoris' },
      { label: 'À regarder', href: '/a-regarder' },
      { label: 'Paramètres', href: '/settings' }
    ];

    for (const { label, href } of expected) {
      const link = getByRole('link', { name: label }) as HTMLAnchorElement;
      expect(link.getAttribute('href')).toBe(href);
    }
  });

  it('expose une nav accessible avec un aria-label explicite', () => {
    const { getByRole } = render(Sidebar);
    const nav = getByRole('navigation');
    expect(nav.getAttribute('aria-label')).toBe('Navigation principale');
  });
});

describe('<Sidebar> — état actif', () => {
  it('marque le lien correspondant à la route courante comme actif', () => {
    pageStore.set({ url: { pathname: '/favoris' } });

    const { getByRole } = render(Sidebar);
    const favoris = getByRole('link', { name: 'Favoris' });
    const films = getByRole('link', { name: 'Films' });

    expect(favoris.classList.contains('active')).toBe(true);
    expect(favoris.getAttribute('aria-current')).toBe('page');

    expect(films.classList.contains('active')).toBe(false);
    expect(films.getAttribute('aria-current')).toBeNull();
  });

  it('met à jour le lien actif quand le store `page` change', async () => {
    pageStore.set({ url: { pathname: '/films' } });

    const { getByRole } = render(Sidebar);
    expect(getByRole('link', { name: 'Films' }).classList.contains('active')).toBe(
      true
    );

    pageStore.set({ url: { pathname: '/settings' } });
    await tick();

    expect(getByRole('link', { name: 'Films' }).classList.contains('active')).toBe(
      false
    );
    expect(
      getByRole('link', { name: 'Paramètres' }).classList.contains('active')
    ).toBe(true);
  });

  it("n'active aucune entrée pour une route hors navigation principale (ex: /detail/42)", () => {
    pageStore.set({ url: { pathname: '/detail/42' } });

    const { container } = render(Sidebar);
    expect(container.querySelectorAll('a.active')).toHaveLength(0);
  });
});
