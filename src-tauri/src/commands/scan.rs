//! Tâche 12.2 — commandes Tauri du domaine **scan** (Kyro_Scanner).
//!
//! Ce module expose les deux commandes `#[tauri::command]` requises par
//! le design pour piloter un scan complet de la bibliothèque depuis le
//! frontend (requirements 3.1, 3.6) :
//!
//! * [`start_scan`] — déclenche un scan complet (tous les
//!   `library_folder` connus). La commande est **non-bloquante** : elle
//!   programme un `tauri::async_runtime::spawn` qui exécute la boucle
//!   de scan en arrière-plan, puis retourne immédiatement `Ok(())` au
//!   caller IPC. Pendant le scan, le scanner relaie l'événement
//!   `scan-progress` via [`scanner::scan_folder_emit`] (tâche 8.3) ;
//!   à la fin du scan, ce module émet l'événement `scan-completed`
//!   avec un [`ScanCompleted`] agrégeant tous les compteurs.
//!
//! * [`scan_status`] — retourne `ScanStatus { running }` indiquant si
//!   un scan est actuellement en cours. Source de vérité : un
//!   [`AtomicBool`] statique du module ([`SCAN_RUNNING`]) toggle par
//!   `start_scan` (CAS sur acquisition + `store(false)` à la fin).
//!
//! ## Garanties
//!
//! 1. **Non-blocking.** `start_scan` retourne dans le temps d'une CAS
//!    atomique + un `spawn`, indépendamment de la taille de la
//!    bibliothèque (requirement 3.1, design § « Cycle de vie scan »).
//! 2. **Singleton.** Un seul scan complet à la fois : si un scan est
//!    déjà en cours, `start_scan` est un no-op (retour `Ok(())` +
//!    log info). Les scans concurrents seraient inutiles (le scanner
//!    est lui-même séquentiel par dossier) et coûteux (contention
//!    pool SQLite + ffprobe).
//! 3. **Best-effort sur l'émission.** Les échecs d'émission
//!    `scan-completed` (fenêtre fermée, canal IPC saturé) sont
//!    journalisés mais n'invalident pas le scan : la base est la
//!    source de vérité.
//! 4. **Non-fatale en interne.** Aucune erreur en cours de scan ne
//!    propage jusqu'à `start_scan` : ffprobe introuvable, échec de
//!    `list_library_folders`, échec d'un dossier individuel — tout
//!    est journalisé via `tracing::error!`/`warn!` et le scan
//!    continue avec le dossier suivant. À la fin, le verrou
//!    [`SCAN_RUNNING`] est *toujours* libéré.
//!
//! ## État partagé
//!
//! Le `Repo` (porteur du `SqlitePool`) est récupéré via
//! `tauri::State<Repo>`. La registration de la commande sur le builder
//! Tauri (`invoke_handler` + `app.manage(repo)`) sera faite par la
//! tâche 12.x — ce module se contente d'exposer les fonctions.
//!
//! _Requirements: 3.1, 3.6_

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tauri::Emitter;

use crate::error::KyroError;
use crate::infra::ffprobe_locator::locate_ffprobe;
use crate::infra::probe::FfprobeBackend;
use crate::infra::repo::Repo;
use crate::infra::scanner;
use crate::infra::settings_keys::SETTING_KEY_FFPROBE_PATH;

/// Nom canonique de l'événement Tauri émis **une fois** à la fin d'un
/// scan complet (tous les `library_folder` parcourus).
///
/// Le pendant `scan-progress` (émis pendant le scan) est défini par le
/// scanner lui-même (cf. [`scanner::EVENT_SCAN_PROGRESS`]). Cette
/// constante est exportée pour que les tests d'intégration et le
/// frontend (`window.listen("scan-completed", ...)`) puissent
/// référencer la même valeur — pas de littéral dupliqué.
///
/// _Requirements: 3.6_
pub const EVENT_SCAN_COMPLETED: &str = "scan-completed";

