//! Couche `commands` — adaptateurs `#[tauri::command]` exposés au
//! frontend SvelteKit (design § « Modules backend Rust »).
//!
//! Chaque sous-module regroupe un domaine fonctionnel :
//!
//! * `playback` — installation du script Lua mpv.net (tâche 11.2,
//!   requirement 9.7) ; les commandes `play_media` / arrêt /
//!   suivi de progression viennent ensuite (tâches 11.x suivantes).
//! * `logs` — exposition au frontend du contenu du fichier de log
//!   le plus récent (tâche 12.5, requirement 18.3).
//! * `scan` — pilotage du `Kyro_Scanner` (tâche 12.2, requirements
//!   3.1 + 3.6) : `start_scan` (non-bloquant via
//!   `tauri::async_runtime::spawn`) et `scan_status`. Relaye les
//!   événements `scan-progress` (émis par le scanner) et
//!   `scan-completed` (émis par ce module à la fin du scan).
//!
//! * `settings` — paramètres utilisateur (tâche 12.4) : dossiers de
//!   bibliothèque, chemin External_Player, chemin ffprobe manuel, scan
//!   au démarrage, thème, dernière catégorie consultée. Cf.
//!   requirements 2.1–2.5, 5.4, 7.1, 10.1–10.4, 16.1, 16.14.
//!
//! Le module `library` annoncé dans le design sera ajouté au fil des
//! tâches 12.x. La registration des commandes sur le `tauri::Builder`
//! (`invoke_handler`) est explicitement déléguée à la tâche 12.x — ce
//! module se contente d'exposer les fonctions.

pub mod backup;
pub mod embedded_player;
pub mod logs;
pub mod library;
pub mod playback;
pub mod scan;
pub mod settings;
pub mod updates;
