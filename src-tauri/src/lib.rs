//! Crate library Kyro.
//!
//! Le binaire `main.rs` reste le point d'entrée Tauri ; cette
//! bibliothèque expose les modules réutilisables (domaine pur,
//! infrastructure, erreurs unifiées) afin qu'ils soient consommables
//! par les tests d'intégration et par les `commands` Tauri ajoutées par
//! les tâches suivantes (cf. design § « Modules backend Rust »).

pub mod commands;
pub mod domain;
pub mod error;
pub mod events;
pub mod infra;
pub mod startup;