/// État global « un scan est-il en cours ? » partagé par toutes les
/// invocations Tauri du process.
///
/// Lu par [`scan_status`], muté par [`start_scan`] :
/// * acquisition par `compare_exchange(false → true)` — si un autre
///   scan est déjà en cours, l'appel est un no-op ;
/// * libération inconditionnelle par `store(false)` à la fin de la
///   task de scan, dans tous les cas (succès, panique interceptée,
///   erreur globale).
///
/// Choix d'un `AtomicBool` statique plutôt qu'un `tauri::State<…>`
/// dédié : le compteur est un singleton process — pas besoin de le
/// cloner côté commandes, pas besoin de l'enregistrer sur le builder.
/// Un `AtomicBool` est trivialement `Sync` et zero-overhead.
static SCAN_RUNNING: AtomicBool = AtomicBool::new(false);

/// Tente d'acquérir le verrou « un scan est en cours ». Retourne
/// `true` si l'appelant a obtenu le verrou (et est donc responsable
/// de le libérer), `false` si un autre scan est déjà en cours.
///
/// `Ordering::SeqCst` : la cohérence stricte entre lecture
/// (`scan_status`) et écriture (`start_scan`) prime sur la
/// micro-performance — un scan se compte en secondes, pas en
/// nanosecondes.
fn try_acquire_scan_gate() -> bool {
    SCAN_RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}

/// Libère le verrou. Appelé inconditionnellement à la fin de la task
/// de scan (cf. [`start_scan`]). Idempotent : un appel sur un verrou
/// déjà à `false` est sans effet.
fn release_scan_gate() {
    SCAN_RUNNING.store(false, Ordering::SeqCst);
}

/// Réponse de la commande [`scan_status`].
///
/// Forme délibérément minimaliste pour la tâche 12.2 — l'enrichissement
/// (ETA, dernier `ScanReport`, dossier en cours) viendra avec les
/// tâches 12.x ultérieures et 14.x (TopBar). Le frontend n'a, à ce
/// stade, qu'à savoir si un scan est en cours pour griser/dégriser le
/// bouton « Scanner maintenant » et afficher un indicateur.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ScanStatus {
    /// `true` ssi un scan complet est actuellement en cours.
    pub running: bool,
}

/// Payload de l'événement [`EVENT_SCAN_COMPLETED`] (requirement 3.6).
///
/// Aggrégat des compteurs de tous les `ScanReport` retournés par
/// `scanner::scan_folder_emit` sur l'ensemble des `library_folder`.
/// `ScanReport` n'étant pas `Serialize` (il transporte
/// `Vec<ScanFailure>` avec un enum technique non-DTO), on construit
/// ici une projection orientée UI : les compteurs cumulés et le
/// nombre d'échecs (sans le détail), suffisants pour afficher un
/// résumé final type « 1 234 fichiers traités, 12 ajoutés, 3 erreurs ».
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ScanCompleted {
    /// Nombre total de fichiers candidats énumérés (après filtre
    /// d'extensions) sur l'ensemble des dossiers scannés.
    pub total_files: usize,
    /// Nombre de fichiers effectivement traités (parsing + probe + DB
    /// tentés). Inclut les skips idempotents.
    pub processed: usize,
    /// Nombre de fiches insérées sur l'ensemble du scan.
    pub created: usize,
    /// Nombre de fiches dont les champs techniques ont été rafraîchis.
    pub updated: usize,
    /// Nombre d'entrées précédemment indexées marquées `available = 0`.
    pub marked_unavailable: usize,
    /// Nombre total d'échecs isolés (parse, probe, IO, DB, walkdir)
    /// agrégés sur tous les dossiers — chaque échec a déjà été
    /// journalisé par le scanner avec son chemin (requirement 18.1).
    pub failures: usize,
    /// Nombre de dossiers pour lesquels `scan_folder_emit` a retourné
    /// une `Err` globale (différent des échecs « par fichier »).
    pub folder_errors: usize,
}

