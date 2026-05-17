// Empêche l'ouverture d'une console Windows en mode release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Tâche 2.1 — bootstrap Tauri + logger structuré + pool SQLite.
//!
//! Cycle de vie démarrage (design § « Cycle de vie démarrage », requirement 1.4) :
//! 1. Le builder Tauri 2 est instancié et expose un `setup` closure.
//! 2. Dans `setup` : résolution de `app_data_dir` (`%APPDATA%\Kyro\` sur
//!    Windows) via le `PathResolver` v2 (`app.path().app_data_dir()`), puis
//!    initialisation de `tracing` avec un `RollingFileAppender` quotidien
//!    écrivant dans `<app_data_dir>/logs/kyro.log.YYYY-MM-DD`.
//! 3. Ouverture du pool `SqlitePool` sur `<app_data_dir>/library.db` avec
//!    `journal_mode=WAL`, `synchronous=NORMAL`, `foreign_keys=ON`,
//!    `create_if_missing=true`.
//! 4. Exécution des migrations SQL versionnées via
//!    `sqlx::migrate!("./migrations").run(&pool)` (tâche 2.3, design §
//!    « Persistance »). Le runner est invoqué dans le runtime Tauri via
//!    `block_on` car le `setup` est synchrone. Toute erreur est fatale.
//! 5. Hook `scan_at_startup` (tâche 8.5, requirement 3.7) : si la clé
//!    `setting_value(scan_at_startup)` est truthy, programmation d'un
//!    scan complet asynchrone non-bloquant via
//!    `tauri::async_runtime::spawn` ; voir `kyro::startup`.
//! 6. Toute défaillance critique (création de la fenêtre, résolution du
//!    data dir, init logger, init DB, migrations) est journalisée puis
//!    termine le processus avec un code de sortie ≠ 0 (requirement 1.4
//!    + 18.2). Les erreurs du hook scan_at_startup sont au contraire
//!    **non-fatales** par contrat : elles sont journalisées et le boot
//!    se poursuit.

use std::path::Path;

use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
};
use tauri::Manager;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::EnvFilter;

// Les modules de domaine pur sont déclarés par la crate `lib` (cf.
// `src/lib.rs` : `pub mod domain;`) et seront consommés via
// `kyro::domain::*` quand la couche `commands` sera câblée. Le binaire
// `main.rs` n'a pas besoin de les redéclarer.

