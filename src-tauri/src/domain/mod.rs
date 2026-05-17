//! Domaine pur Kyro — types et fonctions sans IO ni dépendance à Tauri.
//!
//! Chaque sous-module est indépendant des couches `infra` et `commands`
//! et reste testable directement (`cargo test`). Les sous-modules
//! actuels :
//!
//! * `types`   — types partagés (`Category`, `Quality`,
//!   `NumberingSource`, `MediaItem`, `MediaType`, `MediaItemId`).
//! * `parser`  — Kyro_Parser (tâche 3.1, requirement 4.x).
//! * `status`  — calcul du statut vu/en_cours/non_vu (tâche 4.1).
//! * `resume`  — prochain épisode à reprendre (tâche 4.2).
//! * `filters` — filtrage genre/année/type/statut (tâche 5.1).
//! * `search`  — normalisation Unicode + matching (tâche 5.2).

pub mod filters;
pub mod parser;
pub mod resume;
pub mod search;
pub mod status;
pub mod types;

pub use types::{
    Category, EpisodeItemId, MediaItem, MediaItemId, MediaType, NumberingSource, Quality,
};
