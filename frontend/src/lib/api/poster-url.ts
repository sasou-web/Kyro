/**
 * Helper pour convertir un chemin absolu local d'affiche en URL
 * consommable par la WebView Tauri.
 *
 * Le backend Kyro persiste les `poster_path` en chemins absolus
 * Windows (`C:\Users\…\AppData\Roaming\app.kyro\posters\media-<id>.<ext>`).
 * Pour qu'une `<img>` puisse les afficher, on doit passer par le
 * custom protocol `asset://` configuré dans `tauri.conf.json`
 * (`security.assetProtocol.scope`). Le SDK Tauri v2 expose
 * `convertFileSrc(path)` qui produit la bonne URL.
 *
 * Mais `convertFileSrc` accède à `window.__TAURI_INTERNALS__`, qui
 * n'existe pas dans l'environnement jsdom utilisé par Vitest. Pour
 * que les tests unitaires des composants qui consomment des
 * affiches passent en CI, ce wrapper :
 *
 *   - utilise `convertFileSrc` quand l'environnement Tauri est
 *     disponible (production ou dev runtime) ;
 *   - retombe sur une URL `asset://localhost/<path encodé>`
 *     équivalente sinon (tests Vitest), permettant aux assertions
 *     de vérifier la présence d'un `src` cohérent sans crasher.
 *
 * Cf. https://v2.tauri.app/reference/javascript/api/namespacecore/#convertfilesrc
 */

/**
 * Convertit un chemin absolu local vers une URL `asset://...`
 * consommable par `<img src>`. Retourne `null` si `path` est vide
 * ou nul.
 */
export function toAssetUrl(path: string | null | undefined): string | null {
  if (path == null || path.length === 0) {
    return null;
  }

  // Détection runtime du contexte Tauri : `window.__TAURI_INTERNALS__`
  // est posé par le runtime Tauri lui-même au boot de la WebView.
  // En jsdom (tests Vitest), cet objet n'existe pas — on retombe alors
  // sur un encodage manuel équivalent à ce que fait
  // `convertFileSrc` côté Tauri Windows :
  //
  //   path = "C:\\Users\\…\\posters\\media-1.jpg"
  //   → "http://asset.localhost/C%3A%5CUsers%5C%E2%80%A6%5Cposters%5Cmedia-1.jpg"
  //
  // C'est strictement la même forme que le runtime, donc la valeur
  // est utilisable par les assertions de test sans dépendre du SDK.
  const win = typeof window !== 'undefined' ? (window as unknown as Record<string, unknown>) : null;
  const internals = win?.__TAURI_INTERNALS__ as
    | { convertFileSrc?: (filePath: string, protocol?: string) => string }
    | undefined;
  if (internals?.convertFileSrc != null) {
    return internals.convertFileSrc(path, 'asset');
  }

  // Fallback de secours : encode le chemin et utilise le préfixe
  // standard `http://asset.localhost` employé par Tauri Windows.
  return `http://asset.localhost/${encodeURIComponent(path)}`;
}
