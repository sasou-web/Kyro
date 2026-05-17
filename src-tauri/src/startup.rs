//! Tâche 8.5 — hook `scan_at_startup` (requirement 3.7).
//!
//! Au boot de l'application, après l'init du pool SQLite et l'application
//! des migrations (cf. `main.rs::setup`), on lit la valeur de la clé
//! `setting_value(key='scan_at_startup')`. Si elle est *truthy*
//! (cf. [`is_truthy`]), on lance un scan complet **asynchrone et
//! non-bloquant** : le `setup` Tauri continue immédiatement (la fenêtre
//! principale reste réactive), tandis qu'un `tokio::task` en arrière-plan
//! itère sur tous les `library_folder` connus et exécute
//! `scanner::scan_folder` pour chacun.
//!
//! ## Garanties
//!
//! 1. **Non-blocking** : la fonction publique
//!    [`maybe_run_startup_scan`] est synchrone et retourne immédiatement
//!    après avoir programmé le `spawn`. L'init Tauri n'est donc jamais
//!    retardée par un scan FS potentiellement long (requirement 3.7 +
//!    design § « Cycle de vie démarrage »).
//! 2. **Non-fatale** : aucune erreur côté scan ne fait paniquer
//!    l'application. Les échecs par-dossier (FS injoignable, droits
//!    refusés, dossier supprimé entre temps, etc.) sont **journalisés**
//!    via `tracing::error!`/`warn!` et le scan continue avec le dossier
//!    suivant. Les `ScanReport.failures` (échecs par-fichier déjà
//!    accumulés par `scan_folder`) sont également journalisés.
//! 3. **Production-ready** : on utilise [`FfprobeBackend`] (le backend
//!    réel branché sur le binaire ffprobe résolu par
//!    [`crate::infra::ffprobe_locator`]). Si ffprobe est introuvable au
//!    moment du scan de démarrage, on logue une erreur et on **abandonne
//!    silencieusement** le scan plutôt que de planter le boot — l'UI
//!    Settings exposera de toute façon un bouton « scan » qui retentera
//!    avec retour utilisateur.
//!
//! ## Sémantique « truthy »
//!
//! La table `setting_value` stocke des `TEXT`. Le frontend / les commandes
//! Tauri sont libres d'écrire `"1"`, `"true"`, `"True"`, etc. Pour rester
//! tolérant aux variations sans accepter de valeurs ambiguës, on retient
//! exactement deux littéraux (insensibles à la casse, après trim) :
//!
//! - `"1"` → vrai
//! - `"true"` → vrai
//!
//! Tout le reste — `"0"`, `"false"`, `""`, valeurs absentes (`None`),
//! `"yes"`, `"on"`, espaces seuls, etc. — est traité comme **faux**. Cette
//! liste limitative protège contre les erreurs de typage côté frontend
//! (un `"undefined"` sérialisé tel quel ne déclenchera pas de scan).

use crate::infra::ffprobe_locator::locate_ffprobe;
use crate::infra::probe::FfprobeBackend;
use crate::infra::repo::Repo;
use crate::infra::scanner;
use crate::infra::settings_keys::{
    SETTING_KEY_FFPROBE_PATH, SETTING_KEY_SCAN_AT_STARTUP,
};

/// Indique si la valeur stockée dans `setting_value(scan_at_startup)`
/// doit être interprétée comme « vrai ».
///
/// Comparaison **insensible à la casse** et **trim**ée des espaces ;
/// seules les valeurs `"1"` et `"true"` sont retenues. Voir doc du
/// module pour la justification de cette liste limitative.
pub fn is_truthy(value: Option<&str>) -> bool {
    let Some(raw) = value else {
        return false;
    };
    let trimmed = raw.trim();
    trimmed.eq_ignore_ascii_case("1") || trimmed.eq_ignore_ascii_case("true")
}

/// Programme — sans bloquer — un scan complet asynchrone si la clé
/// `scan_at_startup` est *truthy* (cf. [`is_truthy`]).
///
/// La fonction est **synchrone** : elle retourne immédiatement après
/// avoir appelé `tauri::async_runtime::spawn`. Le scan effectif (lecture
/// du setting, résolution du binaire ffprobe, parcours des dossiers) a
/// lieu dans la task spawnée.
///
/// Aucune erreur n'est remontée à l'appelant : le contrat avec
/// `main.rs::setup` est de **ne jamais bloquer ni faire échouer le
/// boot**. Toutes les erreurs sont journalisées.
pub fn maybe_run_startup_scan(repo: Repo) {
    tauri::async_runtime::spawn(async move {
        run_startup_scan_inner(repo).await;
    });
}

