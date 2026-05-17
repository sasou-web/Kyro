# Kyro

Bibliothèque média locale pour Windows. Films, séries, animes — tout reste sur ta machine.

## Pourquoi Kyro

- **100 % local, zéro requête réseau.** Pas de comptes, pas de télémétrie, pas de TMDB. Tes habitudes de visionnage ne quittent jamais ton PC.
- **Lecteur externe natif.** Délègue la lecture à [mpv.net](https://github.com/mpvnet-player/mpv.net), avec suivi automatique de la position et du statut (vu / en cours / non vu) via un script Lua qui parle à Kyro par named pipe Windows.
- **Léger.** ~19 Mio pour le binaire principal. WebView2 du système, pas de Chromium embarqué.
- **Robuste.** SQLite WAL, watcher filesystem, sauvegarde / restauration intégrées.

## Captures

Une grille minimaliste, une fiche détail riche, un lecteur natif. (Captures à ajouter.)

## Installation

Voir [`docs/INSTALL.md`](docs/INSTALL.md) pour la procédure complète et le warning SmartScreen.

Résumé express :

1. Télécharge `Kyro_1.0.0_x64-setup.exe` depuis les [Releases GitHub](https://github.com/sasou-web/Kyro/releases).
2. Exécute l'installeur. Au premier lancement de Kyro, Paramètres → « Installer le script Lua ».
3. Ajoute tes dossiers de bibliothèque (Films, Séries, Animes).

## Ce que sait faire Kyro

- Indexer des dossiers locaux contenant fichiers vidéo (mkv, mp4, avi, etc.).
- Détecter automatiquement les ajouts / suppressions / renames via watcher filesystem.
- Extraire les métadonnées techniques (durée, codec, résolution) via ffprobe.
- Parser les noms de fichiers pour deviner titre / année / saison / épisode.
- Importer une affiche locale par drag-and-drop.
- Lancer la lecture via mpv.net avec reprise exacte à la position précédente.
- Mettre à jour automatiquement le statut (`vu` à ~90 % de la durée).
- Suivre les transitions d'épisodes (autoplay, skip manuel, end-of-file).
- Recherche FTS5 instantanée, insensible à la casse et aux accents.
- Sauvegarder / restaurer la bibliothèque dans une archive ZIP.

## Ce que Kyro ne fait pas (et ne fera probablement pas)

- Streamer ou télécharger du contenu depuis Internet.
- Récupérer affiches / synopsis depuis TMDB / IMDB. Tout est édité manuellement (pour l'instant).
- Synchroniser entre plusieurs machines (sauf via export/import manuel).
- Lire en intégré (toujours déléguer à un lecteur externe).

## Architecture

- **Backend** : Rust + Tauri 2, SQLite via sqlx, runtime async tokio.
- **Frontend** : SvelteKit + Vite + TypeScript, mode SPA pure.
- **WebView** : Edge / WebView2 (Windows natif).
- **Tests** : 348 tests Rust + 220 tests frontend, plus tests d'intégration IPC, property-based tests via proptest et fast-check, bench search.

Le coeur Rust est découpé en :

```
src-tauri/src/
├── domain/     # logique pure (parser, status, resume, search normalize)
├── infra/      # I/O (sqlite, scanner, ffprobe, player_bridge, ipc_pipe, watcher, backup)
└── commands/   # commandes Tauri exposées au frontend
```

Le frontend en :

```
frontend/src/
├── routes/         # pages SvelteKit
├── lib/api/        # wrappers IPC vers les commandes Rust
├── lib/components/ # MediaCard, TopBar, Sidebar, FilterPanel, PosterPicker, etc.
└── lib/stores/     # stores Svelte (settings, library, theme, toast)
```

## Build local

Prérequis : [Rust stable](https://www.rust-lang.org/tools/install), Node.js LTS, et le SDK Windows pour les outils MSI/NSIS de Tauri.

```powershell
# Une fois pour récupérer les deps frontend
cd frontend ; npm install ; cd ..

# Build complet (release + bundles MSI/NSIS + sommes SHA-256)
pwsh -NoProfile -File scripts/build-windows.ps1
```

Les artefacts arrivent dans `dist/` :
- `Kyro_1.0.0_x64-setup.exe` (installeur NSIS)
- `Kyro_1.0.0_x64_fr-FR.msi` et `Kyro_1.0.0_x64_en-US.msi`
- `Kyro-portable.exe` (binaire seul)
- `SHA256SUMS.txt` (intégrité)

Pour signer les artefacts (si tu as un certificat Authenticode `.pfx`) :

```powershell
$env:KYRO_SIGN_CERT = 'C:\path\to\codesign.pfx'
$env:KYRO_SIGN_CERT_PASSWORD = '...'
pwsh -NoProfile -File scripts/build-windows.ps1
```

## Tests

```powershell
# Tests Rust (unit + intégration + property-based)
cd src-tauri ; cargo test --tests -- --test-threads=1 ; cd ..

# Bench recherche FTS5 (5000 fiches, exigé < 200 ms par requête)
cd src-tauri ; cargo test --test bench_search --release -- --ignored --nocapture ; cd ..

# Tests frontend
cd frontend ; npx vitest run ; cd ..
```

Le `--test-threads=1` évite un flake env-var pré-existant dans le test
`init_default_settings_persists_detected_mpv_net` (concurrence sur
`std::env::set_var`).

## Logs

`%LOCALAPPDATA%\app.kyro\logs\kyro.log.YYYY-MM-DD`. Format JSON ligne, parsable :

```powershell
Get-Content "$env:LOCALAPPDATA\app.kyro\logs\kyro.log.*" |
    ConvertFrom-Json |
    Where-Object { $_.level -eq 'ERROR' }
```

## License

À définir.
