/**
 * @vitest-environment jsdom
 *
 * Test de propriété frontend — Property 35 : Bascule de thème est une
 * involution.
 *
 * On vérifie que pour tout état initial (`adwaita-dark` ou `adwaita-light`)
 * exposé par `get_theme()`, exécuter `toggleTheme()` deux fois successivement
 * ramène l'application à son état initial — aussi bien sur l'attribut
 * `data-theme` du `<html>` (côté DOM) que sur la valeur persistée par
 * `set_theme()` (surrogate de `setting_value(theme)` côté backend).
 *
 * Méthodologie :
 *   - `$lib/api/commands` est intégralement mocké : `getTheme` renvoie la
 *     valeur arbitraire choisie par fast-check, `setTheme` enregistre la
 *     dernière valeur persistée dans une variable locale (le surrogate).
 *   - Avant chaque itération de `fc.asyncProperty`, on appelle
 *     `vi.resetModules()` puis on réimporte le store : cela garantit qu'on
 *     repart d'un état frais (la souscription DOM est réinstallée et la
 *     valeur initiale du writable est `'adwaita-dark'`).
 *   - On purge également l'attribut `data-theme` sur `<html>` pour ne pas
 *     hériter d'une itération précédente.
 *   - On appelle `loadTheme()` pour faire converger DOM + store vers la
 *     valeur initiale, on capture cet état, puis on enchaîne deux
 *     `toggleTheme()` et on vérifie le retour à l'état initial.
 *
 * **Validates: Requirements 16.14**
 */

import { afterEach, describe, expect, it, vi } from 'vitest';
import fc from 'fast-check';

// Variable locale alimentée par le mock de `setTheme` : sert de surrogate
// au `setting_value(theme)` persisté côté backend. Réinitialisée à chaque
// itération via `lastPersisted = null` avant le re-import.
let lastPersisted: string | null = null;

vi.mock('$lib/api/commands', () => ({
  getTheme: vi.fn(),
  setTheme: vi.fn(async (value: string) => {
    lastPersisted = value;
  })
}));

// On importe les mocks (et non le module réel) pour pouvoir reconfigurer
// le retour de `getTheme` à chaque itération.
import * as commands from '$lib/api/commands';

const getThemeMock = commands.getTheme as unknown as ReturnType<typeof vi.fn>;
const setThemeMock = commands.setTheme as unknown as ReturnType<typeof vi.fn>;

afterEach(() => {
  document.documentElement.removeAttribute('data-theme');
});

describe('Property 35 — Bascule de thème est une involution', () => {
  // Timeout étendu : chaque itération de `fc.asyncProperty` enchaîne
  // `vi.resetModules()` + un `await import('./theme')` + trois commandes
  // backend mockées. Avec `numRuns: 100`, le coût cumulé du re-import
  // dynamique sous jsdom dépasse le timeout par défaut (5000 ms) sur
  // certaines machines. 30 secondes laissent une marge confortable.
  it('Property 35 : pour tout état initial, toggleTheme ∘ toggleTheme = identité (data-theme et setting_value reviennent à l\'initial)', async () => {
    await fc.assert(
      fc.asyncProperty(
        fc.constantFrom<'adwaita-dark' | 'adwaita-light'>(
          'adwaita-dark',
          'adwaita-light'
        ),
        async (initialTheme) => {
          // 1. Reset complet : mocks, persistance surrogate, attribut DOM,
          //    et cache de modules pour que l'import suivant produise une
          //    instance fraîche du store (writable initial à 'adwaita-dark',
          //    nouvelle souscription DOM).
          getThemeMock.mockReset();
          setThemeMock.mockClear();
          // Le mock factory (cf. `vi.mock` plus haut) reste actif après
          // `mockClear`, mais l'implémentation est rétablie pour garantir
          // que `setTheme` continue d'écrire dans `lastPersisted`.
          setThemeMock.mockImplementation(async (value: string) => {
            lastPersisted = value;
          });
          lastPersisted = null;
          document.documentElement.removeAttribute('data-theme');
          vi.resetModules();

          // 2. Configurer `getTheme` pour renvoyer l'état initial tiré par
          //    fast-check, puis réimporter le store.
          getThemeMock.mockResolvedValueOnce(initialTheme);
          const { loadTheme, toggleTheme } = await import('./theme');

          // 3. Faire converger le store + DOM vers l'état initial. On
          //    capture les observables qui constituent « l'état initial »
          //    de la propriété : attribut `data-theme` et valeur persistée.
          await loadTheme();
          const initialDomTheme =
            document.documentElement.getAttribute('data-theme');
          // À ce stade, `loadTheme` n'a pas appelé `setTheme` ; le
          // surrogate reste `null`. Pour que la comparaison post-toggle×2
          // soit significative, on initialise le surrogate à la valeur
          // courante (équivalent d'une lecture de `setting_value(theme)`
          // côté backend).
          const initialPersisted = lastPersisted ?? initialTheme;

          // Sanity check : l'état initial doit refléter la valeur
          // demandée. Sinon, la propriété ne teste pas ce qu'elle prétend.
          expect(initialDomTheme).toBe(initialTheme);

          // 4. Appliquer la bascule deux fois successivement.
          await toggleTheme();
          await toggleTheme();

          // 5. Vérifier l'involution :
          //    - `data-theme` est revenu à la valeur initiale ;
          //    - la dernière valeur persistée via `setTheme` est elle aussi
          //      revenue à la valeur initiale (deux appels successifs :
          //      flip puis flip-back).
          expect(document.documentElement.getAttribute('data-theme')).toBe(
            initialDomTheme
          );
          expect(lastPersisted).toBe(initialPersisted);
          expect(setThemeMock).toHaveBeenCalledTimes(2);
          expect(setThemeMock).toHaveBeenLastCalledWith(initialTheme);
        }
      ),
      { numRuns: 100 }
    );
  }, 30_000);
});
