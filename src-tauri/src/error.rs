//! Enum d'erreur unifié `KyroError` (design § « Gestion d'erreurs »).
//!
//! Toutes les couches (`infra`, `commands`, `domain`) qui peuvent
//! échouer pour des raisons applicatives doivent retourner ce type.
//! Les erreurs purement techniques (`std::io::Error`, `sqlx::Error`)
//! sont absorbées via `#[from]` ; les autres familles (probe, parse,
//! ipc, validation) transportent un message court résumant la cause —
//! le détail technique est journalisé par le site qui construit
//! l'erreur.
//!
//! ## Adaptateur Tauri
//!
//! `impl From<KyroError> for tauri::ipc::InvokeError` produit un
//! message **FR localisé** destiné à être affiché à l'utilisateur final
//! (requirement 18.4). Avant la conversion, la cause technique brute
//! (issue de `Display` thiserror, qui inclut la source quand `#[from]`
//! est utilisé) est journalisée via `tracing::error!`. Le frontend ne
//! reçoit donc qu'un message neutre en français, jamais le détail bas
//! niveau (chemin de fichier, numéro de ligne SQL, code Win32, etc.),
//! tandis que les logs `%APPDATA%\Kyro\logs\kyro.log.YYYY-MM-DD`
//! conservent toute l'information nécessaire au diagnostic
//! (requirement 18.2).

use thiserror::Error;

