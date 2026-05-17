//! Tâche 12.6 — événements globaux et mode dégradé du backend.
//!
//! Ce module centralise :
//!
//! * Les **noms canoniques** des événements Tauri émis indépendamment
//!   d'une commande IPC spécifique. Les événements liés à un domaine
//!   précis (`scan-progress`, `scan-completed`, `playback-ended`,
//!   `external-player-missing`, `external-player-non-mpv`) restent
//!   définis dans leur module d'origine — ce module ne couvre que les
//!   événements **globaux** :
//!     - [`EVENT_FFPROBE_MISSING`] — re-export de la constante du
//!       module `infra::ffprobe_locator`. Émis une seule fois au boot
//!       lorsque le résolveur ffprobe échoue (sidecar absent, setting
//!       invalide, PATH dépourvu de `ffprobe.exe`).
//!     - [`EVENT_BACKEND_DEGRADED`] — émis depuis le wrapper d'erreur
//!       des commandes Tauri (`From<KyroError> for InvokeError`)
//!       lorsqu'une erreur **irrécupérable** du backend remonte
//!       (typiquement `sqlx::Error::PoolTimedOut` après plusieurs
//!       tentatives sur une DB verrouillée, ou `PoolClosed` indiquant
//!       que le pool n'est plus utilisable). Le frontend observe cet
//!       événement pour afficher un bandeau persistant en TopBar
//!       (cf. tâche 14.5 + design § « UI : mode dégradé »).
//!
//! * Le `AppHandle` Tauri stocké dans un [`OnceLock`] global, initialisé
//!   une seule fois depuis `main.rs::setup`. Cette indirection est
//!   nécessaire car la conversion `From<KyroError> for InvokeError`
//!   est invoquée par le runtime Tauri *en dehors* du contexte d'une
//!   commande : elle ne reçoit ni `tauri::State`, ni `tauri::Window`,
//!   ni `AppHandle`. Le seul point d'entrée pour émettre depuis ce
//!   site est donc un handle global accessible à tous les threads.
//!   `OnceLock` garantit l'initialisation atomique sans verrou
//!   d'exécution une fois le handle posé.
//!
//! _Requirements: 5.4, 18.4_

use std::sync::OnceLock;

use tauri::{AppHandle, Emitter, Wry};

/// Nom canonique de l'événement émis lorsque le backend bascule en
/// **mode dégradé** suite à une erreur irrécupérable (DB verrouillée
/// plusieurs essais, pool fermé, etc.).
///
/// Le frontend (`frontend/src/lib/api/events.ts`, tâche 13.2) écoute
/// `listen("backend-degraded", payload)` puis bascule
/// `flags.backendDegraded = true` dans le store de bibliothèque. La
/// TopBar (tâche 14.5) affiche alors un badge persistant invitant
/// l'utilisateur à redémarrer Kyro.
///
/// Le payload est une `String` décrivant brièvement la cause technique
/// (ex. `"db: pool timeout"`) — utile pour les logs frontend, jamais
/// affiché tel quel à l'utilisateur (un message FR localisé est rendu
/// par le store côté frontend).
///
/// _Requirements: 18.4_
pub const EVENT_BACKEND_DEGRADED: &str = "backend-degraded";

/// Re-export de [`crate::infra::ffprobe_locator::EVENT_FFPROBE_MISSING`]
/// pour que `events::EVENT_FFPROBE_MISSING` reste l'unique import
/// nécessaire côté `main.rs` et tests d'intégration. La source de
/// vérité reste le module `ffprobe_locator` (qui l'émet déjà via
/// `locate_ffprobe_and_emit` au sein de la couche `infra`).
///
/// _Requirements: 5.4_
pub const EVENT_FFPROBE_MISSING: &str =
    crate::infra::ffprobe_locator::EVENT_FFPROBE_MISSING;

/// Stockage global du `AppHandle` Tauri.
///
/// Initialisé une seule fois par [`set_app_handle`] depuis `main.rs::
/// setup` ; lecture multi-thread sans verrou via [`OnceLock::get`].
/// Si une seconde initialisation est tentée (cas anormal — un seul
/// processus Tauri par exécutable), elle est silencieusement ignorée
/// pour rester idempotente.
///
/// Le runtime est figé à `Wry` : c'est l'unique runtime utilisé par le
/// binaire Kyro (cf. `tauri::Builder::default()` qui produit un
/// `Builder<Wry>`). Les modules `infra::*` qui prennent un
/// `AppHandle<R: Runtime>` générique restent compatibles — ce module
/// est uniquement consommé par le binaire et le wrapper d'erreur.
static APP_HANDLE: OnceLock<AppHandle<Wry>> = OnceLock::new();