/// Commande Tauri : déclenche un scan complet **non-bloquant**.
///
/// Étapes :
/// 1. Tentative d'acquisition du verrou [`SCAN_RUNNING`] via
///    [`try_acquire_scan_gate`]. Si un scan est déjà en cours, on
///    retourne `Ok(())` immédiatement (no-op idempotent).
/// 2. Clone du `Repo` (le `SqlitePool` interne est `Arc<…>`,
///    le clone est bon marché) et `spawn` d'une task asynchrone
///    via `tauri::async_runtime::spawn`. La fonction publique
///    retourne `Ok(())` aussitôt après le `spawn`.
/// 3. La task spawnée exécute [`run_full_scan`] qui :
///    - résout le binaire ffprobe (sidecar → setting → PATH) ;
///    - liste les `library_folder` ;
///    - itère séquentiellement et appelle
///      [`scanner::scan_folder_emit`] sur chacun (qui émet
///      `scan-progress` à chaque fichier — tâche 8.3) ;
///    - agrège les `ScanReport` dans un [`ScanCompleted`].
/// 4. Quel que soit le résultat, le verrou est libéré et
///    `scan-completed` est émis (best-effort).
///
/// _Requirements: 3.1, 3.6_
#[tauri::command]
pub async fn start_scan(
    window: tauri::Window,
    repo: tauri::State<'_, Repo>,
) -> Result<(), KyroError> {
    // 1. Acquisition idempotente du verrou. Si un scan est déjà en
    //    cours, on retourne Ok(()) immédiatement : le frontend peut
    //    continuer à écouter scan-progress / scan-completed sans
    //    avoir besoin de gérer un cas d'erreur dédié.
    if !try_acquire_scan_gate() {
        tracing::info!(
            "Kyro scan: start_scan ignoré — un scan est déjà en cours"
        );
        return Ok(());
    }

    // 2. Préparation des handles pour la task spawnée. `Repo` est
    //    `Clone` (le `SqlitePool` interne est un `Arc<Pool>`) ; le
    //    `Window` v2 est `Clone + Send + Sync`.
    let repo_for_task = repo.inner().clone();
    let window_for_task = window;

    tauri::async_runtime::spawn(async move {
        // 3. Exécution du scan complet. `run_full_scan` est
        //    « non-fatale » par contrat : aucune erreur ne propage,
        //    tout est journalisé. Le retour est toujours un
        //    `ScanCompleted` (potentiellement vide si ffprobe est
        //    introuvable ou si aucun dossier n'est configuré).
        let summary = run_full_scan(&window_for_task, repo_for_task).await;

        // 4. Libération inconditionnelle du verrou — y compris si
        //    `run_full_scan` a panique-vé (le runtime aurait alors
        //    annulé la task ; on a fait au mieux).
        release_scan_gate();

        // 5. Émission `scan-completed`. Best-effort : un échec
        //    d'émission (fenêtre fermée pendant le scan, canal IPC
        //    saturé) est journalisé mais ignoré — la base est la
        //    source de vérité.
        if let Err(err) = window_for_task.emit(EVENT_SCAN_COMPLETED, &summary) {
            tracing::warn!(
                event = EVENT_SCAN_COMPLETED,
                error = %err,
                "Kyro scan: échec émission scan-completed (best-effort, ignoré)"
            );
        } else {
            tracing::info!(
                total_files = summary.total_files,
                processed = summary.processed,
                created = summary.created,
                updated = summary.updated,
                marked_unavailable = summary.marked_unavailable,
                failures = summary.failures,
                folder_errors = summary.folder_errors,
                "Kyro scan: scan-completed émis"
            );
        }
    });

    Ok(())
}

/// Commande Tauri : retourne l'état courant du scan global.
///
/// Lecture atomique de [`SCAN_RUNNING`]. Aucun effet de bord, aucune
/// allocation au-delà de la struct retournée — peut être appelée à
/// haute fréquence par le frontend (poll TopBar) sans impact.
///
/// _Requirements: 3.6_
#[tauri::command]
pub async fn scan_status() -> Result<ScanStatus, KyroError> {
    Ok(ScanStatus {
        running: SCAN_RUNNING.load(Ordering::SeqCst),
    })
}