/// Erreurs métier unifiées de Kyro_Backend.
///
/// Variantes :
/// - `Io` : erreur d'IO système (lecture/écriture fichier, permissions,
///   chemin inaccessible). Convertie automatiquement depuis
///   `std::io::Error`.
/// - `Db` : erreur de la couche persistance (SQLite via `sqlx`).
///   Convertie automatiquement depuis `sqlx::Error`.
/// - `Probe` : échec de l'extraction des métadonnées techniques par
///   ffprobe (exit non nul, JSON illisible, binaire absent). La cause
///   précise est portée par `infra::probe::ProbeError` puis traduite
///   manuellement par le scanner (cf. requirement 5.5 + 18.1) en
///   `KyroError::Probe(String)`.
/// - `Parse` : échec d'analyse syntaxique (NDJSON IPC, JSON ffprobe,
///   nom de fichier non reconnu côté commands).
/// - `Ipc` : défaut sur `Kyro_IPC_Pipe` (named pipe absent,
///   désérialisation NDJSON, déconnexion prématurée). Cf. requirement
///   9.x.
/// - `Config` : binaire externe introuvable, chemin invalide en
///   paramètres, valeur de `setting_value` incohérente. Cf. requirement
///   5.4 (ffprobe absent) et 10.3 (External_Player invalide).
/// - `Validation` : violation d'invariant en entrée d'une commande
///   (note utilisateur hors [0, 10] — 6.6, chemin de dossier inexistant
///   — 2.4, extension d'affiche non supportée, etc.).
#[derive(Debug, Error)]
pub enum KyroError {
    /// Erreur d'IO bas niveau ; transporte la cause système.
    #[error("erreur d'IO: {0}")]
    Io(#[from] std::io::Error),

    /// Erreur de la couche persistance SQLite (`sqlx`).
    #[error("erreur de base de données: {0}")]
    Db(#[from] sqlx::Error),

    /// Échec d'analyse média (ffprobe). Voir `infra::probe::ProbeError`
    /// pour la cause détaillée — le scanner traduit manuellement en
    /// `Probe(String)` afin de rester fidèle au requirement 18.1
    /// (continuation du scan en cas d'erreur isolée).
    #[error("erreur d'analyse média: {0}")]
    Probe(String),

    /// Erreur d'analyse syntaxique (parsing NDJSON, JSON ffprobe,
    /// noms de fichiers non reconnus côté commands).
    #[error("erreur d'analyse: {0}")]
    Parse(String),

    /// Erreur de communication IPC avec mpv.net (named pipe).
    #[error("erreur IPC: {0}")]
    Ipc(String),

    /// Erreur de configuration : binaire externe introuvable, chemin
    /// invalide en paramètres, valeur de `setting_value` incohérente,
    /// etc. Cf. requirement 5.4 (ffprobe absent) et 10.3 (External_Player
    /// invalide — sera consommé par la tâche 10.5).
    #[error("erreur de configuration: {0}")]
    Config(String),

    /// Violation d'invariant côté entrée utilisateur (note hors plage,
    /// dossier inexistant, extension d'affiche non supportée, etc.).
    #[error("erreur de validation: {0}")]
    Validation(String),
}

impl KyroError {
    /// Indique si l'erreur reflète une **défaillance irrécupérable**
    /// du backend justifiant l'émission de l'événement
    /// `backend-degraded` (tâche 12.6, requirement 18.4).
    ///
    /// La granularité est volontairement conservatrice : on ne signale
    /// `backend-degraded` que pour les pannes structurelles du pool
    /// SQLite — pas pour une simple violation de contrainte (CHECK,
    /// FK) ni pour une erreur de décodage isolée. Le frontend
    /// affichera un bandeau persistant en TopBar, ce qui ne doit pas
    /// être déclenché par une erreur métier ponctuelle.
    ///
    /// Cas reconnus comme irrécupérables :
    ///
    /// * `sqlx::Error::PoolTimedOut` — toutes les connexions sont
    ///   prises pendant une durée supérieure à `acquire_timeout`,
    ///   typiquement parce que la DB est verrouillée par un autre
    ///   processus ou qu'une requête écrase le pool. Plusieurs
    ///   essais consécutifs aboutissant à ce variant indiquent un
    ///   verrouillage durable (cf. design § « Mode dégradé »).
    /// * `sqlx::Error::PoolClosed` — le pool a été fermé
    ///   (configuration changée, panique sur la connection task), il
    ///   ne peut plus servir de requêtes.
    /// * `sqlx::Error::Io(_)` — défaillance bas niveau du transport
    ///   SQLite (descripteur fichier perdu, FS du data-dir injoignable).
    ///
    /// Sont volontairement **exclus** :
    ///
    /// * `Database(_)` — erreurs SQL propres (CHECK violé, FK
    ///   manquante, UNIQUE) ; ce sont des erreurs métier
    ///   diagnostiquables par le frontend sans bandeau global.
    /// * `RowNotFound`, `ColumnNotFound`, `Decode` — erreurs locales
    ///   à une requête, n'invalident pas le pool.
    /// * Les autres familles `KyroError` (Io, Probe, Parse, Ipc,
    ///   Config, Validation) — orthogonales au pool DB.
    pub(crate) fn is_irrecoverable_backend_failure(&self) -> bool {
        match self {
            KyroError::Db(err) => matches!(
                err,
                sqlx::Error::PoolTimedOut
                    | sqlx::Error::PoolClosed
                    | sqlx::Error::Io(_)
            ),
            _ => false,
        }
    }

    /// Message **FR localisé** destiné à l'utilisateur final via
    /// `tauri::ipc::InvokeError`. Volontairement neutre : pas de chemin
    /// de fichier ni de code technique, le détail est dans les logs.
    fn user_facing_message_fr(&self) -> String {
        match self {
            KyroError::Io(_) => {
                "Erreur d'accès au système de fichiers.".to_string()
            }
            KyroError::Db(_) => {
                "Erreur d'accès à la base de données locale.".to_string()
            }
            KyroError::Probe(_) => {
                "Erreur lors de l'analyse technique d'un fichier média.".to_string()
            }
            KyroError::Parse(_) => {
                "Erreur d'analyse des données reçues.".to_string()
            }
            KyroError::Ipc(_) => {
                "Erreur de communication avec le lecteur externe.".to_string()
            }
            KyroError::Config(detail) => {
                format!("Erreur de configuration : {detail}.")
            }
            KyroError::Validation(detail) => {
                format!("Erreur de validation : {detail}.")
            }
        }
    }

    /// Identifiant court de la variante, pour structurer les logs sans
    /// dépendre du `Debug` complet (qui inclut des champs internes des
    /// erreurs `sqlx` ou `io::Error` parfois volumineux).
    fn variant_tag(&self) -> &'static str {
        match self {
            KyroError::Io(_) => "io",
            KyroError::Db(_) => "db",
            KyroError::Probe(_) => "probe",
            KyroError::Parse(_) => "parse",
            KyroError::Ipc(_) => "ipc",
            KyroError::Config(_) => "config",
            KyroError::Validation(_) => "validation",
        }
    }
}

/// Conversion vers le type d'erreur attendu par les `#[tauri::command]`
/// en Tauri v2 (`tauri::ipc::InvokeError`).
///
/// Effets :
/// 1. Journalisation `tracing::error!` de la cause technique brute
///    (champ `error = %err`) avec un tag de variante stable
///    (`variant`), afin que le diagnostic complet soit toujours
///    disponible dans `%APPDATA%\Kyro\logs\kyro.log.YYYY-MM-DD`
///    (requirement 18.2).
/// 2. **Mode dégradé** (tâche 12.6, requirement 18.4) : si l'erreur
///    est reconnue comme **irrécupérable** par
///    [`KyroError::is_irrecoverable_backend_failure`] (pool SQLite
///    verrouillé / fermé / IO transport), on émet l'événement global
///    Tauri `backend-degraded` via [`crate::events::emit_backend_degraded`]
///    afin que le frontend affiche un bandeau persistant. L'émission
///    est best-effort (cf. doc de `emit_backend_degraded`) — elle ne
///    peut jamais bloquer la production de l'`InvokeError`.
/// 3. Envoi côté frontend d'un message **FR localisé** neutre via
///    `InvokeError::from(String)` — le frontend l'affichera sans avoir
///    à interpréter la cause technique (requirement 18.4).
impl From<KyroError> for tauri::ipc::InvokeError {
    fn from(err: KyroError) -> Self {
        tracing::error!(
            variant = err.variant_tag(),
            error = %err,
            "Kyro: KyroError remontée vers le frontend"
        );

        if err.is_irrecoverable_backend_failure() {
            // On capture la cause technique brève (pas le message FR
            // localisé) pour l'observabilité côté frontend.
            crate::events::emit_backend_degraded(format!(
                "{}: {err}",
                err.variant_tag()
            ));
        }

        tauri::ipc::InvokeError::from(err.user_facing_message_fr())
    }
}

#[cfg(test)]
mod tests {
    //! Couverture des classifications d'erreurs critiques pour le
    //! mode dégradé (tâche 12.6, requirement 18.4).
    //!
    //! Le wrapper `From<KyroError> for InvokeError` lui-même n'est
    //! pas testable en isolation (l'émission d'événements Tauri exige
    //! un `AppHandle` réel — couvert par les tests d'intégration
    //! 17.x). Ici on cible la fonction pure
    //! `is_irrecoverable_backend_failure` qui décide *quand* le mode
    //! dégradé doit s'activer : une mauvaise classification serait
    //! soit un faux positif (bandeau persistant intempestif), soit un
    //! faux négatif (utilisateur sans signal sur une DB verrouillée).
    use super::*;

