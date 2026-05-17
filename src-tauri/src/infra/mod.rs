//! Couche d'infrastructure Kyro — IO, FFI, persistance, IPC.
//!
//! Les modules listés dans le design § « Modules backend Rust »
//! (`scanner`, `probe`, `player_bridge`, `ipc_pipe`, `repo`, `posters`,
//! `ffprobe_locator`) sont ajoutés au fil des tâches 6.x à 10.x. Sont
//! déjà disponibles :
//!
//! * `ffprobe_locator` — résolveur du binaire ffprobe (tâche 6.1,
//!   requirements 5.3 / 5.4).
//! * `probe` — wrapper `tokio::process::Command` autour de ffprobe et
//!   trait `ProbeBackend` injectable (tâche 6.2, requirements 5.1, 5.2,
//!   5.5, 5.6).
//! * `repo` — couche d'accès SQLite (`sqlx`) ; pour l'instant restreinte
//!   au CRUD `library_folder` (tâche 7.1, requirements 2.1 → 2.5).
//! * `settings_keys` — allowlist des clés de la table `setting_value`
//!   (tâche 7.5, requirements 2.1, 7.1, 10.1, 10.2, 16.1, 16.14).
//! * `posters` — import d'affiches locales : copie d'un fichier image
//!   vers `%APPDATA%\Kyro\posters\` puis mise à jour de la colonne
//!   `poster_path` (tâche 7.7, requirements 6.2, 6.3, 17.1).
//! * `scanner` — fondation Kyro_Scanner : parcours récursif `walkdir`
//!   avec `follow_links(false)` et filtre d'extensions insensible à la
//!   casse (tâche 8.1, requirements 3.1, 3.2). Les couches diff /
//!   événements / robustesse arriveront aux tâches 8.2 → 8.5.
//! * `player_bridge` — fondation Kyro_Player_Bridge : génération du
//!   `session_id` UUID v4, du nom de pipe `\\.\pipe\kyro-<session_id>`
//!   et de la valeur `--script-opts` consommée par mpv.net (tâche 10.1,
//!   requirement 9.6). Tâche 10.3 : détection `is_mpv_net` + builder
//!   `build_mpv_command` + spawn `launch_mpv_net` (requirement 9.1,
//!   12.4). Tâche 10.4 : routage `branch` / `PlayerKind`, builder
//!   `build_external_command`, spawn `launch_external_player`, façade
//!   unifiée `launch_player` retournant `LaunchOutcome::{MpvNet,
//!   External}`, et constante d'événement
//!   `EVENT_EXTERNAL_PLAYER_NON_MPV` consommée par la couche
//!   `commands::play_media` pour activer le badge UI « reprise exacte
//!   non garantie » (requirement 10.5). Tâche 10.5 : validation
//!   `validate_external_player_path` (existence + extension `.exe`,
//!   requirements 10.3 / 10.4). Tâche 10.6 : `run_ipc_session` —
//!   consomme le `mpsc::Receiver<PipeMessage>` retourné par
//!   `ipc_pipe::listen` et persiste la progression au fil des messages
//!   `Progress` / `Paused` / `Resumed`, écrit la position finale sur
//!   `Closed`, et préserve `watch_progress_seconds` si aucun message
//!   ne transite (requirements 9.3, 9.4, 9.5). Wrapper Tauri-aware
//!   `run_ipc_session_emit` qui émet `EVENT_PLAYBACK_ENDED` à la
//!   terminaison.
//! * `ipc_pipe` — serveur Windows named pipe + lecture NDJSON
//!   asynchrone (tâche 10.2, requirements 9.2, 9.3) : expose `listen`
//!   qui retourne un `mpsc::Receiver<PipeMessage>` alimenté par les
//!   lignes UTF-8 émises par `kyro.lua` côté mpv.net. Les lignes
//!   invalides sont journalisées et ignorées.

pub mod backup;
pub mod ffprobe_locator;
pub mod ipc_pipe;
pub mod player_bridge;
pub mod posters;
pub mod probe;
pub mod repo;
pub mod scanner;
pub mod settings_keys;
pub mod watcher;
