//! Tâche 17.1 — module commun pour les tests d'intégration.
//!
//! Ce fichier `mod.rs` est déposé sous `src-tauri/tests/support/` afin
//! d'éviter que Cargo ne compile les helpers comme des binaires de test
//! indépendants : seuls les fichiers `.rs` placés **directement** sous
//! `tests/` sont traités comme des integration tests autonomes ; les
//! sous-dossiers (`tests/support/…`) ne sont compilés que lorsqu'un
//! integration test les inclut explicitement via `mod support;`.
//!
//! Re-exporte les sous-modules consommés par les tests d'intégration
//! suivants (tâche 17.2 et au-delà). Pour l'instant, seul
//! [`mock_mpv`] est exposé : le client NDJSON simulant `kyro.lua`
//! côté mpv.net qui écrit dans le `NamedPipeServer` ouvert par
//! `Kyro_Player_Bridge` (cf. `infra::player_bridge::new_session`,
//! `infra::ipc_pipe::listen`).
//!
//! _Requirements: 9.2, 9.3_

#[cfg(target_os = "windows")]
pub mod mock_mpv;
