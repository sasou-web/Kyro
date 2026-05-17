# Changelog Kyro

Le format suit [Keep a Changelog](https://keepachangelog.com/) et le
versioning [SemVer](https://semver.org/).

## [1.0.0] — 2026-05-17

Première release publique. Toutes les fonctionnalités du périmètre
v1.0 sont opérationnelles.

### Ajouté

- **Bibliothèque média locale** complète : Films, Séries, Animes
  organisés en grille avec affiches, métadonnées, statuts.
- **Lecture mpv.net intégrée** : reprise exacte à la position via
  named pipe Windows + script Lua embarqué (`kyro.lua`).
- **Suivi automatique du visionnage** : la position est poussée toutes
  les 5 s pendant la lecture, le statut bascule en `Vu` à ~90 % de la
  durée, gestion correcte de l'autoplay (E1 → E2 → E3) avec
  basculement de cible IPC à chaque changement de fichier.
- **Watcher filesystem** : détection automatique des ajouts /
  suppressions / renames de fichiers vidéo via `notify`. Plus besoin
  de cliquer Rafraîchir après un copier-coller.
- **Recherche FTS5** : index SQLite virtuel avec tokenizer
  `unicode61 remove_diacritics 2`. Recherche insensible à la casse et
  aux accents en ~1 ms sur 5000 fiches.
- **Sauvegarde / restauration** : export ZIP de la base SQLite
  (`VACUUM INTO` cohérent) + dossier `posters/` + manifest. La
  restauration crée un filet de sécurité avant écrasement.
- **Édition manuelle** : titre, année, synopsis, studio, note, genres,
  affiche par drag-and-drop, statut forcé, favori, à regarder.
- **Reprise série** : « Reprendre » bascule sur le prochain épisode
  non vu, « Revoir depuis le début » remet toute la progression à zéro.
- **Bandeaux d'état dégradé** : ffprobe manquant (jaune), backend
  dégradé (rouge), scan en cours (bleu).
- **À propos** : section Paramètres avec version actuelle et lien
  vers les Releases GitHub (pas de mise à jour automatique côté Kyro
  pour préserver la garantie zéro réseau, le navigateur ouvre la
  page).
- **SHA-256** des artefacts dans `dist/SHA256SUMS.txt` à chaque build.
- **Signature Authenticode optionnelle** via `signtool` quand
  `KYRO_SIGN_CERT` est défini.
- **Documentation** : `README.md` + `docs/INSTALL.md` (procédure
  d'installation, contournement SmartScreen, dépannage).

### Garanties

- **Zéro réseau côté Kyro.** Aucune dépendance HTTP cliente directe
  dans `Cargo.toml` (`reqwest`, `ureq`, `surf`, `isahc`, `attohttpc`,
  `curl`, `hyper-tls`, `hyper-rustls` interdits). Vérifié par le test
  d'intégration `network_isolation.rs` (Property 13).
- **Local-first.** Aucune télémétrie, aucun compte, aucune
  synchronisation cloud. La base de données et les affiches restent
  dans `%APPDATA%\Kyro\`.
- **Cible Windows uniquement** (Tauri 2 + WebView2).

### Tests

- 348 tests automatisés au total :
  - 320 tests unitaires Rust (domaine pur, infrastructure, commandes).
  - 7 tests d'intégration IPC mpv (scénarios autoplay, completed,
    file-changed, closed régressif).
  - 2 tests d'intégration scan + lecture.
  - 10 tests Lua NDJSON (analyse statique du script).
  - 2 tests migrations (idempotence + reset).
  - 7 tests network_isolation (Property 13 + parser robustesse).
  - 220 tests frontend (composants Svelte + property-based via
    fast-check).
- **Bench search** : 1 ms par requête sur 5000 fiches en release
  (budget 200 ms).

### Architecture

- Backend Rust + Tauri 2.11.2 + sqlx 0.8 (SQLite WAL).
- Frontend SvelteKit 2.8 + Svelte 4 + Vite 5 + TypeScript 5.
- WebView2 du système (pas de Chromium embarqué). Binaire ~19 Mio.

### Limitations connues

- Pas de mise à jour automatique : il faut télécharger manuellement
  les nouvelles releases (par choix architectural, voir
  `commands/updates.rs`).
- Pas de signature Authenticode officielle. SmartScreen affichera un
  warning au premier lancement (procédure dans `docs/INSTALL.md`).
- Pas d'i18n complète : l'UI est codée en français, anglais partiel
  uniquement via les MSI multilingues.
- Le sidecar `ffprobe.exe` (~164 Mio) est livré à côté du binaire
  principal, pas embarqué. Indispensable pour extraire durée / codec /
  résolution.

[1.0.0]: https://github.com/sasou-web/Kyro/releases/tag/v1.0.0
