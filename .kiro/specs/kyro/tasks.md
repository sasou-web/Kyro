# Implementation Plan: Kyro

## Overview

Plan d'implémentation incrémental pour Kyro, application desktop Windows construite avec Tauri 2.x : Kyro_Backend en Rust (édition 2021, runtime tokio, SQLite via sqlx) et Kyro_Frontend en SvelteKit (mode SPA, `adapter-static`, `ssr=false`). L'ordre va du bootstrap projet vers la persistance, puis les modules de domaine purs (parser, status, resume, filters, search) accompagnés de leurs tests de propriétés, puis l'infrastructure (ffprobe, scanner, named pipe mpv, Lua), la couche commands Tauri, et enfin le frontend (stores, composants, routes) et le packaging Windows. Les 37 propriétés du design sont chacune une sous-tâche distincte placée près de son module d'implémentation. Les tâches optionnelles sont marquées d'un `*` ; les tâches non marquées doivent être implémentées.

Convention : chaque sous-tâche cite les exigences EARS qu'elle couvre via `_Requirements: X.Y_` et chaque test de propriété annonce explicitement son numéro `Property: P-N` ainsi que les clauses validées.

## Tasks

- [x] 1. Bootstrap projet Tauri 2 + SvelteKit SPA
  - [x] 1.1 Initialiser l'arborescence du workspace
    - Créer `src-tauri/` (crate Rust) et `frontend/` (projet SvelteKit) à la racine
    - Ajouter `tauri.conf.json` ciblé Windows uniquement, identifier `app.kyro`, fenêtre principale 1200x800, allowlist restreinte (fs limitée au data dir, dialog, shell:open désactivé)
    - Configurer Cargo.toml minimal avec édition 2021 et le crate `tauri` v2
    - _Requirements: 1.1, 1.2, 1.3_

  - [x] 1.2 Configurer les dépendances Rust backend
    - Ajouter dans `src-tauri/Cargo.toml` : `tauri` (v2, sans `default-features` réseau), `tokio` (rt-multi-thread, fs, net, process, sync), `sqlx` (sqlite, runtime-tokio-rustls, migrate, macros), `serde`, `serde_json`, `thiserror`, `anyhow`, `regex`, `walkdir`, `uuid`, `unicode-normalization`, `dirs`, `which`, `tracing`, `tracing-subscriber`, `tracing-appender`
    - Ajouter en `dev-dependencies` : `proptest`, `tempfile`, `assert_matches`, `tokio-test`
    - Aucune dépendance HTTP (interdit `reqwest`, `ureq`, `hyper-client`) — épingler les versions exactes
    - _Requirements: 6.4, 17.3_

  - [x] 1.3 Configurer le frontend SvelteKit en mode SPA
    - `frontend/package.json` : `@sveltejs/kit`, `@sveltejs/adapter-static`, `svelte`, `vite`, `@tauri-apps/api`, `typescript`
    - `frontend/svelte.config.js` : utiliser `adapter-static({ fallback: 'index.html' })` ; définir `kit.prerender = { entries: [] }`
    - Dans tous les `+layout.ts` racine : exporter `export const ssr = false; export const prerender = false; export const csr = true;`
    - Ajouter dev-dependencies : `vitest`, `fast-check`, `@testing-library/svelte`
    - _Requirements: 1.1, 1.2_

  - [x] 1.4 Implémenter les tokens du thème Adwaita Dark et clair
    - Créer `frontend/src/styles/theme.css` avec les blocs `:root[data-theme="adwaita-dark"]` et `:root[data-theme="adwaita-light"]` exactement tels que définis dans le design (toutes les variables `--bg-primary`, `--bg-secondary`, `--surface`, `--border`, `--text-primary`, `--text-secondary`, `--accent`, `--accent-hover`, `--radius-card`, `--radius-control`, `--font-family`, `--shadow-card`, échelle `--space-*`)
    - Ajouter règles globales `html, body`, `.card`, `button`, `input`, `select`, `button.primary`, `button.primary:hover`
    - Importer ce fichier dans `+layout.svelte` racine
    - _Requirements: 16.1, 16.2, 16.3, 16.4, 16.5, 16.6, 16.7, 16.8, 16.9, 16.10, 16.11, 16.12, 16.13_