/// Initialise le `AppHandle` global. Appelé une seule fois depuis
/// `main.rs::setup` après l'instanciation du builder Tauri.
///
/// Cette fonction est **idempotente** : un second appel est ignoré
/// silencieusement. Cela protège contre un appel accidentel depuis un
/// test ou un re-mount éventuel sans risque de panique.
pub fn set_app_handle(handle: AppHandle<Wry>) {
    if APP_HANDLE.set(handle).is_err() {
        tracing::debug!(
            "Kyro events: set_app_handle ignoré (déjà initialisé)"
        );
    }
}

/// Retourne le `AppHandle` global, ou `None` s'il n'a pas encore été
/// initialisé. Utilisé par les modules qui veulent émettre un
/// événement global depuis un site sans `tauri::State` (ex. wrapper
/// d'erreur, hook startup).
pub fn try_app_handle() -> Option<AppHandle<Wry>> {
    APP_HANDLE.get().cloned()
}

/// Émet l'événement [`EVENT_BACKEND_DEGRADED`] sur le `AppHandle`
/// global avec un payload textuel décrivant brièvement la cause.
///
/// **Best-effort** : si le handle n'est pas encore initialisé (cas
/// extrême : erreur survenue *pendant* le `setup` Tauri, avant
/// `set_app_handle`), on journalise via `tracing::warn!` et on retourne
/// silencieusement — la cause technique reste consultable dans
/// `%APPDATA%\Kyro\logs\` (requirement 18.2). Si l'émission elle-même
/// échoue (canal IPC saturé, fenêtre fermée), même politique : log et
/// retour silencieux. La conversion `From<KyroError>` ne doit *jamais*
/// échouer à produire un `InvokeError` à cause d'un effet de bord
/// d'observabilité.
pub fn emit_backend_degraded(reason: impl Into<String>) {
    let reason = reason.into();
    let Some(app) = APP_HANDLE.get() else {
        tracing::warn!(
            event = EVENT_BACKEND_DEGRADED,
            reason = %reason,
            "Kyro events: AppHandle non initialisé, backend-degraded non émis"
        );
        return;
    };
    match app.emit(EVENT_BACKEND_DEGRADED, &reason) {
        Ok(()) => tracing::error!(
            event = EVENT_BACKEND_DEGRADED,
            reason = %reason,
            "Kyro events: backend-degraded émis (mode dégradé)"
        ),
        Err(err) => tracing::warn!(
            event = EVENT_BACKEND_DEGRADED,
            reason = %reason,
            error = %err,
            "Kyro events: échec émission backend-degraded (best-effort, ignoré)"
        ),
    }
}

#[cfg(test)]
mod tests {
    //! Couverture unitaire des constantes exportées.
    //!
    //! Les fonctions `set_app_handle` / `emit_backend_degraded` /
    //! `try_app_handle` ne peuvent pas être testées en isolation sans
    //! un runtime Tauri Wry réel (besoin d'un `AppHandle<Wry>` valide).
    //! Leur comportement est validé par les tests d'intégration des
    //! tâches 17.x via le pipeline Tauri complet, et par les tests
    //! manuels de la tâche 14.5 (TopBar / bandeaux).
    //!
    //! Ce qui reste **testable ici** : les littéraux des constantes,
    //! qui constituent un contrat figé avec le frontend (`listen("...
    //! ", ...)`). Toute modification accidentelle d'un littéral
    //! romprait silencieusement la chaîne IPC — d'où l'assertion
    //! explicite.

    use super::*;

    #[test]
    fn event_backend_degraded_constant_value() {
        assert_eq!(EVENT_BACKEND_DEGRADED, "backend-degraded");
    }

    #[test]
    fn event_ffprobe_missing_reexport_matches_source() {
        // La constante doit pointer exactement sur la valeur définie
        // par `infra::ffprobe_locator` — pas de drift entre les deux
        // chemins d'import possibles.
        assert_eq!(
            EVENT_FFPROBE_MISSING,
            crate::infra::ffprobe_locator::EVENT_FFPROBE_MISSING
        );
        assert_eq!(EVENT_FFPROBE_MISSING, "ffprobe-missing");
    }
}
