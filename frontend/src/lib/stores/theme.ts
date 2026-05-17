/**
 * Store du thème Kyro (Adwaita Dark / Adwaita Light).
 *
 * Le thème actif est exposé sous forme d'un store Svelte `Writable`
 * dont la valeur est appliquée à l'attribut `data-theme` de
 * `document.documentElement`. Les feuilles de style (cf.
 * `frontend/src/styles/theme.css`) sélectionnent les blocs
 * `:root[data-theme="adwaita-dark"]` ou `:root[data-theme="adwaita-light"]`
 * en fonction de la valeur courante.
 *
 * Cycle de vie attendu :
 *   1. Au boot du frontend, appeler `loadTheme()` (idéalement depuis
 *      `+layout.svelte`). Cette fonction interroge `get_theme()` côté
 *      backend, met à jour le store et applique le `data-theme` au DOM.
 *   2. Pour basculer (toggle), appeler `toggleTheme()` : la valeur est
 *      inversée entre `'adwaita-dark'` et `'adwaita-light'`, persistée
 *      via `set_theme()` puis appliquée au DOM.
 *   3. N'importe quelle écriture directe (`theme.set(...)`) déclenche
 *      la même mise à jour DOM via la souscription installée plus bas.
 *
 * Au tout premier démarrage (clé absente côté backend), `get_theme()`
 * renvoie `"adwaita-dark"` par défaut, ce qui satisfait le critère
 * d'acceptation 16.1.
 *
 * _Requirements: 16.1, 16.14_
 */

import { writable, type Writable } from 'svelte/store';

import { getTheme, setTheme } from '$lib/api/commands';

/** Identifiants des deux thèmes supportés. */
export type ThemeName = 'adwaita-dark' | 'adwaita-light';

/**
 * Store du thème courant. Valeur initiale `'adwaita-dark'` pour
 * couvrir la phase pré-`loadTheme()` (avant que la commande backend
 * n'ait répondu).
 */
export const theme: Writable<ThemeName> = writable<ThemeName>('adwaita-dark');

/**
 * Applique la valeur passée à l'attribut `data-theme` de la racine du
 * document. No-op si `document` est indisponible (e.g. SSR ou test
 * sans environnement DOM).
 */
function applyToDom(value: ThemeName): void {
  if (typeof document === 'undefined') return;
  document.documentElement.setAttribute('data-theme', value);
}

/**
 * Type-guard : ne laisse passer que les deux valeurs reconnues. Toute
 * autre chaîne (corruption du fichier de configuration, version
 * future) sera ramenée à `'adwaita-dark'` par le caller.
 */
function isThemeName(value: string): value is ThemeName {
  return value === 'adwaita-dark' || value === 'adwaita-light';
}

// Souscription persistante : toute écriture du store (depuis
// `loadTheme`, `toggleTheme` ou un éventuel `theme.set(...)` futur)
// pousse la valeur sur l'attribut `data-theme`.
theme.subscribe(applyToDom);

/**
 * Charge le thème enregistré côté backend et l'applique au DOM.
 *
 * À appeler une fois au boot. La commande Tauri `get_theme` renvoie
 * `"adwaita-dark"` par défaut quand aucune valeur n'a encore été
 * enregistrée (premier démarrage), conformément au requirement 16.1.
 */
export async function loadTheme(): Promise<void> {
  const value = await getTheme();
  const next: ThemeName = isThemeName(value) ? value : 'adwaita-dark';
  theme.set(next);
}

/**
 * Bascule entre `'adwaita-dark'` et `'adwaita-light'`, persiste la
 * nouvelle valeur via `set_theme` puis l'applique au DOM (via la
 * souscription installée plus haut).
 *
 * Si la persistance échoue, l'erreur est propagée au caller et le
 * store reflète tout de même la nouvelle valeur (état optimiste). Le
 * caller peut décider d'afficher un toast / de revenir en arrière.
 */
export async function toggleTheme(): Promise<void> {
  let current: ThemeName = 'adwaita-dark';
  // `update` est synchrone : on calcule la nouvelle valeur et on la
  // capture localement pour la persistance ultérieure.
  let next: ThemeName = 'adwaita-light';
  theme.update((value) => {
    current = value;
    next = value === 'adwaita-dark' ? 'adwaita-light' : 'adwaita-dark';
    return next;
  });
  void current; // current capturé pour debug éventuel.
  await setTheme(next);
}