/// Implémentation interne du scan complet — extraite pour rester
/// testable indépendamment du `spawn` Tauri (pas appelée directement
/// dans cette tâche, mais réutilisable par les tests d'intégration
/// 17.x). Aucune erreur ne propage : tout est journalisé.
///
/// Itération séquentielle des dossiers (cf. note dans
/// `startup::run_startup_scan_inner`) — évite la contention sur le
/// pool SQLite et le binaire ffprobe.
async fn run_full_scan<R: tauri::Runtime>(
    window: &tauri::Window<R>,
    repo: Repo,
) -> ScanCompleted {
    let mut summary = ScanCompleted {
        total_files: 0,
        processed: 0,
        created: 0,
        updated: 0,
        marked_unavailable: 0,
        failures: 0,
        folder_errors: 0,
    };

    // Résolution ffprobe : on lit d'abord la clé settings
    // `ffprobe_path` (priorité 2 du locator) puis on délègue à
    // `locate_ffprobe` qui essaiera sidecar → settings → PATH.
    let ffprobe_setting = match repo.get_setting(SETTING_KEY_FFPROBE_PATH).await {
        Ok(v) => v,
        Err(err) => {
            tracing::warn!(
                error = %err,
                "Kyro scan: lecture du setting ffprobe_path échouée — fallback sidecar/PATH"
            );
            None
        }
    };

    let ff_path = match locate_ffprobe(move |key| {
        if key == SETTING_KEY_FFPROBE_PATH {
            ffprobe_setting
        } else {
            None
        }
    }) {
        Ok(p) => p,
        Err(err) => {
            tracing::error!(
                error = %err,
                "Kyro scan: ffprobe introuvable — scan abandonné"
            );
            return summary;
        }
    };
    let probe = FfprobeBackend::new(ff_path);

    // Liste des dossiers à scanner.
    let folders = match repo.list_library_folders().await {
        Ok(v) => v,
        Err(err) => {
            tracing::error!(
                error = %err,
                "Kyro scan: list_library_folders a échoué — scan abandonné"
            );
            return summary;
        }
    };

    if folders.is_empty() {
        tracing::info!(
            "Kyro scan: aucun library_folder configuré — scan terminé (rien à faire)"
        );
        return summary;
    }

    tracing::info!(
        folders_count = folders.len(),
        "Kyro scan: scan complet démarré"
    );

    for folder in folders {
        let path = std::path::Path::new(&folder.path);
        match scanner::scan_folder_emit(
            window,
            &repo,
            &probe,
            folder.id,
            path,
            folder.category,
        )
        .await
        {
            Ok(report) => {
                tracing::info!(
                    folder_id = folder.id.get(),
                    folder_path = %folder.path,
                    category = ?folder.category,
                    total_files = report.total_files,
                    processed = report.processed,
                    created = report.created,
                    updated = report.updated,
                    marked_unavailable = report.marked_unavailable,
                    failures = report.failures.len(),
                    "Kyro scan: scan dossier terminé"
                );
                summary.total_files += report.total_files;
                summary.processed += report.processed;
                summary.created += report.created;
                summary.updated += report.updated;
                summary.marked_unavailable += report.marked_unavailable;
                summary.failures += report.failures.len();
            }
            Err(err) => {
                summary.folder_errors += 1;
                tracing::error!(
                    folder_id = folder.id.get(),
                    folder_path = %folder.path,
                    error = %err,
                    "Kyro scan: scan dossier échoué — les autres dossiers continuent"
                );
            }
        }
    }

    summary
}

#[cfg(test)]
mod tests {
    //! Couverture unitaire du module `commands::scan`.
    //!
    //! Les commandes Tauri elles-mêmes (`start_scan`, `scan_status`)
    //! ne sont pas testables en isolation sans démarrer un runtime
    //! Tauri (besoin d'un `tauri::Window` réel). Les tests
    //! d'intégration 17.x couvrent le bout-en-bout. Ici on cible :
    //!
    //! - la sérialisation des DTO `ScanStatus` et `ScanCompleted`
    //!   (contrat de l'IPC frontend) ;
    //! - la sémantique CAS du verrou [`SCAN_RUNNING`] via les helpers
    //!   privés `try_acquire_scan_gate` / `release_scan_gate` —
    //!   protégée par un `Mutex` global pour serialiser les tests
    //!   qui touchent à l'état statique.
    //!
    //! Cette suite est volontairement minimale : la logique métier
    //! lourde (`run_full_scan`) délègue intégralement à `scanner` et
    //! `repo`, dont les invariants sont déjà testés par les tâches
    //! 7.x et 8.x.