- [x] 2. Persistance SQLite et migrations
  - [x] 2.1 Initialiser le pool SQLite et le logger
    - Créer `src-tauri/src/main.rs` qui résout `app_data_dir` (`%APPDATA%\Kyro\`) via `tauri::api::path::app_data_dir`
    - Initialiser `tracing` avec `RollingFileAppender::new(Rotation::DAILY, app_data_dir.join("logs"), "kyro.log")`, format JSON ligne, niveau `info` (override via `KYRO_LOG`)
    - Ouvrir `SqlitePool` sur `library.db` avec `journal_mode=WAL`, `synchronous=NORMAL`, `foreign_keys=ON`
    - En cas d'échec critique d'initialisation (création fenêtre, init DB) : logger l'erreur et terminer le processus avec un code de sortie ≠ 0
    - _Requirements: 1.4, 17.1, 18.2_

  - [x] 2.2 Écrire la migration `0001_init.sql`
    - Créer `src-tauri/migrations/0001_init.sql` reproduisant exactement le DDL du design : `library_folder`, `media_item`, `episode_item`, `genre`, `media_genre`, `audio_track`, `subtitle_track`, `setting_value`
    - Inclure tous les CHECK : `category IN ('Films','Series','Animes')`, `type IN ('Film','Series','Anime')`, `user_rating BETWEEN 0 AND 10`, `status IN ('non_vu','en_cours','vu')`, `favorite IN (0,1)`, `to_watch IN (0,1)`, `available IN (0,1)`, `numbering_unknown IN (0,1)`, `watch_progress_seconds >= 0`, contrainte XOR sur `audio_track`/`subtitle_track`
    - Créer les index : `idx_media_item_type`, `idx_media_item_status`, `idx_media_item_year`, `idx_media_item_title (COLLATE NOCASE)`, `idx_episode_series (series_id, season_number, episode_number)`, et clé unique `episode_item.file_path`
    - _Requirements: 17.1_

  - [x] 2.3 Exécuter les migrations au démarrage et définir `KyroError`
    - Wirer `sqlx::migrate!("./migrations").run(&pool)` au boot, après l'ouverture du pool
    - Créer `src-tauri/src/error.rs` avec l'enum `KyroError` (`Io`, `Db`, `Probe`, `Parse`, `Ipc`, `Config`, `Validation`) annoté `thiserror`, et son adaptateur `Into<tauri::InvokeError>` produisant un message FR localisé tout en conservant la cause technique dans les logs
    - _Requirements: 17.2, 18.1, 18.4_

  - [x] 2.4 Tests unitaires pour le runner de migrations
    - Vérifier qu'une DB neuve applique toutes les migrations et que tous les index attendus sont créés (`SELECT name FROM sqlite_master WHERE type='index'`)
    - _Requirements: 17.1, 17.2_

- [x] 3. Domaine pur : Kyro_Parser
  - [x] 3.1 Implémenter le pipeline de parsing
    - Créer `src-tauri/src/domain/parser.rs` avec `ParsedFilename { title, year, quality, episode }` et `EpisodeNumbering { season, episode, source }`
    - Implémenter dans l'ordre : `strip_extension`, `extract_quality` (regex `\b(480p|720p|1080p|1440p|2160p|4k)\b` insensible à la casse), `extract_year` (regex `\b(19[0-9]{2}|20[0-9]{2})\b`), `extract_sxxexx`, `extract_n_x_n`, `extract_loose_episode` (`ép\.?|ep\.?|episode|\-` suivi du numéro)
    - `clean_title` retire séparateurs `.`, `_`, `-` et les segments qualité/année/numérotation reconnus
    - Pour les patterns souples (`extract_loose_episode`), saison par défaut = 1
    - Marquer `numbering_unknown = true` quand catégorie ∈ {Series, Animes} et aucun pattern d'épisode reconnu
    - _Requirements: 4.1, 4.2, 4.3, 4.4, 4.5, 4.6, 4.7_

  - [x] 3.2 Test de propriété — round-trip parse/format
    - **Property 6 : Round-trip parse / format des noms de fichier**
    - Générateur : titre arbitraire (lettres + chiffres + espaces), année optionnelle ∈ [1900, 2099], qualité optionnelle, numérotation arbitraire (`SxxExx`, `NxN`, ou pattern souple) ; construire un nom canonique et vérifier que `parse(name)` rétablit titre/année/qualité/numérotation (saison = 1 par défaut pour patterns souples)
    - **Validates: Requirements 4.1, 4.2, 4.3, 4.4, 4.5, 4.6**
    - _Requirements: 4.1, 4.2, 4.3, 4.4, 4.5, 4.6_

  - [x] 3.3 Test de propriété — numérotation inconnue marquée pour Séries/Animes
    - **Property 7 : Marquage numérotation inconnue**
    - Générateur : nom de fichier sans aucun pattern d'épisode reconnu, catégorie ∈ {Series, Animes} ; vérifier que l'Episode_Item résultant a `numbering_unknown = true`
    - **Validates: Requirements 4.7**
    - _Requirements: 4.7_

- [x] 4. Domaine pur : statut et reprise
  - [x] 4.1 Implémenter `compute_status`
    - Créer `src-tauri/src/domain/status.rs` avec la fonction `compute_status(progress: u32, duration_seconds: u32, manual_override: Option<Status>) -> Status` exactement comme spécifiée dans le design (override prioritaire, ratio > 0.9 ⇒ Vu, ratio ∈ (0, 0.9] ⇒ EnCours, sinon NonVu, durée nulle ⇒ NonVu)
    - _Requirements: 8.1, 8.2, 8.3, 8.4_

  - [x] 4.2 Implémenter `next_resume`
    - Créer `src-tauri/src/domain/resume.rs` avec `next_resume(episodes: &[EpisodeItem]) -> ResumeAction` ; ne considérer que les épisodes `available`, trier par `(season_number, episode_number)`, retourner `RewatchFromStart(first)` si tous sont vus, sinon premier `EnCours` puis premier `NonVu` selon l'ordre
    - _Requirements: 12.1, 12.2, 12.3_

  - [x] 4.3 Test de propriété — calcul déterministe du statut
    - **Property 16 : Calcul déterministe du statut**
    - Générateur : couples (`progress`, `duration`) avec `duration > 0`, `0 ≤ progress ≤ duration`, sans override ; vérifier les trois branches du seuil 90 %
    - **Validates: Requirements 8.2, 8.3**
    - _Requirements: 8.2, 8.3_

  - [x] 4.4 Test de propriété — prévalence du statut manuel
    - **Property 17 : Prévalence du statut manuel**
    - Générateur : statut forcé arbitraire ∈ {Vu, NonVu}, progression arbitraire ; vérifier que `compute_status` renvoie l'override sans considérer la progression
    - **Validates: Requirements 8.4**
    - _Requirements: 8.4_

  - [x] 4.5 Test de propriété — calcul du prochain épisode
    - **Property 27 : Calcul du prochain épisode à reprendre**
    - Générateur : liste d'EpisodeItem arbitraire avec saison/épisode/statut variables ; vérifier les trois branches (RewatchFromStart si tous vus, premier EnCours sinon, premier NonVu sinon) et que l'ordre saison/épisode est respecté
    - **Validates: Requirements 12.1, 12.2, 12.3**
    - _Requirements: 12.1, 12.2, 12.3_

- [x] 5. Domaine pur : filtres et recherche
  - [x] 5.1 Implémenter les filtres
    - Créer `src-tauri/src/domain/filters.rs` exposant `apply_filters(items, Filters { genres, years, types, statuses })` avec union sur `genres` (intersection non vide) et intersection sur `years`, `types`, `statuses` (chaque dimension : si vide ⇒ pas de contrainte)
    - _Requirements: 14.1, 14.2, 14.3, 14.4, 14.5_

  - [x] 5.2 Implémenter la normalisation de recherche
    - Créer `src-tauri/src/domain/search.rs` avec `normalize(s: &str) -> String` = `s.nfd().filter(|c| !is_combining_mark(c)).collect::<String>().to_lowercase()` (via `unicode-normalization`)
    - Exposer `matches(item, query) = normalize(item.title).contains(normalize(query)) || normalize(item.original_title).contains(normalize(query))`
    - Si `query` est vide ⇒ retourner toujours `true`
    - _Requirements: 13.2, 13.4_

  - [x] 5.3 Test de propriété — recherche insensible casse/accents
    - **Property 28 : Recherche insensible à la casse et aux accents**
    - Générateur : titre arbitraire avec accents et casse mixte, requête sous-chaîne ; vérifier `matches ⇔ normalize(title).contains(normalize(query))`
    - **Validates: Requirements 13.2**
    - _Requirements: 13.2_

  - [x] 5.4 Test de propriété — recherche vide retourne tout
    - **Property 29 : Recherche vide retourne tout**
    - Générateur : bibliothèque arbitraire ; vérifier que `apply_search("", L) == L`
    - **Validates: Requirements 13.4**
    - _Requirements: 13.4_

  - [x] 5.5 Test de propriété — conjonction et union des filtres
    - **Property 30 : Conjonction et union des filtres**
    - Générateur : bibliothèque + ensemble `(genres, years, types, statuses)` arbitraires ; vérifier que l'item passe ssi `(genres = ∅ ∨ item.genres ∩ genres ≠ ∅) ∧ (years = ∅ ∨ item.year ∈ years) ∧ (types = ∅ ∨ item.type ∈ types) ∧ (statuses = ∅ ∨ item.status ∈ statuses)`
    - **Validates: Requirements 14.2, 14.3**
    - _Requirements: 14.2, 14.3_

  - [x] 5.6 Test de propriété — filtres vides = identité
    - **Property 31 : Filtres vides = identité**
    - Générateur : bibliothèque arbitraire ; tous filtres vides ⇒ résultat = bibliothèque complète et compteur visible = `card(L)`
    - **Validates: Requirements 14.4, 14.5**
    - _Requirements: 14.4, 14.5_

- [x] 6. ffprobe : localisation, sidecar et wrapper
  - [x] 6.1 Implémenter le résolveur de binaire ffprobe
    - Créer `src-tauri/src/infra/ffprobe_locator.rs` qui essaie dans l'ordre : 1) `tauri::api::process::Command::new_sidecar("ffprobe")`, 2) chemin `setting_value(key='ffprobe_path')`, 3) `which::which("ffprobe")` ; en cas d'échec, émettre l'événement Tauri `ffprobe-missing` et retourner `Err(KyroError::Config)`
    - _Requirements: 5.3, 5.4_

  - [x] 6.2 Implémenter le wrapper Probe
    - Créer `src-tauri/src/infra/probe.rs` exposant le trait `ProbeBackend` et l'implémentation réelle qui exécute `ffprobe -v error -print_format json -show_format -show_streams <path>` via `tokio::process::Command`
    - Désérialiser la sortie JSON via `serde_json` vers `TechnicalMetadata { duration_seconds, video_codec, width, height, audio_tracks, subtitle_tracks, size_bytes }` (taille issue de `std::fs::metadata`)
    - En cas d'exit ≠ 0 ou JSON invalide, retourner `Err(ProbeError)` ; le scanner persistera l'item avec champs techniques `null`
    - _Requirements: 5.1, 5.2, 5.5, 5.6_

  - [x] 6.3 Configurer le bundling sidecar de ffprobe
    - Dans `tauri.conf.json` : `bundle.externalBin = ["binaries/ffprobe"]` ; placer `src-tauri/binaries/ffprobe-x86_64-pc-windows-msvc.exe` (build LGPL minimal documenté dans le README de la tâche)
    - Documenter dans le README `.kiro/specs/kyro/` que la taille du sidecar (~25 Mo) est exclue du seuil 50 Mo (Requirement 1.3 « hors dépendance ffprobe »)
    - _Requirements: 1.3, 5.3_

  - [x] 6.4 Tests unitaires pour le parsing ffprobe
    - Fixtures JSON ffprobe (mkv h264 + AAC, mp4 multi-audio, fichier corrompu) ; vérifier les champs typés et que la branche échec laisse `TechnicalMetadata = None`
    - _Requirements: 5.1, 5.5_

  - [x] 6.5 Test de propriété — mapping ffprobe → métadonnées
    - **Property 9 : Mapping ffprobe → métadonnées techniques**
    - Générateur : sortie JSON valide synthétique (durée, codec, résolution, listes audio/sub) ; vérifier qu'après mapping les champs typés correspondent exactement
    - **Validates: Requirements 5.1, 5.6**
    - _Requirements: 5.1, 5.6_

  - [x] 6.6 Test de propriété — robustesse en cas d'échec probe
    - **Property 10 : Robustesse du probe en cas d'échec**
    - Générateur : conditions d'échec variées (exit ≠ 0, JSON invalide, binaire absent) ; vérifier que l'entrée existe avec champs techniques `null`, qu'un log contient le chemin du fichier, et que le scan continue
    - **Validates: Requirements 5.5, 18.1**
    - _Requirements: 5.5, 18.1_

- [x] 7. Repository (sqlx)
  - [x] 7.1 CRUD library_folder avec validation
    - Créer `src-tauri/src/infra/repo.rs` avec `add_library_folder(path, category)` qui vérifie `std::fs::metadata(path)?.is_dir()` avant d'insérer ; sinon retour `KyroError::Validation` avec message identifiant le chemin
    - Implémenter `remove_library_folder(id)` qui dans une transaction met `available = 0` sur tous les `media_item.folder_id = id` et tous les `episode_item` associés, puis supprime la ligne `library_folder`
    - Implémenter `list_library_folders()`
    - _Requirements: 2.1, 2.2, 2.3, 2.4, 2.5_

  - [x] 7.2 Upsert media_item et episode_item
    - Implémenter `upsert_media_item(MediaItem)` et `upsert_episode_item(EpisodeItem)` ; clé d'unicité = `file_path` ; si l'enregistrement existe avec même `(file_path, size_bytes)`, ne rien modifier ; sinon mettre à jour les champs techniques sans toucher aux champs édités manuellement (sauf override explicite)
    - Persister `size_bytes` depuis `std::fs::metadata`
    - _Requirements: 3.3, 3.4, 5.2_

  - [x] 7.3 set_watch_progress et calcul du statut
    - Implémenter `set_watch_progress(target: ProgressTarget, position: u32)` qui ouvre une transaction, écrit `watch_progress_seconds = position`, recalcule `status` via `compute_status(position, duration, None)`, commit ; signaler succès uniquement après commit
    - Implémenter `force_status(target, status)` qui écrit le statut manuel ; toute mise à jour ultérieure de progression repassera par `compute_status` sans override
    - _Requirements: 8.1, 8.2, 8.3, 8.4, 17.2_

  - [x] 7.4 Listings et requêtes secondaires
    - Implémenter `list_by_category(Category)`, `search(&str)` (utilise `search::matches` côté Rust), `list_favorites()`, `list_to_watch()`, `list_episodes(series_id)` triés `(season_number ASC, episode_number ASC)`, `count_episodes_by_status(series_id)`, et `next_resume_episode(series_id)` qui appelle `domain::resume::next_resume` après avoir chargé les épisodes disponibles
    - _Requirements: 7.2, 7.3, 7.4, 8.5, 11.4, 11.5, 12.1_

  - [x] 7.5 Settings key/value
    - Implémenter `upsert_setting(key, value)` et `get_setting(key)` sur `setting_value` ; clés réservées : `external_player_path`, `ffprobe_path`, `theme`, `scan_at_startup`, `last_category`
    - _Requirements: 2.1, 7.1, 10.1, 10.2, 16.1, 16.14_

  - [x] 7.6 Toggles favori et à_regarder
    - Implémenter `toggle_favorite(media_item_id)` et `toggle_to_watch(media_item_id)` qui basculent l'indicateur ; deux toggles successifs doivent ramener à l'état initial (involution garantie par construction)
    - _Requirements: 11.1, 11.2, 11.3_

  - [x] 7.7 Import d'affiche locale
    - Implémenter `import_poster(target, source_path)` qui copie l'image dans `%APPDATA%\Kyro\posters\<id>.<ext>` et met à jour `poster_path` ; valider l'extension ∈ {jpg, jpeg, png, webp}
    - _Requirements: 6.2, 6.3, 17.1_

  - [x] 7.8 Test de propriété — round-trip ajout/lecture dossier
    - **Property 1 : Round-trip d'ajout / lecture d'un dossier de bibliothèque**
    - Générateur : chemin valide (créé dans `tempfile`) + catégorie ∈ {Films, Series, Animes} ; vérifier que `list_library_folders` contient exactement une entrée correspondant au chemin et à la catégorie
    - **Validates: Requirements 2.2, 2.5**
    - _Requirements: 2.2, 2.5_

  - [x] 7.9 Test de propriété — retrait dossier marque indisponibles
    - **Property 2 : Retrait d'un dossier marque tous ses items indisponibles**
    - Générateur : dossier contenant `n ≥ 0` Media_Item / Episode_Item ; vérifier qu'après `remove_library_folder` tous ont `available = false` et qu'aucun n'est supprimé
    - **Validates: Requirements 2.3**
    - _Requirements: 2.3_

  - [x] 7.10 Test de propriété — persistance fidèle de la taille
    - **Property 8 : Persistance fidèle de la taille de fichier**
    - Générateur : fichiers temporaires de tailles arbitraires ; vérifier `repo.size_bytes == fs::metadata(path).len()`
    - **Validates: Requirements 5.2**
    - _Requirements: 5.2_

  - [x] 7.11 Test de propriété — round-trip d'édition
    - **Property 11 : Round-trip d'édition d'une fiche**
    - Générateur : fiche valide (titre non vide, année ∈ [1900, 2099] ou nulle, note ∈ [0, 10] ou nulle, etc.) ; `update(fiche)` puis `read(id)` ⇒ même fiche
    - **Validates: Requirements 6.1, 6.3, 17.2**
    - _Requirements: 6.1, 6.3, 17.2_

  - [x] 7.12 Test de propriété — validation note utilisateur
    - **Property 12 : Validation de la note utilisateur**
    - Générateur : note hors [0, 10] ; vérifier que `update` est rejeté, fiche persistée non modifiée, message d'erreur retourné
    - **Validates: Requirements 6.6**
    - _Requirements: 6.6_

  - [x] 7.13 Test de propriété — filtrage par catégorie
    - **Property 14 : Filtrage par catégorie**
    - Générateur : bibliothèque mixte arbitraire ; vérifier que `list_by_category(c)` ⇔ `{m | m.type = c}`
    - **Validates: Requirements 7.2**
    - _Requirements: 7.2_

  - [x] 7.14 Test de propriété — tri saison/épisode
    - **Property 15 : Tri saisons puis épisodes**
    - Générateur : `EpisodeItem` arbitraires associés à une série ; vérifier que `list_episodes(series_id)` est triée par `(season_number ASC, episode_number ASC)`
    - **Validates: Requirements 7.3, 7.4**
    - _Requirements: 7.3, 7.4_

  - [x] 7.15 Test de propriété — compteur épisodes vus
    - **Property 18 : Compteur épisodes vus**
    - Générateur : série + épisodes avec statuts arbitraires ; vérifier `count_watched(series) = card({e | e.status = vu})` et `total = card(episodes)`
    - **Validates: Requirements 8.5**
    - _Requirements: 8.5_

  - [x] 7.16 Test de propriété — toggles favori/à_regarder involutions
    - **Property 25 : Toggle favori et toggle à_regarder sont des involutions**
    - Générateur : Media_Item arbitraire ; deux `toggle_favorite` successifs ⇒ état inchangé, idem pour `toggle_to_watch`
    - **Validates: Requirements 11.2, 11.3**
    - _Requirements: 11.2, 11.3_

  - [x] 7.17 Test de propriété — vues favoris et à_regarder
    - **Property 26 : Vue favoris et vue à_regarder**
    - Générateur : bibliothèque arbitraire ; vérifier que `list_favorites() ⇔ {m | m.favori = true}` et idem pour `list_to_watch()`
    - **Validates: Requirements 11.4, 11.5**
    - _Requirements: 11.4, 11.5_

  - [x] 7.18 Test de propriété — persistance avant signal de succès
    - **Property 36 : Persistance avant signal de succès**
    - Générateur : mutations arbitraires ; après que la commande renvoie `Ok`, lecture immédiate ⇒ nouvelle valeur (write-then-read cohérent)
    - **Validates: Requirements 17.2**
    - _Requirements: 17.2_

- [x] 8. Kyro_Scanner
  - [x] 8.1 Walkdir async avec filtre d'extensions
    - Créer `src-tauri/src/infra/scanner.rs` avec la constante `SUPPORTED_EXTENSIONS = ["mkv", "mp4", "avi", "mov", "webm", "m4v", "ts", "wmv"]`
    - Parcourir récursivement chaque `library_folder` via `walkdir::WalkDir` (suivi de liens désactivé), filtrage extension insensible à la casse
    - _Requirements: 3.1, 3.2_

  - [x] 8.2 Diff existants / nouveaux / disparus
    - Pour chaque fichier candidat : si `(file_path, size_bytes)` correspond à une entrée existante ⇒ skip ; sinon insérer en passant par `parser::parse` puis `probe::probe`, classifier selon la `category` du dossier parent, persister via `repo`
    - Marquer comme `available = 0` toute entrée précédemment indexée dont le fichier n'est plus présent au moment du scan, sans la supprimer
    - Si catégorie ∈ {Series, Animes} et aucun motif d'épisode reconnu, marquer `numbering_unknown = 1` et associer au pseudo-dossier `À classer`
    - _Requirements: 3.3, 3.4, 3.5, 4.7_

  - [x] 8.3 Émettre l'événement `scan-progress`
    - Émettre via `tauri::Window::emit("scan-progress", ScanProgress { processed, total, current_file })` à chaque fichier traité
    - _Requirements: 3.6_

  - [x] 8.4 Robustesse aux erreurs isolées
    - Toute erreur de parse/probe/IO sur un fichier est journalisée via `tracing::warn!(file_path, error)` ; le scan continue avec les fichiers suivants
    - Retourner un `ScanReport { total_files, processed, created, updated, marked_unavailable, failures: Vec<ScanFailure> }` à la fin
    - _Requirements: 18.1_

  - [x] 8.5 Hook `scan_at_startup`
    - Au boot, après init DB, lire `setting_value(key='scan_at_startup')` ; si truthy, lancer un scan asynchrone non-bloquant
    - _Requirements: 3.7_

  - [x] 8.6 Test de propriété — idempotence du scan
    - **Property 3 : Idempotence du scan**
    - Générateur : arborescence `tempfile` arbitraire de fichiers vidéo ; deux exécutions successives de `scan_all` ⇒ aucune nouvelle entrée et aucune modification au second passage
    - **Validates: Requirements 3.3, 3.4**
    - _Requirements: 3.3, 3.4_

  - [x] 8.7 Test de propriété — filtrage par extensions supportées
    - **Property 4 : Filtrage par extensions supportées**
    - Générateur : ensemble de fichiers avec extensions arbitraires ; vérifier que les fichiers retenus = sous-ensemble dont extension (insensible à la casse) ∈ `SUPPORTED_EXTENSIONS`
    - **Validates: Requirements 3.2**
    - _Requirements: 3.2_

  - [x] 8.8 Test de propriété — détection de fichiers disparus
    - **Property 5 : Détection de fichiers disparus**
    - Générateur : entrées indexées dont les fichiers sont ensuite supprimés ; vérifier qu'après scan elles existent encore avec `available = false`
    - **Validates: Requirements 3.5**
    - _Requirements: 3.5_

  - [x] 8.9 Test de propriété — continuation en cas d'erreur isolée
    - **Property 37 : Continuation du scan en cas d'erreur isolée**
    - Générateur : liste de fichiers contenant des erreurs (parse, probe, IO) entrelacées avec des fichiers valides ; vérifier que les fichiers valides sont traités, les échecs journalisés avec leur chemin, et que `ScanReport.failures` les liste
    - **Validates: Requirements 18.1**
    - _Requirements: 18.1_

- [x] 9. Checkpoint - Vérifier que tous les tests passent
  - Ensure all tests pass, ask the user if questions arise.

- [x] 10. Kyro_Player_Bridge et Kyro_IPC_Pipe
  - [x] 10.1 Génération du nom de pipe et état de session
    - Créer `src-tauri/src/infra/player_bridge.rs` ; à chaque `play_media`, générer `session_id = Uuid::new_v4()` et le nom de pipe `\\.\pipe\kyro-<session_id>`
    - _Requirements: 9.6_

  - [x] 10.2 Serveur named pipe + lecture NDJSON
    - Créer `src-tauri/src/infra/ipc_pipe.rs` qui expose `listen(pipe_name)` retournant un `mpsc::Receiver<PipeMessage>` à partir d'un `tokio::net::windows::named_pipe::NamedPipeServer` ; lecture ligne-orientée UTF-8, désérialisation `PipeMessage` (`progress`, `paused`, `resumed`, `closed`)
    - Si une ligne est invalide, journaliser et continuer
    - _Requirements: 9.2, 9.3_

  - [x] 10.3 Lancer mpv.net avec la position de reprise
    - Quand `External_Player` se résout en `mpv.net.exe` (détection insensible à la casse sur le nom de fichier), lancer via `tokio::process::Command::new(player_path).arg(file_path).arg(format!("--start={}", progress)).arg(format!("--script-opts=kyro-pipe=kyro-{}", session_id)).spawn()`
    - _Requirements: 9.1, 12.4_

  - [x] 10.4 Branche External_Player ≠ mpv.net
    - Si la détection ne reconnaît pas `mpv.net.exe`, lancer le lecteur avec uniquement `arg(file_path)`, ne pas créer de pipe, et émettre l'événement `external-player-non-mpv` ; exposer dans la fiche du Media_Item un badge « reprise exacte non garantie »
    - _Requirements: 10.5_

  - [x] 10.5 Validation existence et exécutabilité
    - Vérifier au moment du `set_external_player_path` et au moment du `play_media` que `std::fs::metadata(path)?.is_file()` et que le chemin se termine par `.exe` (insensible à la casse) ; sinon, refuser l'enregistrement (KyroError::Validation) ou émettre `external-player-missing` au lancement
    - _Requirements: 10.3, 10.4_

  - [x] 10.6 Persister la progression et la position finale
    - Écouter le receiver retourné par `ipc_pipe::listen` ; sur chaque `Progress` ou `Paused/Resumed`, appeler `repo.set_watch_progress(target, position as u32)` ; sur `Closed`, persister la position finale puis fermer le pipe et émettre `playback-ended`
    - Si aucun message n'est reçu pendant la session (pipe vide jusqu'à fermeture du processus joueur), ne pas modifier `watch_progress_seconds`
    - _Requirements: 9.3, 9.4, 9.5_

  - [x] 10.7 Test de propriété — lancement avec position de reprise
    - **Property 19 : Lancement avec position de reprise**
    - Générateur : Media_Item avec `watch_progress_seconds = p` arbitraire ; vérifier que la commande lancée contient `--start=p` (intercepté via un `Command` mock)
    - **Validates: Requirements 9.1, 12.4**
    - _Requirements: 9.1, 12.4_

  - [x] 10.8 Test de propriété — mise à jour finale via IPC
    - **Property 20 : Mise à jour de Watch_Progress sur message IPC**
    - Générateur : séquence NDJSON arbitraire `{type, position}` envoyée sur un mock pipe ; vérifier qu'après fermeture, `item.watch_progress_seconds = dernière position valide`
    - **Validates: Requirements 9.3, 9.4**
    - _Requirements: 9.3, 9.4_

  - [x] 10.9 Test de propriété — préservation sans message IPC
    - **Property 21 : Préservation en l'absence de message IPC**
    - Générateur : session ouverte sans aucun message reçu avant fermeture ; vérifier que `watch_progress_seconds` est inchangé
    - **Validates: Requirements 9.5**
    - _Requirements: 9.5_

  - [x] 10.10 Test de propriété — unicité du nom de pipe
    - **Property 22 : Unicité du nom de pipe par session**
    - Générateur : 2 lancements consécutifs ; vérifier que les noms de pipe diffèrent (UUID v4)
    - **Validates: Requirements 9.6**
    - _Requirements: 9.6_

  - [x] 10.11 Test de propriété — pas d'IPC pour lecteur non mpv
    - **Property 23 : Pas d'IPC pour lecteur autre que mpv.net**
    - Générateur : chemin External_Player aléatoire ne se résolvant pas en `mpv.net.exe` ; vérifier qu'aucun NamedPipeServer n'est créé et que l'item est annoté « reprise non garantie »
    - **Validates: Requirements 10.5**
    - _Requirements: 10.5_

  - [x] 10.12 Test de propriété — validation chemin External_Player
    - **Property 24 : Validation du chemin External_Player**
    - Générateur : chemins valides et invalides (inexistants, non `.exe`) ; vérifier que la persistance se fait ssi le fichier existe et est `.exe`
    - **Validates: Requirements 10.3**
    - _Requirements: 10.3_

- [x] 11. Kyro_Lua_Script et installation
  - [x] 11.1 Écrire `kyro.lua`
    - Créer `src-tauri/resources/kyro.lua` exactement comme le pseudocode du design : timer 5 s `progress`, événements `pause/unpause/shutdown`, format NDJSON `{"type":..,"position":..,"duration":..,"session":..}` séparé par `\n`, ouverture en mode `wb` sur `\\\\.\\pipe\\<pipe_name>`
    - _Requirements: 9.2, 9.3_

  - [x] 11.2 Commande Tauri `install_lua_script`
    - Créer `src-tauri/src/commands/playback.rs` avec `install_lua_script()` qui copie `resources/kyro.lua` vers `%APPDATA%\mpv.net\scripts\kyro.lua` ; créer le dossier si manquant, écraser si déjà présent ; retourner le chemin écrit
    - _Requirements: 9.7_

  - [x] 11.3 Tests Busted du format NDJSON
    - Vérifier via une suite Lua minimale que la fonction `send` produit bien des lignes NDJSON conformes au schéma `PipeMessage` côté Rust
    - _Requirements: 9.2, 9.3_

- [x] 12. Surface Tauri Commands
  - [x] 12.1 Commands library
    - Créer `src-tauri/src/commands/library.rs` avec : `list_by_category`, `get_media_item`, `list_episodes`, `update_media_item`, `update_episode_item`, `toggle_favorite`, `toggle_to_watch`, `force_status`, `set_watch_progress`, `import_poster_for_media`, `import_poster_for_episode`, `list_favorites`, `list_to_watch`, `next_resume_episode`, `search`
    - Toutes les commandes valident les entrées (note ∈ [0,10], année ∈ [1900,2099] ou null)
    - _Requirements: 6.1, 6.2, 6.3, 6.5, 6.6, 7.2, 7.4, 8.4, 8.5, 11.2, 11.3, 11.4, 11.5, 12.2, 13.2_

  - [x] 12.2 Commands scan
    - Créer `src-tauri/src/commands/scan.rs` avec `start_scan` (non bloquant, spawn `tokio::task`) et `scan_status` ; relayer les événements `scan-progress` et `scan-completed`
    - _Requirements: 3.1, 3.6_

  - [x] 12.3 Commands playback
    - Compléter `src-tauri/src/commands/playback.rs` avec `play_media(target_id, kind)` qui charge la progression, lance le player_bridge, et `play_resume(series_id)` qui appelle `next_resume_episode` puis `play_media`
    - Ajouter `play_rewatch_from_start(series_id)` qui remet à 0 la progression de tous les épisodes avant de jouer le premier
    - _Requirements: 9.1, 12.2, 12.3, 12.4_

  - [x] 12.4 Commands settings
    - Créer `src-tauri/src/commands/settings.rs` avec : `add_library_folder`, `remove_library_folder`, `list_library_folders`, `set_external_player_path`, `get_external_player_path`, `set_ffprobe_path`, `set_scan_at_startup`, `get_settings_summary`, `set_theme`, `get_theme`, `set_last_category`, `get_last_category`
    - Initialiser `external_player_path` à mpv.net par défaut au premier démarrage si la clé n'existe pas
    - _Requirements: 2.1, 2.2, 2.3, 2.4, 2.5, 5.4, 7.1, 10.1, 10.2, 10.3, 10.4, 16.1, 16.14_

  - [x] 12.5 Command logs
    - Créer `src-tauri/src/commands/logs.rs` avec `read_latest_log()` qui retourne le contenu du fichier le plus récent dans `%APPDATA%\Kyro\logs\` (lecture seule, max 5 Mo)
    - _Requirements: 18.3_

  - [x] 12.6 Mode dégradé et émission d'événements globaux
    - Dans le wrapper d'erreur des commandes : sur erreur irrécupérable du backend (ex. DB verrouillée plusieurs essais), émettre `backend-degraded` ; émettre `ffprobe-missing` au démarrage si le résolveur échoue
    - _Requirements: 5.4, 18.4_

- [x] 13. Frontend : invokes typés, stores, thème
  - [x] 13.1 Wrappers TypeScript pour invoke
    - Créer `frontend/src/lib/api/commands.ts` avec un wrapper typé pour chaque commande Tauri ; définir `frontend/src/lib/api/types.ts` mirroir des DTO Rust (`MediaItem`, `EpisodeItem`, `LibraryFolder`, `ScanProgress`, `ScanReport`, `Filters`, `Status`, `Category`, etc.)
    - _Requirements: 1.1_

  - [x] 13.2 Stores Svelte de bibliothèque
    - Créer `frontend/src/lib/stores/library.ts` avec : `items: Writable<MediaItem[]>`, `episodes: Writable<Map<MediaItemId, EpisodeItem[]>>`, `filters: Writable<Filters>`, `search: Writable<string>`, `settings: Writable<Settings>`, `scanProgress: Writable<ScanProgress | null>`, `flags: Writable<{ ffprobeMissing: boolean; backendDegraded: boolean }>`
    - Brancher `listen("scan-progress" | "scan-completed" | "ffprobe-missing" | "backend-degraded" | "external-player-missing" | "playback-ended")` dans un module `frontend/src/lib/api/events.ts` initialisé au layout racine
    - _Requirements: 3.6, 5.4, 18.4_

  - [x] 13.3 Store dérivé filteredItems
    - Créer `frontend/src/lib/stores/filtered.ts` exposant `filteredItems = derived([items, filters, search], applyFiltersAndSearch)` ; cible : moins de 50 ms par re-filtre pour 5000 items (utiliser `Map<genreId, Set<itemId>>` précomputé sur changement de `items`)
    - _Requirements: 13.3, 14.2, 14.3, 14.4, 14.5_

  - [x] 13.4 Système de thème
    - Créer `frontend/src/lib/stores/theme.ts` ; au boot, lire `get_theme()` ; appliquer `document.documentElement.setAttribute('data-theme', value)` ; sur `toggle_theme`, basculer entre `adwaita-dark` et `adwaita-light`, persister via `set_theme`
    - Au tout premier démarrage (clé absente), valeur initiale = `adwaita-dark`
    - _Requirements: 16.1, 16.14_

  - [x] 13.5 Test de propriété frontend — tokens du thème
    - **Property 34 : Tokens du thème Adwaita Dark**
    - Avec `data-theme="adwaita-dark"` appliqué, vérifier que `getComputedStyle(html).getPropertyValue('--bg-primary').trim() === '#1e1e2e'` et identiquement pour les 11 autres tokens du Requirement 16, puis vérifier que `--accent` parsé (HSL) reste dans la gamme bleue (teinte ∈ [200°, 240°])
    - **Validates: Requirements 16.2, 16.3, 16.4, 16.5, 16.6, 16.7, 16.8, 16.9, 16.10, 16.11, 16.12, 16.13**
    - _Requirements: 16.2, 16.3, 16.4, 16.5, 16.6, 16.7, 16.8, 16.9, 16.10, 16.11, 16.12, 16.13_

  - [x] 13.6 Test de propriété frontend — bascule de thème involution
    - **Property 35 : Bascule de thème est une involution**
    - Générateur fast-check : état initial arbitraire ; exécuter `toggle_theme` deux fois ⇒ `data-theme` et `setting_value(theme)` reviennent à l'état initial
    - **Validates: Requirements 16.14**
    - _Requirements: 16.14_

- [x] 14. Frontend : composants Svelte
  - [x] 14.1 MediaCard
    - Créer `frontend/src/lib/components/MediaCard.svelte` ; afficher l'affiche locale via `tauri://localhost/posters/<id>.<ext>`, ou un visuel par défaut SVG intégré si `poster_path` absent ; afficher titre et année (si définie)
    - _Requirements: 15.1, 15.2_

  - [x] 14.2 HoverOverlay
    - Créer `frontend/src/lib/components/HoverOverlay.svelte` ; au survol d'une MediaCard, afficher la note utilisateur, la durée totale (Films) ou le nombre d'épisodes (Séries/Animes), et le statut courant
    - _Requirements: 15.4_

  - [x] 14.3 FilterPanel et FilterChip
    - Créer `frontend/src/lib/components/FilterChip.svelte` et `frontend/src/lib/components/FilterPanel.svelte` ; permettre la sélection multiple sur genre (union), unique sur année/type/statut (intersection) ; afficher en permanence le nombre de Media_Item visibles après filtrage
    - _Requirements: 14.1, 14.2, 14.3, 14.5_

  - [x] 14.4 SearchBar
    - Créer `frontend/src/lib/components/SearchBar.svelte` accessible depuis toutes les vues principales (placée dans le TopBar) ; debounced 50 ms ; lie `search` store
    - _Requirements: 13.1, 13.2, 13.3, 13.4_

  - [x] 14.5 Sidebar et TopBar
    - Créer `frontend/src/lib/components/Sidebar.svelte` (navigation Films / Séries / Animes / Favoris / À regarder / Paramètres) et `TopBar.svelte` (SearchBar + indicateur de scan + badge ffprobe-missing + badge backend-degraded)
    - Appliquer Adwaita Dark via les CSS variables existantes
    - _Requirements: 7.1, 11.4, 11.5, 13.1, 18.4_

  - [x] 14.6 RatingInput
    - Créer `frontend/src/lib/components/RatingInput.svelte` qui rejette toute valeur hors [0, 10] avec un message d'erreur explicite indiquant la plage autorisée
    - _Requirements: 6.6_

  - [x] 14.7 PosterPicker
    - Créer `frontend/src/lib/components/PosterPicker.svelte` qui ouvre `@tauri-apps/api/dialog::open` avec filtre `image/*`, puis appelle `import_poster_for_media` ou `import_poster_for_episode`
    - _Requirements: 6.2, 6.3_

  - [x] 14.8 Test de propriété frontend — contenu de la carte
    - **Property 32 : Carte d'item contient affiche / titre / année**
    - Générateur fast-check `arbMediaItem` ; rendre `<MediaCard>` via `@testing-library/svelte` ; vérifier que le DOM expose (a) une `<img>` (poster ou visuel par défaut), (b) le titre, (c) l'année si définie
    - **Validates: Requirements 15.1, 15.2**
    - _Requirements: 15.1, 15.2_

  - [x] 14.9 Test de propriété frontend — overlay au survol
    - **Property 33 : Overlay au survol**
    - Générateur fast-check : Media_Item arbitraire ; déclencher `mouseenter` ; vérifier que l'overlay contient note, durée ou nombre d'épisodes, et statut
    - **Validates: Requirements 15.4**
    - _Requirements: 15.4_

- [x] 15. Frontend : routes et pages SvelteKit
  - [x] 15.1 Layout racine et bandeaux globaux
    - Créer `frontend/src/routes/+layout.svelte` important `theme.css`, montant `Sidebar` + `TopBar`, observant `flags.ffprobeMissing`, `flags.backendDegraded`, `scanProgress` ; afficher les bandeaux correspondants ; au mount, appeler `init_settings()` et `setup_event_listeners()`
    - _Requirements: 5.4, 18.4, 16.1_

  - [x] 15.2 Page d'accueil et grilles catégorie
    - Créer `frontend/src/routes/+page.svelte` qui redirige vers `last_category` (par défaut `/films`)
    - Créer `frontend/src/routes/films/+page.svelte`, `frontend/src/routes/series/+page.svelte`, `frontend/src/routes/animes/+page.svelte` rendant la grille adaptive de `MediaCard` (CSS `grid-template-columns: repeat(auto-fill, minmax(180px, 1fr))`)
    - _Requirements: 7.1, 7.2, 15.1, 15.3_

  - [x] 15.3 Pages Favoris et À regarder
    - Créer `frontend/src/routes/favoris/+page.svelte` (liste filtrée `favori = true` via `list_favorites`) et `frontend/src/routes/a-regarder/+page.svelte` (`list_to_watch`)
    - _Requirements: 11.4, 11.5_

  - [x] 15.4 Page détail
    - Créer `frontend/src/routes/detail/[id]/+page.svelte` ; afficher fiche complète, et pour Series/Anime la liste des saisons et épisodes triés ; afficher bouton `Reprendre` (cible `next_resume_episode`) ou `Revoir depuis le début` si tous les épisodes sont vus ; afficher compteur `X / N épisodes vus`
    - _Requirements: 7.3, 7.4, 8.5, 12.2, 12.3, 12.4, 15.4_

  - [x] 15.5 Page édition Media_Item
    - Créer `frontend/src/routes/edit/[id]/+page.svelte` ; formulaire avec titre, titre original, année, synopsis, genres (input multi-tag), note (`RatingInput`), studio/réalisateur, statut, `PosterPicker` ; valider année ∈ [1900, 2099] ou nulle
    - _Requirements: 6.1, 6.2, 6.3_

  - [x] 15.6 Page édition Episode_Item
    - Créer `frontend/src/routes/edit-episode/[id]/+page.svelte` ; formulaire titre, synopsis, statut, affiche locale ; permettre la modification de la fiche au niveau de la série, de la saison, et de l'épisode
    - _Requirements: 6.5_

  - [x] 15.7 Page paramètres
    - Créer `frontend/src/routes/settings/+page.svelte` avec sections : (a) Dossiers de bibliothèque (ajout / retrait / liste avec catégorie), (b) Lecteur externe (sélecteur de chemin + bouton « Vérifier »), (c) Chemin ffprobe (sélecteur + indicateur sidecar détecté), (d) `scan_au_démarrage` (toggle), (e) Thème (toggle Adwaita Dark / Adwaita Light), (f) `Installer le script Lua` (bouton qui appelle `install_lua_script`)
    - Si l'utilisateur tente d'ajouter un dossier inexistant, afficher l'erreur retournée par le backend (Requirement 2.4)
    - _Requirements: 2.1, 2.2, 2.3, 2.4, 2.5, 3.7, 5.4, 9.7, 10.1, 10.2, 10.3, 10.4, 16.14_

  - [x] 15.8 Page logs
    - Créer `frontend/src/routes/logs/+page.svelte` ; appeler `read_latest_log` et afficher le contenu en `<pre>` monospace, défilement vertical, max-height fenêtre
    - _Requirements: 18.3_

- [x] 16. Checkpoint - Vérifier que tous les tests passent
  - Ensure all tests pass, ask the user if questions arise.

- [x] 17. Tests d'intégration et packaging Windows
  - [x] 17.1 Mock client mpv NDJSON
    - Créer `src-tauri/tests/support/mock_mpv.rs` qui se connecte au `NamedPipeServer` créé par `player_bridge` (en tant que client) et émet une séquence NDJSON paramétrable (`progress` × N puis `closed`)
    - _Requirements: 9.2, 9.3_

  - [x] 17.2 Test d'intégration scan + lecture
    - Créer `src-tauri/tests/integration_play.rs` : préparer une DB temporaire, créer une arborescence de fichiers mp4 vides (avec `Mock ProbeBackend`), exécuter `start_scan`, puis `play_media` avec un `MockExternalPlayer` qui utilise `mock_mpv` ; assert : `watch_progress_seconds = dernière position envoyée`, statut recalculé via `compute_status`
    - _Requirements: 3.1, 8.2, 8.3, 9.4, 17.2_

  - [x] 17.3 Bench recherche 5000 items
    - Créer `src-tauri/tests/bench_search.rs` qui peuple la DB avec 5000 Media_Item titres synthétiques, mesure le temps de `applyFiltersAndSearch` côté store dérivé (test Vitest avec `performance.now()`), assert ≤ 200 ms par frappe
    - _Requirements: 13.3_

  - [x] 17.4 Test de propriété — aucune requête réseau sortante
    - **Property 13 : Aucune requête réseau sortante**
    - Démarrer un faux résolveur DNS local + un proxy TCP qui échoue toute connexion sortante non-localhost ; exécuter une séquence : ajout dossier → scan → édition → lecture mockée → recherche → filtres ; assert que zéro connexion sortante n'est enregistrée
    - **Validates: Requirements 6.4, 17.3**
    - _Requirements: 6.4, 17.3_

  - [x] 17.5 Configurer le bundling Windows MSI/NSIS
    - Dans `tauri.conf.json` : `bundle.targets = ["msi", "nsis"]`, `bundle.windows.wix.upgradeCode = "<UUID stable>"`, `bundle.publisher = "Kyro"`, `bundle.identifier = "app.kyro"`, `bundle.icon = ["icons/icon.ico"]`
    - Configurer l'installeur NSIS pour ne PAS supprimer `%APPDATA%\Kyro\` à la désinstallation par défaut, et exposer une option « Supprimer aussi mes données » qui efface ce dossier
    - _Requirements: 1.1, 1.3, 17.4_

  - [x] 17.6 Script de build Windows
    - Ajouter `scripts/build-windows.ps1` qui exécute `cargo tauri build` et copie les artefacts MSI et NSIS sous `dist/` ; vérifier que la taille du binaire installé hors ffprobe est ≤ 50 Mo (assertion shell)
    - _Requirements: 1.1, 1.3_

- [x] 18. Final checkpoint - Vérifier que tous les tests passent
  - Ensure all tests pass, ask the user if questions arise.

## Notes

- Les sous-tâches marquées `*` sont optionnelles (tests). Les tâches non marquées doivent être implémentées.
- Chaque test de propriété cite explicitement son numéro `Property: P-N` issu du design et les clauses de Requirements qu'il valide.
- Configuration des tests de propriété : minimum 100 itérations (`proptest` par défaut, `fast-check.numRuns: 100`) ; tag `Feature: kyro, Property N: <texte>`.
- Le langage est fixé par le design : Rust pour le backend Tauri, TypeScript/Svelte pour le frontend. Aucune sélection de langage à demander.
- Aucune dépendance HTTP n'est introduite dans le `Cargo.toml`. La feature flag `network` reste désactivée par défaut et fait l'objet d'une vérification explicite par la propriété 13 (test 17.4).
- Le seuil de taille 50 Mo (Requirement 1.3) exclut explicitement le sidecar ffprobe (~25 Mo) ; seul le binaire installé hors ffprobe est mesuré dans la tâche 17.6.
- Les 37 propriétés du design sont toutes couvertes : P1→7.8, P2→7.9, P3→8.6, P4→8.7, P5→8.8, P6→3.2, P7→3.3, P8→7.10, P9→6.5, P10→6.6, P11→7.11, P12→7.12, P13→17.4, P14→7.13, P15→7.14, P16→4.3, P17→4.4, P18→7.15, P19→10.7, P20→10.8, P21→10.9, P22→10.10, P23→10.11, P24→10.12, P25→7.16, P26→7.17, P27→4.5, P28→5.3, P29→5.4, P30→5.5, P31→5.6, P32→14.8, P33→14.9, P34→13.5, P35→13.6, P36→7.18, P37→8.9.

## Task Dependency Graph

```json
{
  "waves": [
    { "id": 0, "tasks": ["1.1"] },
    { "id": 1, "tasks": ["1.2", "1.3", "1.4"] },
    { "id": 2, "tasks": ["2.1", "2.2", "3.1", "4.1", "4.2", "5.1", "5.2", "6.1", "6.3", "11.1"] },
    { "id": 3, "tasks": ["2.3", "3.2", "3.3", "4.3", "4.4", "4.5", "5.3", "5.4", "5.5", "5.6", "6.2"] },
    { "id": 4, "tasks": ["2.4", "6.4", "6.5", "6.6", "7.1"] },
    { "id": 5, "tasks": ["7.2"] },
    { "id": 6, "tasks": ["7.3"] },
    { "id": 7, "tasks": ["7.4"] },
    { "id": 8, "tasks": ["7.5"] },
    { "id": 9, "tasks": ["7.6"] },
    { "id": 10, "tasks": ["7.7", "7.8", "7.9", "7.10", "7.11", "7.12", "7.13", "7.14", "7.15", "7.16", "7.17", "7.18"] },
    { "id": 11, "tasks": ["8.1"] },
    { "id": 12, "tasks": ["8.2"] },
    { "id": 13, "tasks": ["8.3", "8.4", "8.5", "8.6", "8.7", "8.8", "8.9", "10.1", "11.2", "11.3"] },
    { "id": 14, "tasks": ["10.2"] },
    { "id": 15, "tasks": ["10.3", "10.4", "10.5"] },
    { "id": 16, "tasks": ["10.6"] },
    { "id": 17, "tasks": ["10.7", "10.8", "10.9", "10.10", "10.11", "10.12", "12.1", "12.2", "12.3", "12.4", "12.5"] },
    { "id": 18, "tasks": ["12.6", "13.1"] },
    { "id": 19, "tasks": ["13.2", "13.4"] },
    { "id": 20, "tasks": ["13.3", "13.5", "13.6", "14.1", "14.2", "14.3", "14.4", "14.5", "14.6", "14.7"] },
    { "id": 21, "tasks": ["14.8", "14.9", "15.1"] },
    { "id": 22, "tasks": ["15.2", "15.3", "15.4", "15.5", "15.6", "15.7", "15.8"] },
    { "id": 23, "tasks": ["17.1", "17.5"] },
    { "id": 24, "tasks": ["17.2", "17.3", "17.4", "17.6"] }
  ]
}
```