/// Wrapper conservant le `WorkerGuard` du writer non-bloquant `tracing`
/// pour toute la durée de vie de l'application Tauri. Sans ce guard, le
/// thread d'écriture du log est arrêté prématurément et les derniers
/// événements peuvent être perdus avant flush.
struct LoggerGuard(#[allow(dead_code)] WorkerGuard);

fn main() {
    // Le `setup` closure de Tauri est exécuté après la résolution du
    // contexte (`generate_context!`) et juste avant la création des
    // fenêtres déclarées dans `tauri.conf.json`. C'est l'endroit canonique
    // pour préparer logger + pool DB en v2 car le `PathResolver` n'est
    // disponible qu'à partir de l'`AppHandle`.
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            // Library — tâche 12.1.
            kyro::commands::library::list_by_category,
            kyro::commands::library::search,
            kyro::commands::library::list_favorites,
            kyro::commands::library::list_to_watch,
            kyro::commands::library::get_media_item,
            kyro::commands::library::list_episodes,
            kyro::commands::library::update_media_item,
            kyro::commands::library::update_episode_item,
            kyro::commands::library::toggle_favorite,
            kyro::commands::library::toggle_to_watch,
            kyro::commands::library::force_status,
            kyro::commands::library::set_watch_progress,
            kyro::commands::library::import_poster_for_media,
            kyro::commands::library::import_poster_for_episode,
            kyro::commands::library::next_resume_episode,
            // Scan — tâche 12.2.
            kyro::commands::scan::start_scan,
            kyro::commands::scan::scan_status,
            // Playback — tâches 11.2, 12.3.
            kyro::commands::playback::install_lua_script,
            kyro::commands::playback::play_media,
            kyro::commands::playback::play_resume,
            kyro::commands::playback::play_rewatch_from_start,
            // Settings — tâche 12.4.
            kyro::commands::settings::add_library_folder,
            kyro::commands::settings::remove_library_folder,
            kyro::commands::settings::list_library_folders,
            kyro::commands::settings::set_external_player_path,
            kyro::commands::settings::get_external_player_path,
            kyro::commands::settings::set_ffprobe_path,
            kyro::commands::settings::set_scan_at_startup,
            kyro::commands::settings::set_theme,
            kyro::commands::settings::get_theme,
            kyro::commands::settings::set_last_category,
            kyro::commands::settings::get_last_category,
            kyro::commands::settings::get_settings_summary,
            // Logs — tâche 12.5.
            kyro::commands::logs::read_latest_log,
            // Backup / restore (v1.0).
            kyro::commands::backup::export_library_backup,
            kyro::commands::backup::restore_library_backup,
            // Updates / version (v1.0).
            kyro::commands::updates::get_app_version,
            kyro::commands::updates::open_releases_page,
        ])
        .setup(|app| {
            // 1. Résolution du data dir Windows (`%APPDATA%\Kyro\`).
            let app_data_dir = match app.path().app_data_dir() {
                Ok(p) => p,
                Err(err) => {
                    // Logger pas encore initialisé : on écrit sur stderr
                    // pour que le code d'erreur soit corrélable depuis un
                    // shell ou l'observateur d'événements Windows.
                    eprintln!(
                        "[kyro][fatal] impossible de résoudre app_data_dir: {err}"
                    );
                    std::process::exit(1);
                }
            };

            // Création best-effort des dossiers cibles. `create_dir_all`
            // est idempotent : pas d'erreur si le dossier existe déjà.
            if let Err(err) = std::fs::create_dir_all(&app_data_dir) {
                eprintln!(
                    "[kyro][fatal] impossible de créer le data dir {}: {err}",
                    app_data_dir.display()
                );
                std::process::exit(1);
            }
            let logs_dir = app_data_dir.join("logs");
            if let Err(err) = std::fs::create_dir_all(&logs_dir) {
                eprintln!(
                    "[kyro][fatal] impossible de créer le dossier de logs {}: {err}",
                    logs_dir.display()
                );
                std::process::exit(1);
            }

            // 2. Initialisation `tracing` — RollingFileAppender quotidien,
            //    format JSON ligne, niveau `info` par défaut, override par
            //    la variable d'environnement `KYRO_LOG`.
            let _logger_guard = match init_tracing(&logs_dir) {
                Ok(g) => g,
                Err(err) => {
                    eprintln!("[kyro][fatal] impossible d'initialiser le logger: {err}");
                    std::process::exit(1);
                }
            };
            // On stocke le guard côté state Tauri pour que le thread
            // d'écriture non-bloquant ne soit pas drop avant la fin de
            // l'application.
            app.manage(LoggerGuard(_logger_guard));

            tracing::info!(
                app_data_dir = %app_data_dir.display(),
                logs_dir = %logs_dir.display(),
                "Kyro: data dir résolu et logger initialisé"
            );

            // 3. Ouverture du pool SQLite sur `library.db`.
            let db_path = app_data_dir.join("library.db");
            let pool = match tauri::async_runtime::block_on(init_sqlite_pool(&db_path)) {
                Ok(p) => p,
                Err(err) => {
                    tracing::error!(
                        error = %err,
                        db_path = %db_path.display(),
                        "Kyro: échec d'ouverture du pool SQLite"
                    );
                    std::process::exit(1);
                }
            };

            // 4. Application des migrations SQL versionnées
            //    (`src-tauri/migrations/0001_init.sql` et suivantes).
            //    `sqlx::migrate!` résout les fichiers à la **compile time**
            //    relativement au `CARGO_MANIFEST_DIR` (`src-tauri/`), ce
            //    qui garantit qu'aucun chemin runtime ne peut diverger.
            //    En cas d'échec : log + `exit(1)` (requirement 1.4 +
            //    18.2). On évite volontairement la conversion via
            //    `KyroError::Db` ici car aucun frontend n'est prêt à
            //    recevoir l'erreur — on est encore dans le `setup`.
            if let Err(err) =
                tauri::async_runtime::block_on(sqlx::migrate!("./migrations").run(&pool))
            {
                tracing::error!(
                    error = %err,
                    db_path = %db_path.display(),
                    "Kyro: échec d'exécution des migrations SQLite"
                );
                std::process::exit(1);
            }

            tracing::info!(
                db_path = %db_path.display(),
                "Kyro: pool SQLite ouvert (WAL, synchronous=NORMAL, foreign_keys=ON) et migrations appliquées"
            );

            // 5. **Tâche 12.6** — initialisation du `AppHandle` global
            //    pour les événements émis hors contexte de commande
            //    (`backend-degraded` depuis le wrapper d'erreur,
            //    `ffprobe-missing` depuis le hook de boot ci-dessous).
            //    `set_app_handle` est idempotent ; le clone est bon
            //    marché (`AppHandle` est un `Arc` interne).
            kyro::events::set_app_handle(app.handle().clone());

            // 6. **Tâche 12.6** — émission `ffprobe-missing` au
            //    démarrage si le résolveur échoue (requirement 5.4).
            //    On exécute la résolution **en best-effort** : un
            //    échec ne bloque pas le boot (les fonctionnalités
            //    nécessitant ffprobe afficheront leur propre erreur
            //    plus tard). Si succès, on logue le chemin résolu ;
            //    sinon, `locate_ffprobe_and_emit` se charge de l'émission
            //    et on l'ignore pour ne pas faire échouer l'init.
            //
            //    On lit d'abord la clé settings `ffprobe_path` (priorité
            //    2 du locator) sur le pool ; en cas d'erreur de lecture
            //    on retombe sur `None` (le locator essaiera sidecar +
            //    PATH système).
            let ffprobe_setting = tauri::async_runtime::block_on(async {
                kyro::infra::repo::Repo::new(pool.clone())
                    .get_setting(
                        kyro::infra::settings_keys::SETTING_KEY_FFPROBE_PATH,
                    )
                    .await
                    .unwrap_or(None)
            });
            let _ = kyro::infra::ffprobe_locator::locate_ffprobe_and_emit(
                app.handle(),
                move |key| {
                    if key
                        == kyro::infra::settings_keys::SETTING_KEY_FFPROBE_PATH
                    {
                        ffprobe_setting
                    } else {
                        None
                    }
                },
            );

            // 7. **Tâche 12.4** — initialisation des paramètres au
            //    premier démarrage : détection automatique de mpv.net
            //    si `external_player_path` n'est pas encore défini
            //    (requirement 10.2). Non-fatale : journalise et passe.
            let repo_for_init = kyro::infra::repo::Repo::new(pool.clone());
            tauri::async_runtime::block_on(
                kyro::commands::settings::init_default_settings(&repo_for_init),
            );

            // 8. Hook `scan_at_startup` (tâche 8.5, requirement 3.7).
            //    Si la clé `setting_value(scan_at_startup)` est truthy,
            //    cet appel programme un scan complet asynchrone en
            //    arrière-plan via `tauri::async_runtime::spawn`. La
            //    fonction est synchrone et **non-bloquante** : aucune
            //    erreur côté scan ne peut bloquer ou faire paniquer
            //    l'init Tauri (cf. doc de `kyro::startup`).
            let repo_for_startup = kyro::infra::repo::Repo::new(pool.clone());
            kyro::startup::maybe_run_startup_scan(repo_for_startup);

            // 9. State partagé pour les commandes Tauri (tâche 12.6).
            //    Toutes les commandes définies dans
            //    `kyro::commands::*` consomment `tauri::State<'_, Repo>` —
            //    on enregistre donc le `Repo` (porteur du pool cloné).
            //    Le pool brut reste également managé pour les futurs
            //    consommateurs qui voudraient ouvrir des transactions
            //    ad-hoc.
            app.manage(kyro::infra::repo::Repo::new(pool.clone()));
            app.manage(pool);

            // 10. Watcher filesystem (v1.0). Surveille tous les
            //     library_folder configurés et déclenche un scan
            //     incrémental dès qu'un fichier est ajouté, supprimé
            //     ou renommé. Évite à l'utilisateur de cliquer sur
            //     « Rafraîchir » après chaque opération FS.
            //
            //     L'init est asynchrone et non-bloquante : tout échec
            //     d'attachement est journalisé sans interrompre le
            //     boot. Le state `Arc<WatcherState>` est manage'd
            //     pour pouvoir être ré-utilisé par les commandes
            //     `add/remove_library_folder` qui appellent
            //     `reload_watches` à chaud.
            let watcher_state = std::sync::Arc::new(
                kyro::infra::watcher::WatcherState::new(),
            );
            app.manage(watcher_state.clone());
            let repo_for_watcher = kyro::infra::repo::Repo::new(
                app.state::<sqlx::sqlite::SqlitePool>().inner().clone(),
            );
            let app_handle_for_watcher = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // On attend la création effective de la fenêtre main
                // pour avoir un handle de Window valide à émettre des
                // événements. Boucle d'attente courte (max ~2s).
                use tauri::Manager;
                let mut attempts = 0;
                let window = loop {
                    if let Some(w) = app_handle_for_watcher.get_webview_window("main") {
                        break w.as_ref().window();
                    }
                    if attempts >= 40 {
                        tracing::warn!(
                            "Kyro_Watcher: fenêtre main introuvable après 2s — watcher non démarré"
                        );
                        return;
                    }
                    attempts += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                };
                kyro::infra::watcher::init_watcher(
                    watcher_state,
                    repo_for_watcher,
                    window,
                )
                .await;
            });

            Ok(())
        })
        .run(tauri::generate_context!());

    // Échec post-`setup` : typiquement la création de la fenêtre
    // principale, qui couvre directement le requirement 1.4. Le logger
    // peut être initialisé (setup a réussi) ou non (panic avant setup) ;
    // on émet sur les deux canaux pour être sûr que l'erreur sera
    // observable.
    if let Err(err) = result {
        eprintln!(
            "[kyro][fatal] impossible d'initialiser l'application Tauri (fenêtre principale ?): {err}"
        );
        tracing::error!(
            error = %err,
            "Kyro: échec critique d'initialisation Tauri — sortie code 1"
        );
        std::process::exit(1);
    }
}