/// Implémentation interne du hook : lit le setting, résout ffprobe,
/// itère sur les `library_folder` et exécute `scan_folder` sur chacun.
///
/// Aucune erreur ne propage : tout est journalisé via `tracing`.
async fn run_startup_scan_inner(repo: Repo) {
    // 1. Lire `setting_value(scan_at_startup)`. Une erreur de lecture
    //    (DB inaccessible, transaction bloquée) est logée mais on
    //    abandonne sans paniquer — le boot reste opérationnel et
    //    l'utilisateur pourra retenter manuellement depuis l'UI.
    let raw = match repo.get_setting(SETTING_KEY_SCAN_AT_STARTUP).await {
        Ok(v) => v,
        Err(err) => {
            tracing::error!(
                error = %err,
                "Kyro startup: lecture du setting scan_at_startup échouée — \
                 scan au démarrage abandonné"
            );
            return;
        }
    };

    if !is_truthy(raw.as_deref()) {
        tracing::debug!(
            value = ?raw,
            "Kyro startup: scan_at_startup non truthy — pas de scan au démarrage"
        );
        return;
    }

    tracing::info!("Kyro startup: scan_at_startup actif — lancement du scan asynchrone");

    // 2. Résoudre le binaire ffprobe. On lit d'abord la clé settings
    //    `ffprobe_path` (priorité 2 du locator) puis on délègue au
    //    locator qui essaiera sidecar → settings → PATH système. Si
    //    aucun candidat n'est valide, on abandonne le scan en logant
    //    plutôt que de persister des items sans champs techniques (le
    //    scanner accepte cette dégradation, mais au démarrage automatique
    //    il vaut mieux ne rien faire que d'inonder la base de fiches
    //    incomplètes).
    let ffprobe_setting = match repo.get_setting(SETTING_KEY_FFPROBE_PATH).await {
        Ok(v) => v,
        Err(err) => {
            tracing::warn!(
                error = %err,
                "Kyro startup: lecture du setting ffprobe_path échouée — \
                 fallback sur sidecar/PATH"
            );
            None
        }
    };

    // `locate_ffprobe` prend un `FnOnce(&str) -> Option<String>` ; on le
    // satisfait avec une clôture qui ne répond positivement qu'à la clé
    // ffprobe_path (cohérent avec la doc du locator).
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
                "Kyro startup: ffprobe introuvable — scan au démarrage abandonné"
            );
            return;
        }
    };
    let probe = FfprobeBackend::new(ff_path);

    // 3. Lister les dossiers de bibliothèque. Une erreur ici interrompt
    //    le scan (rien à scanner sans la liste) mais n'affecte pas
    //    l'application.
    let folders = match repo.list_library_folders().await {
        Ok(f) => f,
        Err(err) => {
            tracing::error!(
                error = %err,
                "Kyro startup: list_library_folders a échoué — scan au démarrage \
                 abandonné"
            );
            return;
        }
    };

    if folders.is_empty() {
        tracing::info!(
            "Kyro startup: aucun library_folder configuré — scan au démarrage \
             terminé (rien à faire)"
        );
        return;
    }

    tracing::info!(
        folders_count = folders.len(),
        "Kyro startup: scan au démarrage démarré"
    );

    // 4. Itération séquentielle. Le requirement clé est que le **boot**
    //    ne soit pas bloqué — pas que les dossiers soient parcourus en
    //    parallèle. Une exécution séquentielle évite la contention sur
    //    le pool SQLite (max_connections=5) et sur le binaire ffprobe
    //    (un seul process à la fois → moins de pression CPU/IO disque).
    let mut total_processed: usize = 0;
    let mut total_created: usize = 0;
    let mut total_updated: usize = 0;
    let mut total_marked_unavailable: usize = 0;
    let mut total_failures: usize = 0;
    let mut folder_errors: usize = 0;

    for folder in folders {
        let path = std::path::Path::new(&folder.path);
        match scanner::scan_folder(&repo, &probe, folder.id, path, folder.category).await {
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
                    "Kyro startup: scan dossier terminé"
                );
                total_processed += report.processed;
                total_created += report.created;
                total_updated += report.updated;
                total_marked_unavailable += report.marked_unavailable;
                total_failures += report.failures.len();
                // Les `ScanReport.failures` ont déjà été logés par
                // `scan_folder` au niveau warn ; on les recompte ici
                // uniquement pour le résumé final.
            }
            Err(err) => {
                folder_errors += 1;
                tracing::error!(
                    folder_id = folder.id.get(),
                    folder_path = %folder.path,
                    error = %err,
                    "Kyro startup: scan dossier échoué — les autres dossiers \
                     continuent"
                );
            }
        }
    }

    tracing::info!(
        processed = total_processed,
        created = total_created,
        updated = total_updated,
        marked_unavailable = total_marked_unavailable,
        failures = total_failures,
        folder_errors = folder_errors,
        "Kyro startup: scan au démarrage terminé"
    );
}

#[cfg(test)]
mod tests {
    //! Couverture unitaire du parser `is_truthy`.
    //!
    //! Le hook lui-même (résolution ffprobe + spawn de la task de scan)
    //! est validé par les tests d'intégration du scanner (tâches 8.6 →
    //! 8.9) ; on ne peut pas le tester ici sans démarrer un runtime
    //! Tauri. Le seul invariant testable de manière isolée est la
    //! sémantique de `is_truthy`, qui détermine quand le hook se
    //! déclenche.

    use super::is_truthy;

    #[test]
    fn truthy_recognises_one_and_true_case_insensitive() {
        for v in ["1", "true", "True", "TRUE", "TrUe"] {
            assert!(is_truthy(Some(v)), "{v:?} doit être truthy");
        }
    }

    #[test]
    fn truthy_trims_surrounding_whitespace() {
        for v in [" 1 ", "  true", "true\n", "\t1\t"] {
            assert!(is_truthy(Some(v)), "{v:?} doit être truthy après trim");
        }
    }

    #[test]
    fn falsy_values_are_rejected() {
        for v in ["0", "false", "FALSE", "False", "no", "off", "yes", "on", "2"] {
            assert!(!is_truthy(Some(v)), "{v:?} ne doit pas être truthy");
        }
    }

    #[test]
    fn empty_or_whitespace_only_is_falsy() {
        for v in ["", " ", "\t", "\n", "   "] {
            assert!(!is_truthy(Some(v)), "{v:?} ne doit pas être truthy");
        }
    }

    #[test]
    fn none_value_is_falsy() {
        assert!(!is_truthy(None));
    }
}