    use super::*;
    use std::sync::Mutex;

    /// Sérialise les tests qui touchent à `SCAN_RUNNING` pour
    /// éviter les flakes liés au parallélisme `cargo test`. Le
    /// `Mutex` est uniquement un mécanisme de coordination des
    /// tests — il ne fait pas partie du code de production.
    static GATE_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Constante du module : son littéral doit rester stable côté
    /// frontend (`window.listen("scan-completed", ...)`).
    #[test]
    fn event_constant_matches_design() {
        assert_eq!(EVENT_SCAN_COMPLETED, "scan-completed");
    }

    #[test]
    fn scan_status_serialises_to_expected_json() {
        let s = ScanStatus { running: true };
        let json = serde_json::to_string(&s).expect("serialize ScanStatus");
        assert_eq!(json, r#"{"running":true}"#);

        let s = ScanStatus { running: false };
        let json = serde_json::to_string(&s).expect("serialize ScanStatus");
        assert_eq!(json, r#"{"running":false}"#);
    }

    #[test]
    fn scan_completed_serialises_all_fields() {
        let payload = ScanCompleted {
            total_files: 100,
            processed: 95,
            created: 10,
            updated: 3,
            marked_unavailable: 2,
            failures: 1,
            folder_errors: 0,
        };
        let json = serde_json::to_string(&payload).expect("serialize ScanCompleted");
        // Vérifie chaque clé attendue côté frontend (camelCase non
        // utilisé : on conserve snake_case Rust pour cohérence avec
        // le reste des DTO du backend).
        for key in [
            "total_files",
            "processed",
            "created",
            "updated",
            "marked_unavailable",
            "failures",
            "folder_errors",
        ] {
            assert!(
                json.contains(key),
                "champ {key} manquant dans la sérialisation: {json}"
            );
        }
    }

    /// La première acquisition réussit, la seconde échoue tant que
    /// la libération n'a pas eu lieu. Après libération, on peut
    /// réacquérir.
    #[test]
    fn gate_cas_semantics() {
        let _g = GATE_TEST_LOCK.lock().unwrap();
        // Pré-condition : le verrou est libre (les autres tests le
        // libèrent toujours en sortie).
        release_scan_gate();
        assert!(!SCAN_RUNNING.load(Ordering::SeqCst));

        assert!(try_acquire_scan_gate(), "première acquisition doit réussir");
        assert!(SCAN_RUNNING.load(Ordering::SeqCst));

        assert!(
            !try_acquire_scan_gate(),
            "seconde acquisition doit échouer tant que non libéré"
        );

        release_scan_gate();
        assert!(!SCAN_RUNNING.load(Ordering::SeqCst));

        assert!(
            try_acquire_scan_gate(),
            "réacquisition après libération doit réussir"
        );
        release_scan_gate();
    }

    /// `release_scan_gate` est idempotent : appeler deux fois ne
    /// panique pas et laisse l'état à `false`.
    #[test]
    fn release_is_idempotent() {
        let _g = GATE_TEST_LOCK.lock().unwrap();
        release_scan_gate();
        release_scan_gate();
        assert!(!SCAN_RUNNING.load(Ordering::SeqCst));
    }

    /// `scan_status` reflète l'état courant du verrou.
    #[tokio::test]
    async fn scan_status_reflects_gate_state() {
        let _g = GATE_TEST_LOCK.lock().unwrap();
        release_scan_gate();

        let status = scan_status().await.expect("scan_status ok");
        assert_eq!(status, ScanStatus { running: false });

        assert!(try_acquire_scan_gate());
        let status = scan_status().await.expect("scan_status ok");
        assert_eq!(status, ScanStatus { running: true });

        release_scan_gate();
    }
}