    #[test]
    fn pool_timed_out_is_irrecoverable() {
        let err = KyroError::Db(sqlx::Error::PoolTimedOut);
        assert!(err.is_irrecoverable_backend_failure());
    }

    #[test]
    fn pool_closed_is_irrecoverable() {
        let err = KyroError::Db(sqlx::Error::PoolClosed);
        assert!(err.is_irrecoverable_backend_failure());
    }

    #[test]
    fn row_not_found_is_recoverable() {
        // RowNotFound est une erreur métier locale (typiquement :
        // l'utilisateur a supprimé une fiche en parallèle). Pas de
        // bandeau global pour ce cas.
        let err = KyroError::Db(sqlx::Error::RowNotFound);
        assert!(!err.is_irrecoverable_backend_failure());
    }

    #[test]
    fn validation_errors_are_never_irrecoverable() {
        let err = KyroError::Validation("note hors plage".to_string());
        assert!(!err.is_irrecoverable_backend_failure());
    }

    #[test]
    fn config_errors_are_never_irrecoverable() {
        // Erreurs de config (ffprobe absent, lecteur invalide) : il y
        // a déjà des événements dédiés (`ffprobe-missing`,
        // `external-player-missing`) et pas de bandeau global.
        let err = KyroError::Config("ffprobe introuvable".to_string());
        assert!(!err.is_irrecoverable_backend_failure());
    }

    #[test]
    fn ipc_errors_are_never_irrecoverable() {
        let err = KyroError::Ipc("pipe fermé".to_string());
        assert!(!err.is_irrecoverable_backend_failure());
    }

    #[test]
    fn io_errors_are_never_irrecoverable() {
        // Une erreur d'IO sur un fichier média (KyroError::Io) ne
        // signifie *pas* que la DB est cassée — orthogonal au pool.
        let err = KyroError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "fichier média introuvable",
        ));
        assert!(!err.is_irrecoverable_backend_failure());
    }
}