/// Initialise l'abonné global `tracing` :
/// - `RollingFileAppender` quotidien dans `<logs_dir>/kyro.log.YYYY-MM-DD`,
/// - writer non-bloquant (préserve la latence du backend),
/// - format JSON ligne (un événement = une ligne, parsable),
/// - filtre depuis la variable d'environnement `KYRO_LOG` ; défaut
///   `info,sqlx=warn` (les requêtes sqlx sont volumineuses au niveau
///   `info`).
///
/// Retourne le `WorkerGuard` du writer non-bloquant — l'appelant DOIT le
/// conserver vivant tant que l'application tourne, sinon les derniers
/// événements ne seront pas flushés.
fn init_tracing(logs_dir: &Path) -> Result<WorkerGuard, String> {
    let appender = RollingFileAppender::new(Rotation::DAILY, logs_dir, "kyro.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(appender);

    let env_filter = EnvFilter::try_from_env("KYRO_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn"));

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_writer(non_blocking)
        .with_ansi(false)
        .json()
        .try_init()
        .map_err(|e| format!("tracing_subscriber::try_init a échoué: {e}"))?;

    Ok(guard)
}

/// Ouvre le pool SQLite avec les pragmas requis par le design :
/// - `journal_mode=WAL` (concurrence lectures pendant écritures),
/// - `synchronous=NORMAL` (compromis durabilité/perf adapté à WAL),
/// - `foreign_keys=ON` (intégrité référentielle entre `library_folder`,
///   `media_item`, `episode_item`, etc.),
/// - `create_if_missing=true` (le fichier `library.db` est créé au
///   premier lancement).
async fn init_sqlite_pool(db_path: &Path) -> Result<SqlitePool, sqlx::Error> {
    let connect_options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true);

    SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(connect_options)
        .await
}
