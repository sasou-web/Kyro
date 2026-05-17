// Kyro tourne en mode SPA strict dans la WebView Tauri (WebView2 sur Windows).
// Aucune route ne doit être rendue côté serveur ni pré-rendue : tout l'affichage
// est piloté côté client après hydratation initiale du fallback `index.html`.
//
// Voir tâche 1.3 du plan d'implémentation et Requirements 1.1, 1.2.

export const ssr = false;
export const prerender = false;
export const csr = true;
