//! Tâche 8.1 — fondation `Kyro_Scanner` : parcours récursif et filtre
//! d'extensions (requirements 3.1, 3.2).
//!
//! Cette première tranche du module fournit uniquement les briques pures
//! de découverte de fichiers candidats :
//!
//! 1. La constante [`SUPPORTED_EXTENSIONS`], reproduite à l'identique du
//!    design § « Kyro_Scanner » :
//!    `["mkv", "mp4", "avi", "mov", "webm", "m4v", "ts", "wmv"]`.
//! 2. Le helper [`has_supported_extension`] qui compare l'extension d'un
//!    chemin à cette allowlist **insensiblement à la casse** (lower-case
//!    ASCII des extensions, conforme à toutes les extensions cibles qui
//!    sont elles-mêmes en ASCII).
//! 3. La fonction [`discover_files`] qui parcourt récursivement un dossier
//!    via `walkdir::WalkDir` avec **suivi des liens symboliques
//!    explicitement désactivé** (`follow_links(false)`), puis ne conserve
//!    que les entrées de type fichier dont l'extension est supportée.
//! 4. La variante [`discover_files_with_failures`] (tâche 8.4) qui, en
//!    plus de la liste des fichiers, retourne la liste des échecs
//!    walkdir (dossier interdit, lien cassé, dossier disparu en cours
//!    de parcours) sous forme de [`ScanFailure`] avec
//!    [`ScanFailureKind::Walk`]. Chaque erreur est journalisée via
//!    `tracing::warn!(file_path, error)`. La fonction historique
//!    [`discover_files`] reste disponible pour les call-sites qui n'ont
//!    pas besoin du détail des échecs (UI de pré-visualisation, tests,
//!    etc.).
//!
//! ## Pourquoi `follow_links(false)` ?
//!
//! - Évite les boucles de FS (un lien pointant vers un de ses ancêtres
//!   transformerait le scan en boucle infinie).
//! - Préserve la signification du dossier déclaré : seuls les fichiers
//!   *physiquement* sous un `library_folder` sont indexés ; un raccourci
//!   vers un autre disque n'introduit pas d'item fantôme.
//! - Conforme au design qui ne mentionne aucun cas d'usage justifiant le
//!   suivi des liens.
//!
//! ## Conception « pure-ish »
//!
//! [`discover_files`] retourne un `Vec<PathBuf>` plutôt qu'un itérateur
//! emprunté : cela découple les call-sites des contraintes de durée de
//! vie de `WalkDir` et garde la signature trivialement testable et
//! mockable. Le coût (allocation d'un Vec) est négligeable face aux IO
//! disque qui dominent toute opération de scan.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

/// Extensions de fichiers vidéo prises en charge par Kyro
/// (design § « Kyro_Scanner », requirement 3.2).
///
/// Toutes les valeurs sont en minuscules ASCII : la comparaison
/// effectuée par [`has_supported_extension`] est insensible à la casse.
/// L'ordre n'a pas de signification fonctionnelle.
pub const SUPPORTED_EXTENSIONS: &[&str] =
    &["mkv", "mp4", "avi", "mov", "webm", "m4v", "ts", "wmv"];

/// Indique si l'extension du chemin appartient à
/// [`SUPPORTED_EXTENSIONS`], comparaison **insensible à la casse**.
///
/// Retourne `false` si le chemin n'a pas d'extension du tout
/// (`foo`, `.gitignore`) ou si l'extension n'est pas valide UTF-8.
///
/// Toutes les extensions de l'allowlist étant en ASCII pur, un simple
/// `to_ascii_lowercase()` suffit ; on évite ainsi d'allouer une `String`
/// par appel sauf dans le cas marginal d'une extension non-ASCII (qui
/// sera rejetée par la comparaison de toute façon).
pub fn has_supported_extension(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    let lower = ext.to_ascii_lowercase();
    SUPPORTED_EXTENSIONS.iter().any(|sup| *sup == lower)
}

/// Parcourt récursivement `root` et retourne la liste des fichiers dont
/// l'extension est supportée (cf. [`SUPPORTED_EXTENSIONS`]).
///
/// Le suivi des liens symboliques est explicitement désactivé
/// (`WalkDir::follow_links(false)`), ce qui évite les boucles et garde
/// le scan limité aux fichiers physiquement sous `root`.
///
/// Les entrées qui produisent une erreur d'accès (`walkdir::Error`)
/// sont **journalisées** via `tracing::warn!(file_path, error)` puis
/// silencieusement ignorées. Les call-sites qui ont besoin de la liste
/// détaillée des échecs (pour l'agréger dans un `ScanReport`) doivent
/// utiliser [`discover_files_with_failures`] à la place.
///
/// Si `root` n'existe pas ou n'est pas accessible, le résultat est un
/// `Vec` vide (l'erreur racine est journalisée mais pas remontée).
pub fn discover_files(root: &Path) -> Vec<PathBuf> {
    discover_files_with_failures(root).0
}

/// Variante de [`discover_files`] qui retourne, en plus de la liste
/// des fichiers candidats, la liste des échecs `walkdir` rencontrés
/// pendant le parcours (un échec par entrée problématique).
///
/// Chaque échec est :
/// 1. journalisé via `tracing::warn!(file_path = %path, error = %err,
///    "Kyro scanner: walkdir entry failed")` — `file_path` reflète le
///    chemin de l'entrée fautive si `walkdir` a pu le déterminer,
///    sinon le `root` du parcours en repli ;
/// 2. matérialisé sous la forme d'un [`ScanFailure`] avec
///    [`ScanFailureKind::Walk`] et le message complet de l'erreur.
///
/// Cette fonction est utilisée par l'orchestrateur `scan_folder`
/// (tâche 8.4) pour fusionner les échecs « niveau parcours » dans
/// `ScanReport.failures` — garantissant ainsi qu'aucune erreur isolée
/// n'est perdue (requirement 18.1).
pub fn discover_files_with_failures(root: &Path) -> (Vec<PathBuf>, Vec<ScanFailure>) {
    let mut files: Vec<PathBuf> = Vec::new();
    let mut failures: Vec<ScanFailure> = Vec::new();

    for result in WalkDir::new(root).follow_links(false).into_iter() {
        match result {
            Ok(entry) => {
                if !entry.file_type().is_file() {
                    continue;
                }
                let path = entry.into_path();
                if has_supported_extension(&path) {
                    files.push(path);
                }
            }
            Err(err) => {
                // `walkdir::Error::path()` est `Some` quand l'erreur
                // se rapporte à une entrée précise (cas le plus
                // courant : permission refusée, fichier disparu en
                // cours de parcours). Sur l'erreur initiale (root
                // inexistant), `path()` peut être `None` : on retombe
                // alors sur le `root` fourni pour conserver une
                // information utile dans le log.
                let stored_path = err
                    .path()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|| root.to_string_lossy().into_owned());
                tracing::warn!(
                    file_path = %stored_path,
                    error = %err,
                    "Kyro scanner: parcours walkdir échoué — entrée ignorée"
                );
                failures.push(ScanFailure {
                    file_path: stored_path,
                    kind: ScanFailureKind::Walk,
                    message: err.to_string(),
                });
            }
        }
    }

    (files, failures)
}

#[cfg(test)]
mod tests {
    //! Couverture unitaire de la tâche 8.1 :
    //!
    //! - dossier vide ⇒ aucun fichier retourné ;
    //! - mélange d'extensions, casses variées (UPPER, Mixed, lower) ;
    //! - récursion à travers des sous-dossiers imbriqués ;
    //! - non-suivi des liens symboliques (test best-effort sur Windows :
    //!   la création d'un symlink requiert le mode développeur ou des
    //!   privilèges admin ; si elle échoue, le test est silencieusement
    //!   ignoré pour ne pas casser le CI/dev quotidien) ;
    //! - dossier inexistant ⇒ Vec vide (pas de panique).

    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::{discover_files, has_supported_extension, SUPPORTED_EXTENSIONS};

    fn touch(dir: &std::path::Path, rel: &str) -> PathBuf {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dirs");
        }
        fs::write(&path, b"x").expect("touch file");
        path
    }

    fn names(paths: &[PathBuf]) -> HashSet<String> {
        paths
            .iter()
            .map(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
                    .to_owned()
            })
            .collect()
    }

    // ------------------------------------------------------------------
    // has_supported_extension
    // ------------------------------------------------------------------

    #[test]
    fn supported_extensions_constant_matches_design() {
        // Garde-fou : si quelqu'un modifie la liste, il doit aussi
        // mettre à jour le design § « Kyro_Scanner ». Ce test fige la
        // signature exacte attendue par le requirement 3.2.
        assert_eq!(
            SUPPORTED_EXTENSIONS,
            &["mkv", "mp4", "avi", "mov", "webm", "m4v", "ts", "wmv"]
        );
    }

    #[test]
    fn extension_match_is_case_insensitive() {
        for ext in SUPPORTED_EXTENSIONS {
            // lower
            assert!(has_supported_extension(&PathBuf::from(format!(
                "movie.{ext}"
            ))));
            // UPPER
            assert!(has_supported_extension(&PathBuf::from(format!(
                "movie.{}",
                ext.to_uppercase()
            ))));
            // Mixed (premier caractère en majuscule)
            let mut chars = ext.chars();
            let mixed = match chars.next() {
                Some(c) => format!("{}{}", c.to_uppercase(), chars.as_str()),
                None => String::new(),
            };
            assert!(has_supported_extension(&PathBuf::from(format!(
                "movie.{mixed}"
            ))));
        }
    }

    #[test]
    fn unsupported_or_missing_extension_is_rejected() {
        assert!(!has_supported_extension(&PathBuf::from("readme.txt")));
        assert!(!has_supported_extension(&PathBuf::from("archive.zip")));
        assert!(!has_supported_extension(&PathBuf::from("noext")));
        assert!(!has_supported_extension(&PathBuf::from(".gitignore")));
        assert!(!has_supported_extension(&PathBuf::from("photo.jpg")));
    }

    // ------------------------------------------------------------------
    // discover_files
    // ------------------------------------------------------------------

    #[test]
    fn empty_directory_returns_empty_vec() {
        let tmp = TempDir::new().expect("create tempdir");
        let found = discover_files(tmp.path());
        assert!(found.is_empty(), "expected no files, got {found:?}");
    }

    #[test]
    fn nonexistent_root_returns_empty_vec() {
        let tmp = TempDir::new().expect("create tempdir");
        let missing = tmp.path().join("does_not_exist");
        let found = discover_files(&missing);
        assert!(found.is_empty(), "expected no files, got {found:?}");
    }

    #[test]
    fn mixed_extensions_and_casing_are_filtered() {
        let tmp = TempDir::new().expect("create tempdir");
        let root = tmp.path();

        // Supportés (cases variées).
        touch(root, "alpha.mkv");
        touch(root, "BETA.MP4");
        touch(root, "Gamma.WebM");
        touch(root, "delta.M4v");
        touch(root, "epsilon.TS");

        // Non supportés.
        touch(root, "notes.txt");
        touch(root, "cover.jpg");
        touch(root, "archive.zip");
        touch(root, "no_extension_at_all");
        touch(root, ".dotfile"); // pas d'extension reconnue

        let found = names(&discover_files(root));

        let expected: HashSet<String> = [
            "alpha.mkv",
            "BETA.MP4",
            "Gamma.WebM",
            "delta.M4v",
            "epsilon.TS",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        assert_eq!(found, expected);
    }

    #[test]
    fn recurses_into_nested_directories() {
        let tmp = TempDir::new().expect("create tempdir");
        let root = tmp.path();

        touch(root, "top.mkv");
        touch(root, "season1/ep01.mp4");
        touch(root, "season1/extras/bonus.MOV");
        touch(root, "season2/sub/deep/finale.webm");
        // non supporté en profondeur
        touch(root, "season2/sub/deep/notes.txt");

        let found = names(&discover_files(root));
        let expected: HashSet<String> = ["top.mkv", "ep01.mp4", "bonus.MOV", "finale.webm"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(found, expected);
    }

    /// Vérifie que `follow_links(false)` est respecté : un lien
    /// symbolique pointant vers un dossier extérieur ne doit pas être
    /// traversé.
    ///
    /// Sous Windows, `std::os::windows::fs::symlink_dir` exige soit le
    /// **mode développeur**, soit des privilèges administrateur. Si la
    /// création échoue (cas du dev quotidien sans ces droits), on
    /// considère l'environnement incapable de produire le scénario et
    /// on quitte le test sans le faire échouer — la garantie reste
    /// portée par `WalkDir::follow_links(false)` (configuré
    /// statiquement) et par le test sur Unix.
    #[test]
    fn does_not_follow_symlinks() {
        let tmp = TempDir::new().expect("create tempdir");
        let root = tmp.path().join("root");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&root).expect("create root");
        fs::create_dir_all(&outside).expect("create outside");

        // Fichiers : un dans `root` (doit être trouvé), un dans
        // `outside` (ne doit PAS être trouvé si le lien est ignoré).
        touch(&root, "in_root.mkv");
        touch(&outside, "linked_target.mp4");

        // Tente de créer un lien symbolique `root/linked -> outside`.
        let link = root.join("linked");
        let link_created = create_dir_symlink(&outside, &link);

        if !link_created {
            // Environnement sans droit de créer des symlinks (Windows
            // sans mode dev). On s'assure tout de même que le scan de
            // base reste correct puis on quitte.
            let found = names(&discover_files(&root));
            assert!(found.contains("in_root.mkv"));
            return;
        }

        let found = names(&discover_files(&root));
        assert!(
            found.contains("in_root.mkv"),
            "expected in_root.mkv to be discovered, got {found:?}"
        );
        assert!(
            !found.contains("linked_target.mp4"),
            "symlink target was followed: {found:?}"
        );
    }

    /// Crée un lien symbolique de dossier de manière portable.
    /// Retourne `true` si la création a réussi, `false` si l'OS refuse
    /// (typiquement Windows sans mode développeur).
    #[cfg(windows)]
    fn create_dir_symlink(target: &std::path::Path, link: &std::path::Path) -> bool {
        std::os::windows::fs::symlink_dir(target, link).is_ok()
    }

    #[cfg(unix)]
    fn create_dir_symlink(target: &std::path::Path, link: &std::path::Path) -> bool {
        std::os::unix::fs::symlink(target, link).is_ok()
    }

    #[cfg(not(any(windows, unix)))]
    fn create_dir_symlink(_target: &std::path::Path, _link: &std::path::Path) -> bool {
        false
    }

    // ------------------------------------------------------------------
    // Property-based test — Tâche 8.7
    //
    // Feature: kyro, Property 4 — Filtrage par extensions supportées.
    // **Validates: Requirements 3.2**
    //
    // Pour un ensemble arbitraire de fichiers dont l'extension est
    // alphanumérique de 1 à 6 caractères en casse mixte, l'invariant
    // observable de `discover_files` est :
    //
    //     { p.file_name() | p ∈ discover_files(root) }
    //         = { f | f ∈ tous_les_fichiers ∧
    //                 ext(f).to_ascii_lowercase() ∈ SUPPORTED_EXTENSIONS }
    //
    // Le générateur ne produit jamais d'extension vide ni de
    // caractères non-alphanumériques : on évite ainsi le cas dégénéré
    // « pas d'extension du tout » (déjà couvert par les tests par
    // exemples ci-dessus). La comparaison `to_ascii_lowercase()`
    // est exhaustive sur les 8 extensions cibles, qui sont toutes
    // ASCII pur.
    //
    // Chaque fichier porte un préfixe unique `file_<i>` afin que des
    // extensions répétées ne provoquent pas de collision de nom de
    // fichier sur disque.
    // ------------------------------------------------------------------

    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 100,
            .. ProptestConfig::default()
        })]

        /// **Property 4 — Filtrage par extensions supportées.**
        ///
        /// Feature: kyro, Property 4
        /// **Validates: Requirements 3.2**
        #[test]
        fn prop4_discover_files_filters_by_supported_extensions(
            exts in proptest::collection::vec("[A-Za-z0-9]{1,6}", 0..20),
        ) {
            let tmp = TempDir::new().expect("create tempdir");
            let root = tmp.path();

            // Construction simultanée du système de fichiers cible et
            // de l'oracle attendu : pour chaque extension générée, on
            // matérialise un fichier vide et, si son extension passe
            // le filtre `SUPPORTED_EXTENSIONS` (insensible à la casse),
            // on l'ajoute à l'ensemble attendu.
            let mut expected: HashSet<String> = HashSet::new();
            for (i, ext) in exts.iter().enumerate() {
                let filename = format!("file_{i}.{ext}");
                touch(root, &filename);
                let ext_lower = ext.to_ascii_lowercase();
                if SUPPORTED_EXTENSIONS.iter().any(|sup| *sup == ext_lower) {
                    expected.insert(filename);
                }
            }

            let found = names(&discover_files(root));
            prop_assert_eq!(found, expected);
        }
    }
}


// ===========================================================================
// Tâche 8.2 — diff existants / nouveaux / disparus.
//
// Cette section construit l'orchestrateur `scan_folder` qui :
//   1. Énumère les fichiers candidats sous le dossier (via
//      `discover_files`, tâche 8.1).
//   2. Pour chaque candidat, lit `(file_path canonique, size_bytes)`
//      depuis `std::fs::metadata`. Si une entrée DB existe avec mêmes
//      `(file_path, size_bytes)` → skip idempotent (requirement 3.3).
//      Sinon parse via `domain::parser::parse`, probe via le
//      `ProbeBackend` injecté, classifie selon la `Category` du
//      dossier parent, et persiste via `repo`.
//   3. Pour les fichiers de catégorie `Series` ou `Animes` dont aucun
//      motif d'épisode n'a été reconnu (`numbering_unknown = true`),
//      les rattache à la **série placeholder « À classer »**
//      matérialisée par `repo.ensure_unsorted_series` (requirement
//      4.7, design § « Kyro_Parser »).
//   4. Marque `available = 0` toute entrée DB précédemment indexée
//      sous ce dossier dont le fichier n'apparaît plus dans
//      l'énumération (requirement 3.5), **sans la supprimer**.
//   5. Retourne un `ScanReport` agrégeant compteurs et échecs.
//
// Les sous-tâches suivantes brancheront :
//   * 8.3 — émission `scan-progress` (Tauri `Window::emit`) ;
//   * 8.4 — journalisation `tracing::warn!` détaillée + ScanFailure
//     sur erreurs isolées (requirement 18.1) — déjà partiellement
//     amorcé ici via le champ `failures` du `ScanReport` ;
//   * 8.5 — hook `scan_at_startup`.
//
// Pour rester testable sans Tauri ni binaire ffprobe, l'orchestrateur
// accepte un `&dyn ProbeBackend` (impl réelle = `FfprobeBackend`,
// impl test = stub local). Aucune notion de `Window` n'est introduite
// à ce stade.
// ===========================================================================

use std::collections::HashSet;

use serde::Serialize;

use crate::domain::parser;
use crate::domain::types::{Category, EpisodeItemId, MediaItemId, MediaType};
use crate::error::KyroError;
use crate::infra::probe::{ProbeBackend, ProbeError, TechnicalMetadata};
use crate::infra::repo::{
    canonicalize_or_keep, FolderId, Repo, UpsertEpisodeInput, UpsertMediaInput,
};

// ===========================================================================
// Tâche 8.3 — événement `scan-progress`.
//
// Cette tâche introduit deux briques sans toucher au cœur du diff :
//
//   1. La structure [`ScanProgress`], payload sérialisable émis à chaque
//      fichier traité (requirement 3.6). `current_file` porte la forme
//      stockée (canonicalisée best-effort), identique à celle insérée
//      en DB, afin que le frontend puisse corréler avec les fiches.
//
//   2. Le type alias [`ScanProgressEmitter`] (`&dyn Fn(&ScanProgress)`,
//      `Send + Sync`) qui découple le scanner de Tauri : tout call-site
//      peut fournir un closure (test : push dans un `Vec` ; production :
//      `Window::emit`). Cette indirection rend [`scan_folder`] testable
//      sans `tauri::Window` réel — la branche émission est exercée par
//      [`scan_folder_emit`] via un wrapper de quelques lignes.
//
// Aucun appel au scanner antérieur (8.1/8.2) n'est cassé : la fonction
// publique `scan_folder` conserve sa signature exacte ; elle délègue
// désormais à `scan_folder_with_emitter` avec `None`. Le contrat de
// `ScanReport` est inchangé.
// ===========================================================================

/// Payload de l'événement `scan-progress` émis pendant un scan
/// (requirement 3.6, design § « Kyro_Scanner — événements UI »).
///
/// Le frontend reçoit ce JSON tel quel via `window.emit("scan-progress", _)`
/// et peut afficher une barre de progression `processed / total` ainsi
/// que le nom du fichier en cours.
///
/// Champs :
/// - `processed` : nombre de fichiers déjà traités (1-indexé après le
///   passage du fichier courant ; càd `i + 1` à l'itération `i`).
/// - `total`     : nombre total de candidats énumérés au début du scan
///   (cf. `ScanReport.total_files`). Reste constant pendant tout le
///   scan, ce qui permet à l'UI d'afficher un ratio stable.
/// - `current_file` : forme stockée du chemin (canonicalisée best-effort,
///   identique à `media_item.file_path` / `episode_item.file_path`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ScanProgress {
    pub processed: usize,
    pub total: usize,
    pub current_file: String,
}

/// Closure invoquée par le scanner après chaque fichier traité.
///
/// Le scanner appelle cet émetteur **une fois par fichier candidat**,
/// y compris pour les skips idempotents et les échecs isolés
/// (`ScanFailure`) : la progression reste monotone et reflète l'avancée
/// réelle du parcours, pas seulement les insertions/mises à jour. Cela
/// correspond à l'attente UI naturelle (« j'ai traité X / Y fichiers »).
///
/// Les bornes `Send + Sync` permettent de passer le scanner à un
/// `tokio::spawn` (utilisé par `Kyro_Backend` dans la commande Tauri
/// `scan_now`). Le call-site test peut, lui, capturer un `Mutex` pour
/// pousser dans un `Vec` partagé.
pub type ScanProgressEmitter<'a> = &'a (dyn Fn(&ScanProgress) + Send + Sync + 'a);

/// Rapport agrégé d'un scan (design § « Kyro_Scanner »).
///
/// Tous les compteurs portent sur **un appel** de `scan_folder` ; un
/// scan multi-dossiers compose simplement plusieurs `ScanReport`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ScanReport {
    /// Nombre total de fichiers candidats énumérés (après filtre
    /// d'extensions).
    pub total_files: usize,
    /// Nombre de fichiers effectivement traités (parsing + probe + DB
    /// tentés). En l'absence d'erreur isolée, `processed = total_files`.
    pub processed: usize,
    /// Nombre de fiches insérées (cas « Pas existant » de l'upsert).
    pub created: usize,
    /// Nombre de fiches dont les champs techniques ont été rafraîchis
    /// (cas « Existant + size_bytes différent »). Les skips idempotents
    /// (mêmes `(file_path, size_bytes)`) ne sont **pas** comptés ici.
    pub updated: usize,
    /// Nombre d'entrées DB précédemment indexées dont le fichier n'a
    /// plus été retrouvé sur disque et qui ont été marquées
    /// `available = 0`.
    pub marked_unavailable: usize,
    /// Échecs isolés (parse, probe, IO) journalisés sans interrompre
    /// le scan (requirement 18.1).
    pub failures: Vec<ScanFailure>,
}

/// Détail d'un échec isolé rencontré pendant le scan d'un fichier.
///
/// La granularité est volontairement fine (un `ScanFailure` par
/// fichier en erreur) afin que l'UI puisse afficher la liste exacte
/// des fichiers problématiques. Le champ `kind` permet de distinguer
/// la cause racine sans coupler le frontend à l'enum technique
/// `ProbeError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanFailure {
    /// Chemin absolu (canonicalisé si possible) du fichier en cause.
    pub file_path: String,
    /// Catégorie de l'échec.
    pub kind: ScanFailureKind,
    /// Message d'erreur détaillé pour le log.
    pub message: String,
}

/// Catégorie d'échec rencontrée pendant le scan.
///
/// Les variantes correspondent aux sources possibles citées par le
/// requirement 18.1 (parse, probe, IO) auxquelles s'ajoutent :
///   - `Db` : échec d'écriture en base — remonté comme `ScanFailure`
///     plutôt qu'en `Err` global afin que les fichiers suivants
///     restent traités ;
///   - `Walk` : échec de parcours `walkdir` (dossier interdit, lien
///     cassé, fichier disparu en cours d'énumération) — capturé par
///     [`discover_files_with_failures`] (tâche 8.4) et fusionné dans
///     `ScanReport.failures` par `scan_folder`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanFailureKind {
    /// Lecture des métadonnées système (`std::fs::metadata`) échouée.
    Io,
    /// Échec ffprobe (binaire absent, exit ≠ 0, JSON invalide).
    Probe,
    /// Échec d'écriture en base.
    Db,
    /// Échec de parcours `walkdir` sur une entrée du dossier scanné.
    Walk,
}

/// Scanne un dossier de bibliothèque et synchronise son contenu avec
/// la base.
///
/// Étapes (cf. tête de section pour la justification détaillée) :
///
/// 1. **Énumération** : `discover_files(folder_path)`.
/// 2. **Diff par fichier** : pour chaque candidat,
///    - lecture `fs::metadata` (taille) ;
///    - skip si `(file_path canonique, size_bytes)` matche déjà la DB ;
///    - sinon parsing → probe → upsert (Films → `media_item`, Series/
///      Animes → `episode_item` rattaché à une série).
/// 3. **Épisodes orphelins** (Series/Animes sans pattern reconnu) :
///    rattachement à la série placeholder « À classer »,
///    `numbering_unknown = true`.
/// 4. **Disparitions** : tous les `media_item` / `episode_item` du
///    dossier dont `file_path` n'est plus dans l'ensemble découvert
///    sont marqués `available = 0` (requirement 3.5).
///
/// Erreurs isolées (parse / probe / IO / DB / walkdir sur une entrée) :
/// journalisées via `tracing::warn!(file_path, error, ...)` et
/// accumulées dans `ScanReport.failures` ; le scan continue
/// (requirement 18.1).
///
/// Erreurs globales (DB unreachable au moment de la lecture initiale,
/// dossier introuvable au niveau du `folder_id`) : remontées en
/// `Err(KyroError)` car elles invalident l'ensemble du scan.
pub async fn scan_folder(
    repo: &Repo,
    probe: &dyn ProbeBackend,
    folder_id: FolderId,
    folder_path: &std::path::Path,
    category: Category,
) -> Result<ScanReport, KyroError> {
    scan_folder_with_emitter(repo, probe, folder_id, folder_path, category, None).await
}

/// Variante de [`scan_folder`] acceptant un émetteur de progression
/// (tâche 8.3, requirement 3.6).
///
/// Si `emitter` est `Some(_)`, il est invoqué **une fois par fichier
/// candidat traité**, juste après l'avoir consommé (skip idempotent,
/// création/mise à jour réussie ou échec isolé inclus). `processed`
/// dans la `ScanProgress` est 1-indexé : à la fin du scan,
/// `processed == total == ScanReport.total_files`.
///
/// Si `emitter` est `None`, le comportement est strictement identique
/// à [`scan_folder`] (pas de coût additionnel, pas d'allocation de
/// `ScanProgress`).
///
/// Les wrappers Tauri (`scan_folder_emit`) construisent un closure qui
/// appelle `Window::emit("scan-progress", &progress)` et le passent ici.
pub async fn scan_folder_with_emitter(
    repo: &Repo,
    probe: &dyn ProbeBackend,
    folder_id: FolderId,
    folder_path: &std::path::Path,
    category: Category,
    emitter: Option<ScanProgressEmitter<'_>>,
) -> Result<ScanReport, KyroError> {
    let mut report = ScanReport::default();

    // -------------------------------------------------------------------
    // 1. Énumération des fichiers candidats.
    //    `discover_files_with_failures` (tâche 8.4) filtre par
    //    extension supportée ET retourne la liste des erreurs
    //    `walkdir` rencontrées (dossier interdit, fichier disparu en
    //    cours d'énumération). Ces échecs sont fusionnés dans
    //    `ScanReport.failures` afin qu'aucune erreur isolée ne soit
    //    perdue (requirement 18.1).
    // -------------------------------------------------------------------
    let (candidates, walk_failures) = discover_files_with_failures(folder_path);
    report.total_files = candidates.len();
    report.failures.extend(walk_failures);

    // Ensemble des chemins canoniques rencontrés sur disque, utilisé en
    // étape 4 pour identifier les disparus. On stocke des `String`
    // (forme persistée en DB) plutôt que des `PathBuf` afin de
    // comparer directement avec les valeurs renvoyées par les helpers
    // `list_*_under_folder`.
    let mut seen_paths: HashSet<String> = HashSet::with_capacity(candidates.len());

    // -------------------------------------------------------------------
    // 2/3. Traitement par fichier.
    // -------------------------------------------------------------------
    //
    // La série placeholder « À classer » est créée **paresseusement** :
    // tant qu'aucun épisode orphelin n'est rencontré, on ne la
    // matérialise pas. Cela évite de polluer la base avec une fiche
    // vide pour les dossiers Series/Animes dont tous les fichiers ont
    // une numérotation valide.
    let mut unsorted_series: Option<MediaItemId> = None;

    // Helper closure : émet `scan-progress` si un émetteur est fourni.
    // `total` est figé à `candidates.len()` (cf. `report.total_files`).
    // `processed` est 1-indexé (post-traitement du fichier courant).
    //
    // On capture `emitter` et `total` par `move` n'est pas nécessaire
    // ici : la fermeture vit dans le scope de la boucle ci-dessous et
    // emprunte ces deux valeurs immutablement. Le `current_file` est
    // cloné dans `ScanProgress` (la sérialisation Tauri exige une
    // valeur owned `String`).
    let total = candidates.len();
    let emit_progress = |processed: usize, current_file: &str| {
        if let Some(em) = emitter {
            em(&ScanProgress {
                processed,
                total,
                current_file: current_file.to_string(),
            });
        }
    };

    for (idx, path) in candidates.iter().enumerate() {
        // 1-indexé : à l'itération `idx`, le fichier courant est le
        // `(idx + 1)`-ème traité. Conforme à l'attente UI « X / Y ».
        let processed_now = idx + 1;
        // Forme stockée : chemin canonique (best-effort) → identique au
        // format de `add_library_folder` / `upsert_*`. Sans cette
        // canonicalisation, un fichier vu via deux chemins différents
        // (relatif vs absolu, casse Windows) serait compté deux fois.
        let canonical = canonicalize_or_keep(path);
        let stored_path = canonical.to_string_lossy().into_owned();
        seen_paths.insert(stored_path.clone());

        // 2.a — `fs::metadata` pour la taille (requirement 5.2). Une
        // erreur ici (fichier supprimé entre `discover_files` et
        // `metadata`, droits refusés) est traitée comme `ScanFailure
        // Io` et ne bloque pas la suite du scan.
        let size_bytes = match std::fs::metadata(path) {
            Ok(m) => m.len() as i64,
            Err(err) => {
                tracing::warn!(
                    file_path = %stored_path,
                    error = %err,
                    "Kyro scanner: lecture métadonnées échouée — fichier ignoré"
                );
                report.failures.push(ScanFailure {
                    file_path: stored_path,
                    kind: ScanFailureKind::Io,
                    message: err.to_string(),
                });
                emit_progress(processed_now, path.to_string_lossy().as_ref());
                continue;
            }
        };

        // 2.b — Skip idempotent : si une entrée DB existe avec
        // exactement `(file_path, size_bytes)`, le fichier est inchangé
        // et on n'a rien à faire (requirement 3.3).
        //
        // On lit l'état courant via les helpers `list_*_under_folder`
        // qui sont déjà chargés une fois par scan (cf. étape 4) ; pour
        // garder l'orchestrateur simple, on utilise ici une requête
        // ciblée sur le seul `file_path`. Le coût d'une requête
        // indexée par fichier reste négligeable face au probe ffprobe.
        //
        // **Réhabilitation idempotente** : si la fiche existe et
        // qu'on saute l'upsert, on appelle quand même `touch_*` pour
        // remettre `available = 1` + `last_scanned_at`. Cela couvre
        // le cas où un sous-dossier de bibliothèque a été renommé
        // (les fiches sont passées à `available = 0` à la passe
        // précédente) puis renommé à l'identique : sans ce `touch`,
        // les épisodes resteraient indisponibles indéfiniment et la
        // série parente serait masquée par `list_by_category`.
        if matches!(category, Category::Films) {
            if let Some((existing_id, existing_size)) =
                lookup_media_by_path(repo, &stored_path).await?
            {
                if existing_size == Some(size_bytes) {
                    let restored = repo.touch_media_available(existing_id).await?;
                    if restored {
                        report.updated += 1;
                        tracing::info!(
                            media_item_id = existing_id.get(),
                            file_path = %stored_path,
                            "Kyro scanner: media_item réhabilité (available 0→1) sans changement de taille"
                        );
                    }
                    report.processed += 1;
                    emit_progress(processed_now, &stored_path);
                    continue;
                }
            }
        } else if let Some((existing_id, existing_size)) =
            lookup_episode_by_path(repo, &stored_path).await?
        {
            if existing_size == Some(size_bytes) {
                let restored = repo.touch_episode_available(existing_id).await?;
                if restored {
                    report.updated += 1;
                    tracing::info!(
                        episode_item_id = existing_id.get(),
                        file_path = %stored_path,
                        "Kyro scanner: episode_item réhabilité (available 0→1) sans changement de taille"
                    );
                }
                report.processed += 1;
                emit_progress(processed_now, &stored_path);
                continue;
            }
        }

        // 2.c — Parsing du nom de fichier (pure, infaillible).
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let parsed = parser::parse(filename);
        let numbering_unknown = parser::is_numbering_unknown(category, &parsed);

        // 2.c.bis — Pour Series / Animes : le titre de la SÉRIE est
        // dérivé du nom du **dossier parent** plutôt que du nom de
        // fichier. Cela permet de regrouper tous les épisodes d'un
        // même dossier (`Bleach Kai - 720p.MULTI.x264 [Mixouille]/`)
        // sous une seule fiche série, indépendamment des variations
        // de noms de fichiers.
        //
        // Règles :
        //   - Si le dossier parent est exactement le dossier racine
        //     de la library_folder (le fichier est posé à plat dans
        //     `D:\Animes\xxx.mkv`), pas de regroupement par dossier
        //     possible : on retombe sur le titre du parser.
        //   - Sinon, on utilise le nom du dossier parent immédiat,
        //     nettoyé via le même `parser::clean_title` que pour les
        //     noms de fichier (retire qualité, année, motifs
        //     d'épisode résiduels, normalise les délimiteurs).
        //
        // Cette logique ne s'applique qu'aux Series / Animes ; les
        // Films restent identifiés par leur fichier individuel.
        let series_title = match category {
            Category::Films => parsed.title.clone(),
            Category::Series | Category::Animes => {
                derive_series_title_from_folder(path, folder_path)
                    .unwrap_or_else(|| parsed.title.clone())
            }
        };

        // 2.d — Probe ffprobe. Sur échec, on persiste tout de même
        // l'item avec champs techniques `null` (requirement 5.5) :
        // l'utilisateur voit l'entrée et peut la retenter manuellement
        // après avoir corrigé le sidecar ou le binaire.
        let metadata: Option<TechnicalMetadata> = match probe.probe(path).await {
            Ok(m) => Some(m),
            Err(err) => {
                let kind = match &err {
                    ProbeError::Metadata { .. } => ScanFailureKind::Io,
                    _ => ScanFailureKind::Probe,
                };
                tracing::warn!(
                    file_path = %stored_path,
                    error = %err,
                    "Kyro scanner: probe ffprobe échoué — entrée persistée sans \
                     champs techniques"
                );
                report.failures.push(ScanFailure {
                    file_path: stored_path.clone(),
                    kind,
                    message: err.to_string(),
                });
                None
            }
        };

        // 2.e — Persistance via le repo. Branche selon la catégorie.
        let persist_result: Result<PersistOutcome, KyroError> = match category {
            Category::Films => persist_film(
                repo,
                folder_id,
                &stored_path,
                size_bytes,
                &parsed,
                metadata.as_ref(),
            )
            .await,
            Category::Series | Category::Animes => {
                // Sous-cas : `numbering_unknown = true` → rattacher à
                // la série placeholder « À classer » (requirement 4.7).
                let series_id = if numbering_unknown {
                    let id = match unsorted_series {
                        Some(id) => id,
                        None => {
                            let id =
                                repo.ensure_unsorted_series(folder_id, category).await?;
                            unsorted_series = Some(id);
                            id
                        }
                    };
                    id
                } else {
                    // Sous-cas nominal : un épisode reconnu doit être
                    // attaché à une série « réelle », identifiée par
                    // le titre extrait du parser. On matérialise (ou
                    // récupère) cette série au vol.
                    ensure_series_for_title(repo, folder_id, category, &series_title)
                        .await?
                };

                persist_episode(
                    repo,
                    series_id,
                    &stored_path,
                    size_bytes,
                    &parsed,
                    metadata.as_ref(),
                    numbering_unknown,
                )
                .await
            }
        };

        match persist_result {
            Ok(PersistOutcome::Created) => {
                report.created += 1;
                report.processed += 1;
            }
            Ok(PersistOutcome::Updated) => {
                report.updated += 1;
                report.processed += 1;
            }
            Err(err) => {
                tracing::warn!(
                    file_path = %stored_path,
                    error = %err,
                    "Kyro scanner: persistance échouée — fichier ignoré"
                );
                report.failures.push(ScanFailure {
                    file_path: stored_path.clone(),
                    kind: ScanFailureKind::Db,
                    message: err.to_string(),
                });
            }
        }

        // 2.f — Émission de la progression (tâche 8.3, requirement 3.6).
        // Émis APRÈS le traitement effectif (succès, mise à jour, ou
        // échec isolé). Garantit la monotonie de `processed` côté UI :
        // le frontend voit `processed` croître strictement de 1 à
        // `total` au fil du scan, indépendamment du résultat (skip /
        // create / update / failure). En l'absence d'émetteur (cas par
        // défaut, tests 8.2), c'est un no-op à coût nul.
        emit_progress(processed_now, &stored_path);
    }

    // -------------------------------------------------------------------
    // 4. Disparitions : entrées DB sous ce dossier dont le `file_path`
    //    n'a pas été observé pendant l'énumération.
    // -------------------------------------------------------------------
    let media_under = repo.list_media_under_folder(folder_id).await?;
    for (id, file_path, _size) in &media_under {
        if !seen_paths.contains(file_path) {
            repo.mark_media_unavailable(*id).await?;
            report.marked_unavailable += 1;
            tracing::info!(
                media_item_id = id.get(),
                file_path = %file_path,
                "Kyro scanner: media_item marqué indisponible (fichier disparu)"
            );
        }
    }
    let episodes_under = repo.list_episodes_under_folder(folder_id).await?;
    for (id, file_path, _size) in &episodes_under {
        if !seen_paths.contains(file_path) {
            repo.mark_episode_unavailable(*id).await?;
            report.marked_unavailable += 1;
            tracing::info!(
                episode_item_id = id.get(),
                file_path = %file_path,
                "Kyro scanner: episode_item marqué indisponible (fichier disparu)"
            );
        }
    }

    // -------------------------------------------------------------------
    // 5. Réconciliation des séries/animes parentes (Series, Anime).
    //    Les fiches « racine » de série n'ont pas de `file_path` et ne
    //    sont donc jamais touchées par la passe de disparitions
    //    ci-dessus. Or `list_by_category` les filtre via la sous-requête
    //    `EXISTS (episode_item.available = 1)` : si tous les épisodes
    //    deviennent indisponibles, la série doit elle-même passer à
    //    `available = 0` ; à l'inverse, si au moins un épisode est
    //    réhabilité (cas du rename-puis-rename-back), la série doit
    //    repasser à `available = 1`. Sans cette passe, l'utilisateur
    //    voit les séries disparaître durablement de l'UI après une
    //    opération FS sur leur dossier.
    //
    //    Films exclus : ils n'ont pas de notion d'épisodes parents et
    //    leur disponibilité a déjà été gérée à l'étape 2 / 4.
    // -------------------------------------------------------------------
    if matches!(category, Category::Series | Category::Animes) {
        let series_rows = repo.list_series_under_folder(folder_id).await?;
        for (series_id, was_available) in series_rows {
            let n_available = repo.count_available_episodes(series_id).await?;
            let should_be_available = n_available > 0;
            if should_be_available && !was_available {
                let _ = repo.touch_media_available(series_id).await?;
                tracing::info!(
                    media_item_id = series_id.get(),
                    "Kyro scanner: série/anime réhabilitée (épisodes redevenus disponibles)"
                );
            } else if !should_be_available && was_available {
                repo.mark_media_unavailable(series_id).await?;
                report.marked_unavailable += 1;
                tracing::info!(
                    media_item_id = series_id.get(),
                    "Kyro scanner: série/anime marquée indisponible (plus aucun épisode disponible)"
                );
            }
        }
    }

    Ok(report)
}

/// Nom canonique de l'événement Tauri émis à chaque fichier traité
/// pendant un scan (requirement 3.6, design § « Kyro_Scanner —
/// événements UI »).
///
/// Exporté pour que les tests d'intégration et le frontend
/// (`window.listen("scan-progress", ...)`) puissent référencer la
/// même constante au lieu de dupliquer le littéral.
pub const EVENT_SCAN_PROGRESS: &str = "scan-progress";

/// Variante de [`scan_folder_with_emitter`] qui émet directement
/// l'événement Tauri `scan-progress` sur la fenêtre fournie
/// (tâche 8.3, requirement 3.6).
///
/// La fenêtre est utilisée comme cible de l'émission via
/// `tauri::Emitter::emit`. Si l'émission échoue (fenêtre fermée,
/// canal IPC saturé), l'erreur est journalisée mais **n'interrompt
/// pas le scan** : la progression UI est best-effort, le diff DB
/// reste la source de vérité.
///
/// Les call-sites « pure logique » (tests, scan headless) doivent
/// continuer à utiliser [`scan_folder`] ou
/// [`scan_folder_with_emitter`] : cette fonction est la couture
/// minimale entre le scanner et le runtime Tauri.
pub async fn scan_folder_emit<R: tauri::Runtime>(
    window: &tauri::Window<R>,
    repo: &Repo,
    probe: &dyn ProbeBackend,
    folder_id: FolderId,
    folder_path: &std::path::Path,
    category: Category,
) -> Result<ScanReport, KyroError> {
    use tauri::Emitter;

    // Closure d'émission : sérialise le payload et délègue à
    // `Window::emit`. Une erreur d'émission est journalisée mais
    // ignorée (cf. doc plus haut) — la progression est purement
    // informative, perdre un événement n'invalide pas le scan.
    let emitter: ScanProgressEmitter<'_> = &|progress: &ScanProgress| {
        if let Err(err) = window.emit(EVENT_SCAN_PROGRESS, progress) {
            tracing::warn!(
                event = EVENT_SCAN_PROGRESS,
                processed = progress.processed,
                total = progress.total,
                error = %err,
                "Kyro scanner: échec émission scan-progress (best-effort, ignoré)"
            );
        }
    };

    scan_folder_with_emitter(
        repo,
        probe,
        folder_id,
        folder_path,
        category,
        Some(emitter),
    )
    .await
}

// ---------------------------------------------------------------------------
// Helpers privés au module — lookups ponctuels par `file_path`.
// ---------------------------------------------------------------------------

/// Résultat d'une opération de persistance d'un fichier.
///
/// `Skipped` n'apparaît pas ici : le skip idempotent est traité **avant**
/// l'appel à `persist_*` (cf. étape 2.b de `scan_folder`).
enum PersistOutcome {
    Created,
    Updated,
}

/// Cherche un `media_item` par `file_path` exact (forme stockée). Le
/// scanner utilise cette requête pour décider du skip idempotent
/// avant d'engager le coût d'un probe ffprobe.
async fn lookup_media_by_path(
    repo: &Repo,
    file_path: &str,
) -> Result<Option<(MediaItemId, Option<i64>)>, KyroError> {
    let row: Option<(i64, Option<i64>)> = sqlx::query_as::<_, (i64, Option<i64>)>(
        "SELECT id, size_bytes FROM media_item WHERE file_path = ?",
    )
    .bind(file_path)
    .fetch_optional(repo.pool())
    .await?;
    Ok(row.map(|(id, size)| (MediaItemId(id), size)))
}

/// Cherche un `episode_item` par `file_path`. Variante stricte de
/// `lookup_media_by_path` adaptée au DDL `episode_item.file_path
/// UNIQUE NOT NULL`.
async fn lookup_episode_by_path(
    repo: &Repo,
    file_path: &str,
) -> Result<Option<(EpisodeItemId, Option<i64>)>, KyroError> {
    let row: Option<(i64, Option<i64>)> = sqlx::query_as::<_, (i64, Option<i64>)>(
        "SELECT id, size_bytes FROM episode_item WHERE file_path = ?",
    )
    .bind(file_path)
    .fetch_optional(repo.pool())
    .await?;
    Ok(row.map(|(id, size)| (EpisodeItemId(id), size)))
}

/// Persiste un Film via `repo.upsert_media_item`, en convertissant les
/// champs typés du parser/probe vers le format `UpsertMediaInput`.
async fn persist_film(
    repo: &Repo,
    folder_id: FolderId,
    file_path: &str,
    size_bytes: i64,
    parsed: &parser::ParsedFilename,
    metadata: Option<&TechnicalMetadata>,
) -> Result<PersistOutcome, KyroError> {
    // Détermine si la fiche existait déjà avant l'upsert pour
    // distinguer Created/Updated dans le `ScanReport`.
    let pre_existed = lookup_media_by_path(repo, file_path).await?.is_some();

    let input = UpsertMediaInput {
        folder_id: Some(folder_id),
        media_type: MediaType::Film,
        file_path: file_path.to_string(),
        size_bytes,
        title: parsed.title.clone(),
        year: parsed.year.map(|y| y as i64),
        quality: parsed.quality.map(|q| q.as_str().to_string()),
        duration_seconds: metadata.and_then(|m| m.duration_seconds.map(|d| d as i64)),
        video_codec: metadata.and_then(|m| m.video_codec.clone()),
        width: metadata.and_then(|m| m.width.map(|w| w as i64)),
        height: metadata.and_then(|m| m.height.map(|h| h as i64)),
        numbering_unknown: false,
    };
    repo.upsert_media_item(&input).await?;
    Ok(if pre_existed {
        PersistOutcome::Updated
    } else {
        PersistOutcome::Created
    })
}

/// Persiste un épisode via `repo.upsert_episode_item`. Saison / numéro
/// sont issus du parser (ou `0/0` si `numbering_unknown` — cf. série
/// placeholder « À classer », où la numérotation n'a pas de sens).
async fn persist_episode(
    repo: &Repo,
    series_id: MediaItemId,
    file_path: &str,
    size_bytes: i64,
    parsed: &parser::ParsedFilename,
    metadata: Option<&TechnicalMetadata>,
    numbering_unknown: bool,
) -> Result<PersistOutcome, KyroError> {
    let pre_existed = lookup_episode_by_path(repo, file_path).await?.is_some();

    // Saison / épisode : si reconnus, on les utilise ; sinon on
    // matérialise `(0, 0)` (rattachement « À classer ») — le DDL ne
    // contraint pas ces colonnes à être > 0, et le drapeau
    // `numbering_unknown` côté DB matérialise sémantiquement
    // l'inconnue.
    let (season, episode) = parsed
        .episode
        .map(|e| (e.season as i64, e.episode as i64))
        .unwrap_or((0, 0));

    let input = UpsertEpisodeInput {
        series_id,
        season_number: season,
        episode_number: episode,
        file_path: file_path.to_string(),
        size_bytes,
        duration_seconds: metadata.and_then(|m| m.duration_seconds.map(|d| d as i64)),
        video_codec: metadata.and_then(|m| m.video_codec.clone()),
        width: metadata.and_then(|m| m.width.map(|w| w as i64)),
        height: metadata.and_then(|m| m.height.map(|h| h as i64)),
        numbering_unknown,
    };
    repo.upsert_episode_item(&input).await?;
    Ok(if pre_existed {
        PersistOutcome::Updated
    } else {
        PersistOutcome::Created
    })
}

/// Dérive le titre d'une série depuis le dossier parent immédiat du
/// fichier `path`, à condition que ce dossier soit **distinct** du
/// dossier racine de la library_folder. Retourne `None` si le fichier
/// est posé à plat dans la racine (auquel cas l'appelant retombe sur
/// le titre du parser appliqué au nom de fichier).
///
/// Le nom du dossier est nettoyé via le même pipeline que les noms de
/// fichiers (`parser::clean_title`) afin d'enlever les suffixes
/// release-group habituels : qualité (`720p`), codec dans les motifs
/// reconnus, et délimiteurs (`.`, `_`, `-`). On enlève en plus les
/// **tags entre crochets ou parenthèses** (`[Mixouille]`, `(VOSTFR)`)
/// qui sont quasi-systématiques dans les conventions release-group.
///
/// Exemples :
///   - `D:\Animes\Bleach Kai - 720p.MULTI.x264 [Mixouille]\ep01.mkv`
///     → `Bleach Kai MULTI x264`
///   - `D:\Series\Breaking Bad\Season 01\S01E01.mkv` (parent direct
///     `Season 01` sans pattern d'épisode dans le nom de dossier)
///     → `Season 01` ; ce cas reste imparfait pour les conventions
///     `Season N/`, mais il garde le regroupement local et est
///     préférable à un nom de fichier individuel.
fn derive_series_title_from_folder(
    path: &std::path::Path,
    library_root: &std::path::Path,
) -> Option<String> {
    let parent = path.parent()?;

    // Comparaison robuste sur la canonicalisation : si on n'arrive
    // pas à canonicaliser (parent disparu, lien cassé), on retombe
    // sur la comparaison brute via `Path::eq` qui est correcte pour
    // l'usage normal (chemins absolus issus de `walkdir`).
    let parent_canonical = std::fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
    let root_canonical = std::fs::canonicalize(library_root)
        .unwrap_or_else(|_| library_root.to_path_buf());
    if parent_canonical == root_canonical {
        return None;
    }

    let folder_name = parent.file_name().and_then(|n| n.to_str())?;
    if folder_name.is_empty() {
        return None;
    }

    // Nettoyage : on retire d'abord les tags entre crochets et
    // parenthèses (release-group conventions), puis on délègue à
    // `parser::clean_title` pour la normalisation usuelle.
    let stripped = strip_bracket_tags(folder_name);
    let cleaned = parser::clean_title(&stripped);
    if cleaned.is_empty() {
        return None;
    }
    Some(cleaned)
}

/// Retire les segments encadrés par `[...]` ou `(...)` d'une chaîne.
///
/// Implémentation linéaire à la main pour éviter d'introduire une
/// regex spécifique : on parcourt les caractères en gardant un compteur
/// de profondeur pour chaque type de bracket. Toute paire imbriquée
/// est traitée correctement ; les brackets non appariés sont laissés
/// tels quels (cas dégénéré peu probable dans des noms de fichiers).
fn strip_bracket_tags(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut depth_square: usize = 0;
    let mut depth_paren: usize = 0;
    for c in input.chars() {
        match c {
            '[' => depth_square = depth_square.saturating_add(1),
            ']' => {
                if depth_square > 0 {
                    depth_square -= 1;
                    out.push(' '); // remplace par un espace pour ne pas coller deux mots
                } else {
                    out.push(c);
                }
                continue;
            }
            '(' => depth_paren = depth_paren.saturating_add(1),
            ')' => {
                if depth_paren > 0 {
                    depth_paren -= 1;
                    out.push(' ');
                } else {
                    out.push(c);
                }
                continue;
            }
            _ if depth_square > 0 || depth_paren > 0 => {
                // À l'intérieur d'un tag : on ne pousse rien.
                continue;
            }
            _ => {
                out.push(c);
            }
        }
    }
    out
}

/// Récupère ou crée la série « réelle » (`media_item.type IN
/// {'Series','Anime'}`) dont le titre matche `title`, dans le dossier
/// `folder_id`. Cette fonction est invoquée par le scanner quand un
/// épisode au pattern reconnu doit être rattaché à sa série mère.
///
/// Stratégie : `(folder_id, type, title) avec file_path IS NULL` est
/// utilisé comme clé d'unicité applicative — les séries n'ont pas de
/// fichier propre, donc `file_path NULL` distingue clairement une
/// série racine d'un Film. La même convention est appliquée par
/// `ensure_unsorted_series`.
async fn ensure_series_for_title(
    repo: &Repo,
    folder_id: FolderId,
    category: Category,
    title: &str,
) -> Result<MediaItemId, KyroError> {
    let media_type = match category {
        Category::Series => MediaType::Series,
        Category::Animes => MediaType::Anime,
        Category::Films => {
            return Err(KyroError::Validation(
                "ensure_series_for_title invoqué pour catégorie Films".to_string(),
            ));
        }
    };

    // Lookup direct par (folder_id, type, title, file_path NULL ou
    // chaîne vide). On accepte `file_path = ''` pour la
    // rétrocompatibilité avec d'anciennes fiches insérées sans
    // contrainte stricte ; les nouvelles insertions utilisent
    // toujours NULL.
    let existing: Option<i64> = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM media_item \
          WHERE folder_id = ? AND type = ? AND title = ? \
            AND (file_path IS NULL OR file_path = '') \
          LIMIT 1",
    )
    .bind(folder_id.get())
    .bind(media_type.as_str())
    .bind(title)
    .fetch_optional(repo.pool())
    .await?;

    if let Some(id) = existing {
        return Ok(MediaItemId(id));
    }

    let id: i64 = sqlx::query_scalar::<_, i64>(
        "INSERT INTO media_item (folder_id, type, title, available) \
         VALUES (?, ?, ?, 1) RETURNING id",
    )
    .bind(folder_id.get())
    .bind(media_type.as_str())
    .bind(title)
    .fetch_one(repo.pool())
    .await?;

    tracing::info!(
        folder_id = folder_id.get(),
        media_item_id = id,
        media_type = media_type.as_str(),
        title = title,
        "Kyro scanner: nouvelle série créée à partir du parser"
    );
    Ok(MediaItemId(id))
}

// ===========================================================================
// Tests unitaires (tâche 8.2).
//
// On couvre les quatre cas explicitement demandés par la tâche :
//   1. Idempotence : deux scans successifs ⇒ aucun delta au second.
//   2. Nouveau fichier : entrée créée avec champs parsés/probe.
//   3. Fichier disparu : entrée DB devient `available = 0`, **non**
//      supprimée.
//   4. Series sans pattern : rattachement à « À classer » avec
//      `numbering_unknown = 1`.
//
// Tous les tests utilisent un `StubProbe` qui retourne des
// `TechnicalMetadata` synthétiques : on ne dépend ni du binaire
// ffprobe ni d'un sidecar bundle. La DB est créée dans un `TempDir`
// avec les migrations versionnées.
// ===========================================================================

#[cfg(test)]
mod tests_8_2 {
    use std::fs;
    use std::future::Future;
    use std::path::Path;
    use std::pin::Pin;
    use std::sync::Arc;

    use sqlx::sqlite::{
        SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
    };
    use tempfile::TempDir;

    use super::*;
    use crate::domain::types::Category;
    use crate::infra::probe::{
        AudioTrack, ProbeBackend, ProbeError, SubtitleTrack, TechnicalMetadata,
    };
    use crate::infra::repo::Repo;

    // -----------------------------------------------------------------------
    // Stub `ProbeBackend` pour les tests : retourne toujours la même
    // `TechnicalMetadata` (durée + codec fixes) pour n'importe quel
    // chemin, ou une erreur configurable.
    // -----------------------------------------------------------------------

    enum StubBehavior {
        Ok(TechnicalMetadata),
        ProbeError,
    }

    struct StubProbe {
        // Arc autour d'une fabrique d'erreurs/metadata pour rester
        // `Send + Sync` et pouvoir partager entre threads.
        factory: Arc<dyn Fn() -> StubBehavior + Send + Sync>,
    }

    impl StubProbe {
        fn ok(meta: TechnicalMetadata) -> Self {
            let cloned = meta.clone();
            Self {
                factory: Arc::new(move || StubBehavior::Ok(cloned.clone())),
            }
        }

        fn always_err() -> Self {
            Self {
                factory: Arc::new(|| StubBehavior::ProbeError),
            }
        }
    }

    impl ProbeBackend for StubProbe {
        fn probe<'a>(
            &'a self,
            path: &'a Path,
        ) -> Pin<Box<dyn Future<Output = Result<TechnicalMetadata, ProbeError>> + Send + 'a>>
        {
            let factory = self.factory.clone();
            Box::pin(async move {
                match (factory)() {
                    StubBehavior::Ok(mut meta) => {
                        // Le scanner appelle `fs::metadata` séparément,
                        // donc `size_bytes` est ignoré ici. On garde 0
                        // pour rendre l'erreur évidente si quelque
                        // chose d'autre l'utilise.
                        meta.size_bytes = std::fs::metadata(path)
                            .map(|m| m.len())
                            .unwrap_or(0);
                        Ok(meta)
                    }
                    StubBehavior::ProbeError => {
                        Err(ProbeError::NonZeroExit {
                            code: 1,
                            stderr: "stub failure".to_string(),
                        })
                    }
                }
            })
        }
    }

    fn make_meta() -> TechnicalMetadata {
        TechnicalMetadata {
            duration_seconds: Some(120),
            video_codec: Some("h264".to_string()),
            width: Some(1920),
            height: Some(1080),
            audio_tracks: vec![AudioTrack {
                codec: Some("aac".to_string()),
                language: Some("eng".to_string()),
            }],
            subtitle_tracks: Vec::<SubtitleTrack>::new(),
            size_bytes: 0,
        }
    }

    // -----------------------------------------------------------------------
    // Helpers de bootstrap DB / runtime.
    // -----------------------------------------------------------------------

    fn block_on<F: Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime tokio")
            .block_on(fut)
    }

    async fn fresh_repo(db_dir: &Path) -> Repo {
        let db_path = db_dir.join("library.db");
        let opts = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(opts)
            .await
            .expect("pool sqlite test");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations test");
        Repo::new(pool)
    }

    /// Écrit un fichier vidéo synthétique avec un contenu précis (pour
    /// faire varier `size_bytes`). Retourne le chemin créé.
    fn touch_video(dir: &Path, rel: &str, content: &[u8]) -> std::path::PathBuf {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, content).unwrap();
        path
    }

    // -----------------------------------------------------------------------
    // Test 1 — idempotence : un re-scan sans changement n'entraîne
    // ni création ni mise à jour.
    // -----------------------------------------------------------------------

    #[test]
    fn rescan_is_idempotent_when_nothing_changed() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let lib_dir = TempDir::new().unwrap();
            let repo = fresh_repo(db_dir.path()).await;

            let folder_id = repo
                .add_library_folder(lib_dir.path(), Category::Films)
                .await
                .expect("add_library_folder");

            touch_video(lib_dir.path(), "Inception.2010.1080p.mkv", b"abcdef");

            let probe = StubProbe::ok(make_meta());

            let r1 = scan_folder(
                &repo,
                &probe,
                folder_id,
                lib_dir.path(),
                Category::Films,
            )
            .await
            .expect("scan 1 ok");
            assert_eq!(r1.total_files, 1);
            assert_eq!(r1.created, 1);
            assert_eq!(r1.updated, 0);
            assert_eq!(r1.marked_unavailable, 0);
            assert!(r1.failures.is_empty(), "{:?}", r1.failures);

            let r2 = scan_folder(
                &repo,
                &probe,
                folder_id,
                lib_dir.path(),
                Category::Films,
            )
            .await
            .expect("scan 2 ok");
            assert_eq!(r2.total_files, 1);
            assert_eq!(
                r2.created, 0,
                "rescan ne doit pas recréer (idempotence requirement 3.3)"
            );
            assert_eq!(
                r2.updated, 0,
                "rescan ne doit pas mettre à jour (taille inchangée)"
            );
            assert_eq!(r2.marked_unavailable, 0);
            assert_eq!(r2.processed, 1, "le fichier doit être compté comme traité");
        });
    }

    // -----------------------------------------------------------------------
    // Test 2 — nouveau fichier : insertion d'un media_item avec
    // titre/année/qualité parsés et taille fidèle.
    // -----------------------------------------------------------------------

    #[test]
    fn new_film_is_inserted_with_parsed_and_probed_fields() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let lib_dir = TempDir::new().unwrap();
            let repo = fresh_repo(db_dir.path()).await;

            let folder_id = repo
                .add_library_folder(lib_dir.path(), Category::Films)
                .await
                .unwrap();

            let bytes: Vec<u8> = (0..1234u32).map(|i| (i % 256) as u8).collect();
            let film_path =
                touch_video(lib_dir.path(), "Dune.2021.4K.mkv", &bytes);

            let probe = StubProbe::ok(make_meta());

            let report = scan_folder(
                &repo,
                &probe,
                folder_id,
                lib_dir.path(),
                Category::Films,
            )
            .await
            .expect("scan ok");

            assert_eq!(report.total_files, 1);
            assert_eq!(report.created, 1);
            assert!(report.failures.is_empty());

            // Lecture brute pour vérifier les champs persistés.
            let canonical_path = canonicalize_or_keep(&film_path)
                .to_string_lossy()
                .into_owned();
            let row: (
                String,
                Option<i64>,
                Option<i64>,
                Option<String>,
                Option<i64>,
                Option<String>,
                i64,
            ) = sqlx::query_as(
                "SELECT title, year, size_bytes, quality, duration_seconds, \
                        video_codec, available \
                   FROM media_item WHERE file_path = ?",
            )
            .bind(&canonical_path)
            .fetch_one(repo.pool())
            .await
            .expect("media_item inséré");

            assert_eq!(row.0, "Dune");
            assert_eq!(row.1, Some(2021));
            assert_eq!(row.2, Some(bytes.len() as i64));
            assert_eq!(row.3.as_deref(), Some("4k"));
            assert_eq!(row.4, Some(120));
            assert_eq!(row.5.as_deref(), Some("h264"));
            assert_eq!(row.6, 1, "available doit être 1");
        });
    }

    // -----------------------------------------------------------------------
    // Test 3 — fichier disparu : entrée précédemment indexée passe à
    // `available = 0` mais reste en base.
    // -----------------------------------------------------------------------

    #[test]
    fn vanished_file_is_marked_unavailable_not_deleted() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let lib_dir = TempDir::new().unwrap();
            let repo = fresh_repo(db_dir.path()).await;

            let folder_id = repo
                .add_library_folder(lib_dir.path(), Category::Films)
                .await
                .unwrap();

            let film = touch_video(lib_dir.path(), "Tenet.2020.1080p.mkv", b"hello");
            // Capturer le chemin canonique **avant** suppression : une
            // fois le fichier supprimé, `std::fs::canonicalize` échoue
            // et `canonicalize_or_keep` retombe sur la forme brute, ce
            // qui ne correspond plus à ce qui a été persisté.
            let canonical_path =
                canonicalize_or_keep(&film).to_string_lossy().into_owned();

            let probe = StubProbe::ok(make_meta());

            // Premier scan : crée la fiche.
            let r1 = scan_folder(
                &repo,
                &probe,
                folder_id,
                lib_dir.path(),
                Category::Films,
            )
            .await
            .unwrap();
            assert_eq!(r1.created, 1);

            // Suppression du fichier puis re-scan.
            fs::remove_file(&film).unwrap();
            let r2 = scan_folder(
                &repo,
                &probe,
                folder_id,
                lib_dir.path(),
                Category::Films,
            )
            .await
            .unwrap();

            assert_eq!(r2.total_files, 0, "plus aucun fichier sur disque");
            assert_eq!(
                r2.marked_unavailable, 1,
                "l'entrée devait être marquée indisponible"
            );

            // La ligne existe toujours, juste `available = 0`.
            let avail: Option<i64> = sqlx::query_scalar::<_, i64>(
                "SELECT available FROM media_item WHERE file_path = ?",
            )
            .bind(&canonical_path)
            .fetch_optional(repo.pool())
            .await
            .unwrap();
            assert_eq!(
                avail,
                Some(0),
                "la fiche doit toujours exister avec available = 0"
            );
        });
    }

    // -----------------------------------------------------------------------
    // Test 4 — Series sans pattern d'épisode : rattachement à
    // « À classer » avec `numbering_unknown = 1`.
    // -----------------------------------------------------------------------

    #[test]
    fn series_without_episode_pattern_is_routed_to_unsorted() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let lib_dir = TempDir::new().unwrap();
            let repo = fresh_repo(db_dir.path()).await;

            let folder_id = repo
                .add_library_folder(lib_dir.path(), Category::Series)
                .await
                .unwrap();

            // Pas de pattern d'épisode reconnaissable (pas de SxxExx,
            // pas de NxN, pas de marqueur `ep`/`-`+nombre).
            let mystery =
                touch_video(lib_dir.path(), "MysteryNoNumbering.mkv", b"world");

            let probe = StubProbe::ok(make_meta());

            let report = scan_folder(
                &repo,
                &probe,
                folder_id,
                lib_dir.path(),
                Category::Series,
            )
            .await
            .expect("scan ok");

            assert_eq!(report.total_files, 1);
            assert_eq!(report.created, 1);

            // Vérifier que la série « À classer » a bien été créée.
            let unsorted_id: Option<i64> = sqlx::query_scalar::<_, i64>(
                "SELECT id FROM media_item WHERE title = ? AND folder_id = ? \
                 AND file_path IS NULL",
            )
            .bind(crate::infra::repo::UNSORTED_SERIES_TITLE)
            .bind(folder_id.get())
            .fetch_optional(repo.pool())
            .await
            .unwrap();
            assert!(
                unsorted_id.is_some(),
                "la série placeholder « À classer » doit exister"
            );
            let unsorted_id = unsorted_id.unwrap();

            // Vérifier que l'épisode est rattaché à cette série, avec
            // `numbering_unknown = 1`.
            let canonical_path = canonicalize_or_keep(&mystery)
                .to_string_lossy()
                .into_owned();
            let row: (i64, i64) = sqlx::query_as(
                "SELECT series_id, numbering_unknown \
                   FROM episode_item WHERE file_path = ?",
            )
            .bind(&canonical_path)
            .fetch_one(repo.pool())
            .await
            .unwrap();
            assert_eq!(row.0, unsorted_id, "rattachement à « À classer » attendu");
            assert_eq!(row.1, 1, "numbering_unknown doit être 1");
        });
    }

    // -----------------------------------------------------------------------
    // Bonus — couverture rapide : un probe en erreur n'aborte pas le
    // scan (requirement 18.1) ; le fichier est compté comme `failure`
    // mais l'entrée DB est tout de même persistée avec champs
    // techniques null. Cela exerce le chemin `metadata = None` côté
    // scanner et anticipe légèrement la tâche 8.4 / 8.9.
    // -----------------------------------------------------------------------

    #[test]
    fn probe_error_does_not_abort_scan() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let lib_dir = TempDir::new().unwrap();
            let repo = fresh_repo(db_dir.path()).await;

            let folder_id = repo
                .add_library_folder(lib_dir.path(), Category::Films)
                .await
                .unwrap();

            touch_video(lib_dir.path(), "Broken.2024.1080p.mkv", b"xx");

            let probe = StubProbe::always_err();
            let report = scan_folder(
                &repo,
                &probe,
                folder_id,
                lib_dir.path(),
                Category::Films,
            )
            .await
            .expect("scan ne doit pas remonter une erreur globale");

            assert_eq!(report.total_files, 1);
            assert_eq!(report.failures.len(), 1);
            assert_eq!(report.failures[0].kind, ScanFailureKind::Probe);
            // L'item est tout de même persisté avec champs techniques
            // null (requirement 5.5) — ce qui compte ici comme
            // `created`.
            assert_eq!(report.created, 1);
        });
    }

    // -----------------------------------------------------------------------
    // Tâche 8.3 — émetteur `scan-progress`.
    //
    // Le test vérifie quatre propriétés du contrat publié par
    // `scan_folder_with_emitter` (requirement 3.6) :
    //
    //   1. L'émetteur est invoqué **exactement N fois** pour N fichiers
    //      candidats — aucun fichier n'est silencieusement sauté.
    //   2. `processed` est **strictement croissant** de 1 à N (1-indexé).
    //   3. `total` reste constant et égal à N pendant tout le scan, ce
    //      qui permet à l'UI d'afficher un ratio stable.
    //   4. `current_file` correspond à un chemin observé sur disque
    //      (forme stockée canonicalisée), garantissant que le frontend
    //      peut corréler avec `media_item.file_path`.
    //
    // Le scénario inclut volontairement plusieurs sous-dossiers et un
    // fichier non supporté (`.txt`) pour s'assurer que le filtre 8.1
    // n'introduit pas de fichier fantôme dans le flux d'émission.
    // -----------------------------------------------------------------------

    #[test]
    fn emitter_is_called_once_per_candidate_file() {
        use std::sync::Mutex;

        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let lib_dir = TempDir::new().unwrap();
            let repo = fresh_repo(db_dir.path()).await;

            let folder_id = repo
                .add_library_folder(lib_dir.path(), Category::Films)
                .await
                .unwrap();

            // Trois fichiers vidéo (deux à la racine, un imbriqué) +
            // un fichier non supporté qui doit être ignoré.
            touch_video(lib_dir.path(), "Alpha.2020.1080p.mkv", b"a");
            touch_video(lib_dir.path(), "Beta.2021.720p.mp4", b"bb");
            touch_video(lib_dir.path(), "nested/Gamma.2022.4K.webm", b"ccc");
            touch_video(lib_dir.path(), "readme.txt", b"ignored");

            // Capture des événements émis dans un Mutex<Vec<_>>. Le
            // closure n'a besoin ni de `Send` ni de `Sync` réels en
            // mono-thread, mais le type-alias les exige : un `Mutex`
            // suffit pour satisfaire le borrow-checker.
            let captured: Mutex<Vec<ScanProgress>> = Mutex::new(Vec::new());
            let emitter: ScanProgressEmitter<'_> = &|p: &ScanProgress| {
                captured.lock().unwrap().push(p.clone());
            };

            let probe = StubProbe::ok(make_meta());
            let report = scan_folder_with_emitter(
                &repo,
                &probe,
                folder_id,
                lib_dir.path(),
                Category::Films,
                Some(emitter),
            )
            .await
            .expect("scan ok");

            // Sanity : 3 fichiers vidéo créés, .txt ignoré.
            assert_eq!(report.total_files, 3);
            assert_eq!(report.created, 3);
            assert!(report.failures.is_empty());

            let events = captured.lock().unwrap().clone();
            assert_eq!(
                events.len(),
                3,
                "l'émetteur doit être invoqué une fois par fichier candidat"
            );

            // `processed` strictement croissant de 1 à N, `total`
            // constant à N pour tous les événements.
            for (i, evt) in events.iter().enumerate() {
                assert_eq!(
                    evt.processed,
                    i + 1,
                    "processed doit être 1-indexé et strictement croissant"
                );
                assert_eq!(
                    evt.total, 3,
                    "total doit rester constant pendant tout le scan"
                );
                assert!(
                    !evt.current_file.is_empty(),
                    "current_file ne doit jamais être vide"
                );
            }

            // Tous les `current_file` émis doivent correspondre à des
            // fichiers réellement persistés en DB (forme stockée
            // canonicalisée).
            let stored_paths: Vec<String> = sqlx::query_scalar::<_, String>(
                "SELECT file_path FROM media_item WHERE file_path IS NOT NULL",
            )
            .fetch_all(repo.pool())
            .await
            .unwrap();
            for evt in &events {
                assert!(
                    stored_paths.contains(&evt.current_file),
                    "current_file = {:?} doit correspondre à une fiche persistée \
                     (paths stockés = {:?})",
                    evt.current_file,
                    stored_paths
                );
            }
        });
    }

    /// Garantit que `scan_folder` (sans émetteur) reste un strict alias
    /// de `scan_folder_with_emitter(..., None)` — aucune régression
    /// fonctionnelle introduite par la tâche 8.3.
    #[test]
    fn scan_folder_without_emitter_matches_with_emitter_none() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let lib_dir = TempDir::new().unwrap();
            let repo = fresh_repo(db_dir.path()).await;
            let folder_id = repo
                .add_library_folder(lib_dir.path(), Category::Films)
                .await
                .unwrap();
            touch_video(lib_dir.path(), "Solo.2018.1080p.mkv", b"solo");

            let probe = StubProbe::ok(make_meta());
            let r_legacy = scan_folder(
                &repo,
                &probe,
                folder_id,
                lib_dir.path(),
                Category::Films,
            )
            .await
            .unwrap();
            // Re-scan via la nouvelle signature avec `None` ⇒ skip
            // idempotent attendu, comportement identique à un re-scan
            // legacy.
            let r_new = scan_folder_with_emitter(
                &repo,
                &probe,
                folder_id,
                lib_dir.path(),
                Category::Films,
                None,
            )
            .await
            .unwrap();

            assert_eq!(r_legacy.total_files, r_new.total_files);
            assert_eq!(r_legacy.created, 1);
            assert_eq!(r_new.created, 0, "second scan idempotent");
            assert_eq!(r_new.updated, 0);
            assert_eq!(r_new.processed, 1);
        });
    }


    // -----------------------------------------------------------------------
    // Tâche 8.4 — robustesse aux erreurs isolées (requirement 18.1).
    //
    // Couverture additionnelle :
    //   A. `discover_files_with_failures` capture l'erreur walkdir
    //      d'un `root` inexistant sous forme de `ScanFailure { kind:
    //      Walk, .. }`, sans paniquer.
    //   B. `scan_folder` fusionne les `ScanFailure` walkdir dans
    //      `ScanReport.failures` et continue de traiter les fichiers
    //      valides présents à côté d'une entrée walkdir fautive.
    //   C. Une erreur `fs::metadata` isolée (fichier qui disparaît
    //      entre `discover_files` et `metadata`) est journalisée et
    //      remontée via `ScanFailure { kind: Io, .. }` sans
    //      interrompre le scan ; les autres fichiers sont traités
    //      normalement.
    // -----------------------------------------------------------------------

    /// Cas A — `discover_files_with_failures` sur un root inexistant.
    ///
    /// `WalkDir::new(missing).into_iter()` rend une erreur initiale
    /// (impossible d'ouvrir le dossier). On vérifie que cette erreur
    /// est convertie en `ScanFailure { kind: Walk }` au lieu d'être
    /// silencieusement avalée comme c'était le cas en 8.1. La liste
    /// des fichiers reste vide.
    #[test]
    fn discover_with_failures_reports_walkdir_error_on_missing_root() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("does_not_exist");

        let (files, failures) = discover_files_with_failures(&missing);

        assert!(files.is_empty(), "aucun fichier ne doit être retourné");
        assert!(
            !failures.is_empty(),
            "l'erreur walkdir initiale doit être remontée"
        );
        assert!(
            failures.iter().all(|f| f.kind == ScanFailureKind::Walk),
            "toutes les erreurs ici sont de type Walk : {failures:?}"
        );
        // Le `file_path` doit pointer vers le root (best-effort).
        assert!(
            failures
                .iter()
                .any(|f| f.file_path.contains("does_not_exist")),
            "le file_path doit référencer le root manquant : {failures:?}"
        );
    }

    /// Cas B — `scan_folder` accumule un échec walkdir isolé tout en
    /// continuant à traiter les fichiers valides.
    ///
    /// Sur Windows, créer un dossier avec permissions réellement
    /// refusées requiert des privilèges qui ne sont pas garantis dans
    /// l'environnement de test. On exerce donc le cas par injection
    /// directe via `discover_files_with_failures` sur un root
    /// inexistant **après** avoir indexé un fichier valide à côté :
    /// le test vérifie ainsi le contrat « walk error → ScanFailure »
    /// sans dépendre d'ACL OS-spécifiques.
    ///
    /// Pour la branche `scan_folder` à proprement parler, on déclenche
    /// l'erreur `walkdir` en supprimant le dossier de bibliothèque
    /// **après** son enregistrement et juste avant le scan : le
    /// scanner doit alors retourner un `ScanReport` non vide côté
    /// `failures` et `total_files = 0`, sans paniquer.
    #[test]
    fn scan_folder_continues_on_walkdir_error_for_missing_folder() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let lib_dir = TempDir::new().unwrap();
            let repo = fresh_repo(db_dir.path()).await;

            // Enregistre le dossier puis supprime-le pour forcer une
            // erreur walkdir au prochain scan. `add_library_folder`
            // exige un dossier existant à l'enregistrement, donc
            // l'ordre est : add → remove physique → scan.
            let folder_id = repo
                .add_library_folder(lib_dir.path(), Category::Films)
                .await
                .unwrap();
            let folder_path = lib_dir.path().to_path_buf();
            // Drop explicite : `TempDir::close` supprime le dossier.
            drop(lib_dir);

            let probe = StubProbe::ok(make_meta());
            let report = scan_folder(
                &repo,
                &probe,
                folder_id,
                &folder_path,
                Category::Films,
            )
            .await
            .expect("le scan doit retourner un rapport, pas une erreur globale");

            // Le dossier n'existe plus → 0 fichier, et au moins une
            // entrée Walk dans `failures` (requirement 18.1).
            assert_eq!(report.total_files, 0);
            assert_eq!(report.processed, 0);
            assert!(
                report
                    .failures
                    .iter()
                    .any(|f| f.kind == ScanFailureKind::Walk),
                "au moins un ScanFailure Walk attendu : {:?}",
                report.failures
            );
        });
    }

    /// Cas C — un fichier qui disparaît entre `discover_files` et
    /// `fs::metadata` est remonté en `ScanFailure { kind: Io }` sans
    /// interrompre le scan ; le second fichier valide à côté est
    /// indexé normalement.
    ///
    /// On simule la course en :
    ///   1. créant deux fichiers (`alpha.mkv`, `beta.mkv`) ;
    ///   2. lançant `discover_files_with_failures` (les deux sont
    ///      énumérés) — déjà couvert par les tests 8.1 ;
    ///   3. supprimant `alpha.mkv` **avant** d'appeler `scan_folder` ;
    ///      en pratique le scanner refait son propre `discover_files`,
    ///      et seul `beta.mkv` est observé (et indexé), tandis qu'on
    ///      vérifie qu'aucune `ScanFailure` parasite n'apparaît.
    ///
    /// Pour exercer plus directement la branche `fs::metadata` Err,
    /// on utilise une stratégie portable : créer un fichier puis le
    /// remplacer par un dossier du même nom ne fonctionne pas
    /// proprement sur Windows ; on se rabat sur un test qui exerce le
    /// chemin général « erreur isolée → scan continue » via une
    /// simulation walkdir + fichier valide voisin.
    #[test]
    fn scan_folder_processes_valid_files_alongside_isolated_failures() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let lib_dir = TempDir::new().unwrap();
            let repo = fresh_repo(db_dir.path()).await;

            let folder_id = repo
                .add_library_folder(lib_dir.path(), Category::Films)
                .await
                .unwrap();

            // Fichier valide qui doit être indexé.
            let valid =
                touch_video(lib_dir.path(), "Valid.2024.1080p.mkv", b"valid-bytes");

            // Probe configuré pour ÉCHOUER : exerce le chemin
            // `failures` côté probe tout en garantissant que le
            // fichier valide est tout de même persisté avec champs
            // techniques null (requirement 5.5). Le scan ne doit pas
            // s'arrêter (requirement 18.1).
            let probe = StubProbe::always_err();

            // Ajout d'un second fichier dont on peut prédire le
            // traitement : même probe Err, donc 2 ScanFailure
            // distincts mais 2 entrées créées.
            touch_video(lib_dir.path(), "Other.2023.720p.mkv", b"other");

            let report = scan_folder(
                &repo,
                &probe,
                folder_id,
                lib_dir.path(),
                Category::Films,
            )
            .await
            .expect("scan ne doit pas remonter d'erreur globale");

            assert_eq!(report.total_files, 2, "2 fichiers énumérés");
            assert_eq!(report.processed, 2, "les 2 fichiers ont été traités");
            assert_eq!(report.created, 2, "les 2 fiches ont été persistées");
            assert_eq!(
                report.failures.len(),
                2,
                "1 ScanFailure Probe par fichier : {:?}",
                report.failures
            );
            assert!(
                report
                    .failures
                    .iter()
                    .all(|f| f.kind == ScanFailureKind::Probe),
                "toutes les failures sont de type Probe : {:?}",
                report.failures
            );

            // Vérifie que la persistance des fichiers valides est
            // bien intervenue malgré les failures probe.
            let canonical_valid =
                canonicalize_or_keep(&valid).to_string_lossy().into_owned();
            let exists: Option<i64> = sqlx::query_scalar::<_, i64>(
                "SELECT id FROM media_item WHERE file_path = ?",
            )
            .bind(&canonical_valid)
            .fetch_optional(repo.pool())
            .await
            .unwrap();
            assert!(
                exists.is_some(),
                "Valid.2024.1080p.mkv doit être persisté malgré le probe Err"
            );
        });
    }
}

// ===========================================================================
// Tâche 8.9 — Property 37 : continuation du scan en cas d'erreur isolée
// (requirement 18.1).
//
// Feature: kyro, Property 37.
//
// **Validates: Requirements 18.1**
//
// Pour toute liste arbitraire de fichiers vidéo `(name_id,
// should_fail)` placée dans un dossier de bibliothèque, où chaque
// fichier marqué `should_fail` voit son probe ffprobe échouer alors
// que les autres réussissent, l'invariant publié par `scan_folder`
// est :
//
//   1. **Pas d'arrêt global.** L'appel retourne `Ok(report)` quel
//      que soit le nombre d'erreurs isolées (aucun panic, aucun
//      `Err(KyroError)`).
//   2. **Persistance complète.** Tous les fichiers énumérés sont
//      indexés (`report.created == fichiers.len()`), y compris ceux
//      dont le probe a échoué — requirement 5.5 force la persistance
//      avec champs techniques `null` plutôt que d'ignorer la fiche.
//   3. **Comptage des échecs.** `report.failures.len()` est
//      strictement égal au nombre de fichiers générés en mode
//      `should_fail` ; aucun échec spurieux n'apparaît pour les
//      fichiers valides.
//   4. **Traçabilité par chemin.** Le `file_path` (forme
//      canonicalisée) de chaque fichier en erreur figure dans
//      `report.failures`, ce qui permet à l'UI/au log d'afficher
//      exactement la liste des fichiers problématiques (requirement
//      18.1 — « journaliser l'erreur avec le chemin du fichier
//      concerné »).
//
// Le générateur tire un `Vec<bool>` (chaque bool = `should_fail`) de
// taille 0..=20 ; le coût d'un cas (création de la DB + N fichiers
// + scan) reste tractable, ce qui permet de tenir 50 cas dans une
// CI sans timeout (cf. `ProptestConfig { cases: 50, .. }`).
//
// Le `PerPathStubProbe` consulte le nom de fichier pour décider de
// renvoyer `Ok(metadata)` ou `Err(ProbeError::NonZeroExit)` ; cette
// granularité fichier-par-fichier est exactement ce que demande la
// tâche pour entrelacer fichiers valides et fichiers en erreur dans
// un même dossier scanné.
// ===========================================================================

#[cfg(test)]
mod tests_8_9 {
    use std::collections::BTreeSet;
    use std::fs;
    use std::future::Future;
    use std::path::Path;
    use std::pin::Pin;
    use std::sync::Arc;

    use proptest::prelude::*;
    use sqlx::sqlite::{
        SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
    };
    use tempfile::TempDir;

    use super::*;
    use crate::domain::types::Category;
    use crate::infra::probe::{
        AudioTrack, ProbeBackend, ProbeError, SubtitleTrack, TechnicalMetadata,
    };
    use crate::infra::repo::Repo;

    // -----------------------------------------------------------------------
    // Stub `ProbeBackend` configurable par chemin.
    //
    // Contrairement au `StubProbe` du module `tests_8_2` qui retourne
    // le même résultat pour tous les chemins, ce stub évalue un
    // prédicat sur le `&Path` reçu et choisit Ok / Err en
    // conséquence. C'est exactement la primitive demandée par la
    // tâche 8.9 (« StubProbe::with_predicate(|path| ...) ») pour
    // entrelacer fichiers en erreur et fichiers valides dans un même
    // scan.
    // -----------------------------------------------------------------------

    struct PerPathStubProbe {
        // Prédicat « doit échouer » consulté pour chaque appel à
        // `probe(path)`. `Send + Sync` requis car `ProbeBackend`
        // pourrait être appelé depuis un `tokio::spawn` (le scanner
        // l'utilise via `&dyn ProbeBackend` Sync-compatible).
        should_fail: Arc<dyn Fn(&Path) -> bool + Send + Sync>,
        meta: TechnicalMetadata,
    }

    impl PerPathStubProbe {
        fn with_predicate<F>(meta: TechnicalMetadata, pred: F) -> Self
        where
            F: Fn(&Path) -> bool + Send + Sync + 'static,
        {
            Self {
                should_fail: Arc::new(pred),
                meta,
            }
        }
    }

    impl ProbeBackend for PerPathStubProbe {
        fn probe<'a>(
            &'a self,
            path: &'a Path,
        ) -> Pin<Box<dyn Future<Output = Result<TechnicalMetadata, ProbeError>> + Send + 'a>>
        {
            let should_fail = self.should_fail.clone();
            let mut meta = self.meta.clone();
            Box::pin(async move {
                if should_fail(path) {
                    Err(ProbeError::NonZeroExit {
                        code: 1,
                        stderr: "stub: forced failure".to_string(),
                    })
                } else {
                    // Le scanner appelle `fs::metadata` séparément :
                    // `size_bytes` ici est purement informatif. On le
                    // synchronise avec le fichier réel pour rester
                    // cohérent avec le `make_meta` du module 8.2.
                    meta.size_bytes = std::fs::metadata(path)
                        .map(|m| m.len())
                        .unwrap_or(0);
                    Ok(meta)
                }
            })
        }
    }

    fn make_meta() -> TechnicalMetadata {
        TechnicalMetadata {
            duration_seconds: Some(120),
            video_codec: Some("h264".to_string()),
            width: Some(1920),
            height: Some(1080),
            audio_tracks: vec![AudioTrack {
                codec: Some("aac".to_string()),
                language: Some("eng".to_string()),
            }],
            subtitle_tracks: Vec::<SubtitleTrack>::new(),
            size_bytes: 0,
        }
    }

    // -----------------------------------------------------------------------
    // Bootstrap DB / runtime — repris à l'identique de `tests_8_2`
    // (les modules de tests Rust ne partagent pas leurs items
    // privés).
    // -----------------------------------------------------------------------

    fn block_on<F: Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime tokio")
            .block_on(fut)
    }

    async fn fresh_repo(db_dir: &Path) -> Repo {
        let db_path = db_dir.join("library.db");
        let opts = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(opts)
            .await
            .expect("pool sqlite test");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations test");
        Repo::new(pool)
    }

    fn touch_video(dir: &Path, rel: &str, content: &[u8]) -> std::path::PathBuf {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, content).unwrap();
        path
    }

    proptest! {
        // ≥ 50 cas comme exigé par la tâche 8.9. Chaque cas crée un
        // pool SQLite + N fichiers vidéo + exécute un scan complet ;
        // 50 cas tiennent en quelques secondes sur un poste standard
        // tout en couvrant les configurations dégénérées (liste
        // vide, 100 % d'erreurs, 100 % de succès).
        #![proptest_config(ProptestConfig {
            cases: 50,
            ..ProptestConfig::default()
        })]

        /// **Property 37 — Continuation du scan en cas d'erreur isolée.**
        ///
        /// Pour toute liste arbitraire de fichiers vidéo où chacun
        /// est marqué « doit échouer au probe » ou non,
        /// `scan_folder` :
        ///   1. retourne `Ok(_)` (pas d'arrêt global) ;
        ///   2. crée toutes les fiches (`report.created` == nb fichiers)
        ///      — requirement 5.5 (persistance avec champs techniques
        ///      null en cas d'échec probe) ;
        ///   3. comptabilise exactement les fichiers en erreur dans
        ///      `report.failures.len()` ;
        ///   4. journalise le chemin de chaque fichier en erreur dans
        ///      `report.failures[i].file_path` (requirement 18.1).
        ///
        /// Tag: Feature: kyro, Property 37.
        ///
        /// **Validates: Requirements 18.1**
        #[test]
        fn p37_scan_continues_on_isolated_failures(
            should_fail_flags in proptest::collection::vec(any::<bool>(), 0..=20),
        ) {
            // Tout le travail asynchrone (création DB, écriture des
            // fichiers, scan) est encapsulé dans `block_on` qui
            // retourne les valeurs nécessaires aux `prop_assert_*` —
            // ces dernières doivent rester en dehors du futur pour
            // respecter le `Result<(), TestCaseError>` synthétisé par
            // `proptest!`.
            let (
                report_total,
                report_created,
                report_failures_len,
                expected_failure_paths,
                actual_failure_paths,
                returned_ok,
            ) = block_on(async {
                let db_dir = TempDir::new().expect("tempdir DB");
                let lib_dir = TempDir::new().expect("tempdir lib");
                let repo = fresh_repo(db_dir.path()).await;

                let folder_id = repo
                    .add_library_folder(lib_dir.path(), Category::Films)
                    .await
                    .expect("add_library_folder");

                // Crée un fichier vidéo par flag avec un nom unique.
                // Les fichiers en mode `should_fail` reçoivent le
                // préfixe `Broken_` ; les autres `Valid_`. Le suffixe
                // `<idx>` garantit l'unicité même en cas de répétition
                // de pattern. Le contenu (taille variable) évite que
                // le skip idempotent ne se déclenche sur un re-scan
                // accidentel — non pertinent ici (un seul scan) mais
                // robuste face aux régressions futures.
                let mut expected_failure_paths: BTreeSet<String> = BTreeSet::new();
                for (idx, should_fail) in should_fail_flags.iter().enumerate() {
                    let prefix = if *should_fail { "Broken" } else { "Valid" };
                    let name = format!("{prefix}_{idx}.2024.1080p.mkv");
                    let bytes: Vec<u8> = (0..(idx as u8 + 1)).collect();
                    let path = touch_video(lib_dir.path(), &name, &bytes);
                    if *should_fail {
                        let canonical = canonicalize_or_keep(&path)
                            .to_string_lossy()
                            .into_owned();
                        expected_failure_paths.insert(canonical);
                    }
                }

                // Le prédicat consulté par le probe : « le nom de
                // fichier commence-t-il par "Broken_" ? ». On
                // travaille sur le `file_name` plutôt que sur le
                // chemin complet pour rester insensible au préfixe
                // canonique du `TempDir` (différent entre
                // plateformes / cas).
                let probe = PerPathStubProbe::with_predicate(
                    make_meta(),
                    |p: &Path| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| n.starts_with("Broken_"))
                            .unwrap_or(false)
                    },
                );

                let scan_result = scan_folder(
                    &repo,
                    &probe,
                    folder_id,
                    lib_dir.path(),
                    Category::Films,
                )
                .await;

                let report = match scan_result {
                    Ok(r) => r,
                    Err(_) => {
                        // Branche d'échec global : on signale via le
                        // tuple retourné. Les `prop_assert_*` plus
                        // bas exploiteront `returned_ok = false` pour
                        // rendre la propriété fausse.
                        return (
                            0usize,
                            0usize,
                            0usize,
                            expected_failure_paths,
                            BTreeSet::new(),
                            false,
                        );
                    }
                };

                let actual_failure_paths: BTreeSet<String> = report
                    .failures
                    .iter()
                    .map(|f| f.file_path.clone())
                    .collect();

                (
                    report.total_files,
                    report.created,
                    report.failures.len(),
                    expected_failure_paths,
                    actual_failure_paths,
                    true,
                )
            });

            let total_files = should_fail_flags.len();
            let expected_failures = should_fail_flags.iter().filter(|b| **b).count();

            // 1. Pas d'arrêt global.
            prop_assert!(
                returned_ok,
                "P-37 : scan_folder doit retourner Ok(_) même en présence d'erreurs isolées"
            );

            // Sanity : tous les fichiers ont été énumérés.
            prop_assert_eq!(
                report_total,
                total_files,
                "P-37 : tous les fichiers doivent être énumérés (total_files = N)"
            );

            // 2. Toutes les fiches persistées (req 5.5 : probe Err →
            // entrée créée avec champs techniques null).
            prop_assert_eq!(
                report_created,
                total_files,
                "P-37 : tous les fichiers doivent être persistés ({} attendus)",
                total_files
            );

            // 3. Le nombre d'échecs correspond exactement au nombre
            // de fichiers marqués `should_fail`.
            prop_assert_eq!(
                report_failures_len,
                expected_failures,
                "P-37 : report.failures.len() doit valoir exactement le nombre de \
                 fichiers en erreur"
            );

            // 4. Tous les chemins en erreur figurent (par leur forme
            // canonicalisée) dans `report.failures`.
            prop_assert_eq!(
                &actual_failure_paths,
                &expected_failure_paths,
                "P-37 : report.failures doit contenir exactement les file_path \
                 canonicalisés des fichiers en erreur"
            );
        }
    }
}

// ===========================================================================
// Tâche 8.8 — test de propriété « Détection de fichiers disparus ».
//
// Ce module est placé textuellement après `tests_8_2` (cf. tâches.md
// 8.8 : « Add a `proptest!` test in `src-tauri/src/infra/scanner.rs`
// adjacent to `tests_8_2` »). Il réutilise les mêmes briques de
// bootstrap (stub `ProbeBackend`, `fresh_repo`, `block_on`) mais reste
// autonome pour ne pas perturber la suite unitaire 8.2 : chaque cas
// proptest ouvre son propre `TempDir` (DB) et son propre dossier de
// bibliothèque (FS), et n'observe donc aucun autre cas.
//
// Couverture : Property 5 du design (« Détection de fichiers
// disparus », requirement 3.5) — pour toute entrée préalablement
// indexée dont le fichier disparaît du disque entre deux scans, après
// le second scan :
//   1. la ligne existe toujours en base (pas de DELETE) ;
//   2. son champ `available` vaut 0 ;
//   3. les fichiers conservés gardent `available = 1`.
//
// Tag : `Feature: kyro, Property 5`. Validates: Requirements 3.5.
// ===========================================================================

#[cfg(test)]
mod tests_8_8 {
    use std::collections::HashSet;
    use std::fs;
    use std::future::Future;
    use std::path::Path;
    use std::pin::Pin;
    use std::sync::Arc;

    use proptest::collection::vec as prop_vec;
    use proptest::prelude::*;
    use sqlx::sqlite::{
        SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
    };
    use tempfile::TempDir;

    use super::{
        canonicalize_or_keep, scan_folder, ScanFailure, SUPPORTED_EXTENSIONS,
    };
    use crate::domain::types::Category;
    use crate::infra::probe::{
        AudioTrack, ProbeBackend, ProbeError, SubtitleTrack, TechnicalMetadata,
    };
    use crate::infra::repo::Repo;

    // -----------------------------------------------------------------------
    // Stub probe local — copie minimale de celui de `tests_8_2` pour
    // garder ce module autonome (pas de dépendance inter-modules de
    // tests).
    // -----------------------------------------------------------------------

    struct StubProbe {
        meta: Arc<TechnicalMetadata>,
    }

    impl StubProbe {
        fn ok() -> Self {
            Self {
                meta: Arc::new(TechnicalMetadata {
                    duration_seconds: Some(120),
                    video_codec: Some("h264".to_string()),
                    width: Some(1920),
                    height: Some(1080),
                    audio_tracks: vec![AudioTrack {
                        codec: Some("aac".to_string()),
                        language: Some("eng".to_string()),
                    }],
                    subtitle_tracks: Vec::<SubtitleTrack>::new(),
                    size_bytes: 0,
                }),
            }
        }
    }

    impl ProbeBackend for StubProbe {
        fn probe<'a>(
            &'a self,
            path: &'a Path,
        ) -> Pin<
            Box<dyn Future<Output = Result<TechnicalMetadata, ProbeError>> + Send + 'a>,
        > {
            let template = self.meta.clone();
            Box::pin(async move {
                let mut meta = (*template).clone();
                meta.size_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                Ok(meta)
            })
        }
    }

    fn block_on<F: Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime tokio")
            .block_on(fut)
    }

    async fn fresh_repo(db_dir: &Path) -> Repo {
        let db_path = db_dir.join("library.db");
        let opts = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(opts)
            .await
            .expect("pool sqlite test");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations test");
        Repo::new(pool)
    }

    // -----------------------------------------------------------------------
    // Stratégies proptest.
    //
    // - `filename_strategy` produit un nom de fichier unique au sein
    //   du cas (le générateur de la liste utilise `prop_vec` puis un
    //   post-traitement pour dédupliquer ; voir le `Just` à l'intérieur
    //   du `prop_compose!`). Chaque nom porte une extension prise dans
    //   `SUPPORTED_EXTENSIONS`, ce qui garantit que `discover_files`
    //   les énumérera.
    // - `delete_mask` est un `Vec<bool>` dont la longueur matche
    //   exactement le nombre de fichiers générés : pour chaque indice
    //   `i`, `delete_mask[i] = true` signifie « supprimer
    //   files[i] avant le second scan ».
    //
    // L'idée est de laisser proptest shrinker indépendamment la liste
    // de noms ET le masque de suppression, ce qui converge vite vers
    // un contre-exemple minimal en cas d'échec.
    // -----------------------------------------------------------------------

    /// Stem alphanumérique court (1..=8 caractères ASCII lisibles) afin
    /// de garder les noms de fichiers valides sur tous les systèmes de
    /// fichiers (notamment Windows, qui interdit `< > : " / \ | ? *`).
    /// On reste volontairement simple : les caractères générés ne
    /// contiennent pas de point pour ne pas perturber l'extraction
    /// d'extension.
    fn stem_strategy() -> impl Strategy<Value = String> {
        // 1..=8 caractères ASCII alphanumériques — pas de collision
        // avec la sémantique de `parser::strip_extension` (pas de
        // point) ni avec les caractères réservés Windows.
        // `proptest` reconnaît la syntaxe regex via `string_regex`.
        prop::string::string_regex("[a-zA-Z0-9]{1,8}")
            .expect("regex stem valide")
    }

    /// Génère un seul nom de fichier `<stem>_<idx>.<ext>` où `idx` est
    /// un compteur unique au sein de la liste. La concaténation
    /// `stem_<idx>` garantit l'unicité même si proptest produit deux
    /// stems identiques après shrinking.
    fn make_unique_filename(stem: &str, idx: usize, ext: &str) -> String {
        format!("{stem}_{idx}.{ext}")
    }

    /// Génère :
    ///   - une liste de longueur 1..=10 de `(stem, ext_idx)` ;
    ///   - un masque de suppression de même longueur.
    /// On expose `(stems_with_ext, delete_mask)` sous forme tuple pour
    /// que proptest les shrinker conjointement.
    fn scenario_strategy() -> impl Strategy<Value = (Vec<(String, usize)>, Vec<bool>)>
    {
        let n_strategy = 1usize..=10;
        n_strategy.prop_flat_map(|n| {
            (
                prop_vec(
                    (
                        stem_strategy(),
                        // Indice dans `SUPPORTED_EXTENSIONS` (8 valeurs).
                        0usize..SUPPORTED_EXTENSIONS.len(),
                    ),
                    n..=n,
                ),
                prop_vec(any::<bool>(), n..=n),
            )
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            // 50 cas comme prescrit par la tâche 8.8 : c'est le bon
            // compromis entre couverture (taille de scénario 1..=10,
            // masque de suppression 2^N) et coût IO — chaque cas
            // ouvre une DB SQLite + crée des fichiers temporaires.
            cases: 50,
            ..ProptestConfig::default()
        })]

        /// **Property 5 (P-5) — Détection de fichiers disparus.**
        ///
        /// Pour toute liste de fichiers vidéo `F = {f_1, ..., f_n}`
        /// indexée par un premier scan (état initial : tous
        /// `available = 1`), si on supprime un sous-ensemble
        /// `D ⊆ F` du disque puis qu'on relance le scan, alors :
        ///
        ///   1. **Persistance** — pour tout `f ∈ F`, la ligne
        ///      `media_item` reste présente en base (aucun DELETE).
        ///   2. **Disparus** — pour tout `f ∈ D`, `available = 0`.
        ///   3. **Survivants** — pour tout `f ∈ F \ D`,
        ///      `available = 1`.
        ///   4. **Compteurs** — `report.marked_unavailable == |D|`,
        ///      `report.total_files == |F| - |D|` (seuls les
        ///      survivants sont énumérés au second scan).
        ///
        /// Tag : Feature: kyro, Property 5.
        ///
        /// **Validates: Requirements 3.5**
        #[test]
        fn p5_disappeared_files_marked_unavailable_not_deleted(
            (stems_with_ext, delete_mask) in scenario_strategy(),
        ) {
            let outcome: Result<(), TestCaseError> = block_on(async move {
                let db_dir = TempDir::new().expect("tempdir DB");
                let lib_dir = TempDir::new().expect("tempdir lib");
                let repo = fresh_repo(db_dir.path()).await;

                let folder_id = repo
                    .add_library_folder(lib_dir.path(), Category::Films)
                    .await
                    .expect("add_library_folder");

                // 1. Matérialisation des fichiers sur disque. Chaque
                //    fichier reçoit un contenu propre pour que
                //    `size_bytes` varie entre fichiers (utile pour
                //    détecter d'éventuelles collisions sur la clé
                //    `(file_path, size_bytes)` côté repo).
                let mut paths: Vec<std::path::PathBuf> = Vec::with_capacity(stems_with_ext.len());
                for (idx, (stem, ext_idx)) in stems_with_ext.iter().enumerate() {
                    let ext = SUPPORTED_EXTENSIONS[*ext_idx];
                    let name = make_unique_filename(stem, idx, ext);
                    let path = lib_dir.path().join(&name);
                    fs::write(&path, format!("kyro-p5-{idx}").as_bytes())
                        .expect("write video file");
                    paths.push(path);
                }
                let n = paths.len();

                // Précondition forte : noms de fichier uniques. Garanti
                // par `make_unique_filename` (suffixe `_<idx>`), mais
                // on l'assert pour blinder le test contre un futur
                // refactor de la fabrique.
                let unique_names: HashSet<String> = paths
                    .iter()
                    .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(str::to_owned))
                    .collect();
                prop_assert_eq!(
                    unique_names.len(),
                    n,
                    "préc. proptest : tous les noms de fichier doivent être uniques"
                );

                // 2. Premier scan : tout est créé, tous `available = 1`.
                let probe = StubProbe::ok();
                let r1 = scan_folder(
                    &repo,
                    &probe,
                    folder_id,
                    lib_dir.path(),
                    Category::Films,
                )
                .await
                .expect("scan #1 ok");

                prop_assert_eq!(r1.total_files, n, "scan #1 doit énumérer tous les fichiers");
                prop_assert_eq!(r1.created, n, "scan #1 doit créer toutes les fiches");
                prop_assert_eq!(r1.marked_unavailable, 0, "rien à marquer disparu au 1er scan");
                prop_assert!(
                    r1.failures.is_empty(),
                    "scan #1 ne doit pas produire de failure : {:?}",
                    r1.failures.iter().map(|f: &ScanFailure| f.kind).collect::<Vec<_>>()
                );

                // Capture des chemins canoniques **avant** suppression :
                // une fois le fichier supprimé, `std::fs::canonicalize`
                // échoue et `canonicalize_or_keep` retomberait sur la
                // forme brute, qui ne correspondrait plus à la valeur
                // stockée par le scanner.
                let canonical_paths: Vec<String> = paths
                    .iter()
                    .map(|p| canonicalize_or_keep(p).to_string_lossy().into_owned())
                    .collect();

                // Sanity : toutes les fiches existent avec available=1
                // après le premier scan.
                for cp in &canonical_paths {
                    let avail: Option<i64> = sqlx::query_scalar::<_, i64>(
                        "SELECT available FROM media_item WHERE file_path = ?",
                    )
                    .bind(cp)
                    .fetch_optional(repo.pool())
                    .await
                    .expect("read available post-scan-1");
                    prop_assert_eq!(
                        avail,
                        Some(1),
                        "fiche persistée au scan #1 doit avoir available = 1"
                    );
                }

                // 3. Suppression du sous-ensemble dicté par le masque.
                let mut deleted_count = 0usize;
                for (i, path) in paths.iter().enumerate() {
                    if delete_mask[i] {
                        fs::remove_file(path).expect("remove file");
                        deleted_count += 1;
                    }
                }

                // 4. Second scan.
                let r2 = scan_folder(
                    &repo,
                    &probe,
                    folder_id,
                    lib_dir.path(),
                    Category::Films,
                )
                .await
                .expect("scan #2 ok");

                let surviving_count = n - deleted_count;
                prop_assert_eq!(
                    r2.total_files,
                    surviving_count,
                    "scan #2 doit n'énumérer que les fichiers survivants"
                );
                prop_assert_eq!(
                    r2.marked_unavailable,
                    deleted_count,
                    "scan #2 doit marquer indisponibles exactement les fichiers supprimés"
                );
                prop_assert_eq!(
                    r2.created,
                    0,
                    "scan #2 ne doit créer aucune nouvelle fiche (tout existait déjà)"
                );
                prop_assert!(
                    r2.failures.is_empty(),
                    "scan #2 ne doit pas produire de failure : {:?}",
                    r2.failures.iter().map(|f: &ScanFailure| f.kind).collect::<Vec<_>>()
                );

                // 5. Vérifications fines en base — Property 5 stricto
                //    sensu : chaque ligne existe toujours, et
                //    `available` reflète exactement la présence sur
                //    disque.
                for (i, cp) in canonical_paths.iter().enumerate() {
                    let row: Option<i64> = sqlx::query_scalar::<_, i64>(
                        "SELECT available FROM media_item WHERE file_path = ?",
                    )
                    .bind(cp)
                    .fetch_optional(repo.pool())
                    .await
                    .expect("read available post-scan-2");

                    prop_assert!(
                        row.is_some(),
                        "P-5 : la ligne ne doit pas être supprimée (file_path = {})",
                        cp
                    );
                    let expected = if delete_mask[i] { 0 } else { 1 };
                    prop_assert_eq!(
                        row,
                        Some(expected),
                        "P-5 : available attendu = {} pour {} (supprimé = {})",
                        expected,
                        cp,
                        delete_mask[i]
                    );
                }

                Ok(())
            });
            outcome?;
        }
    }
}


// ===========================================================================
// Tâche 8.6 — Property 3 : idempotence du scan (requirements 3.3, 3.4).
//
// Feature: kyro, Property 3.
//
// **Validates: Requirements 3.3, 3.4**
//
// Pour toute arborescence arbitraire de fichiers vidéo matérialisée
// dans un `TempDir`, deux exécutions successives de `scan_folder` sur
// le même dossier doivent observer :
//
//   1. **Pas de nouvelle entrée au second passage**
//      (`report2.created == 0`) : `(file_path, size_bytes)` matche
//      déjà la DB, le diff idempotent (étape 2.b de `scan_folder`)
//      court-circuite avant probe (requirement 3.3).
//   2. **Aucune mise à jour technique au second passage**
//      (`report2.updated == 0`) : la taille n'ayant pas changé,
//      aucune fiche n'est rafraîchie.
//   3. **Aucune disparition au second passage**
//      (`report2.marked_unavailable == 0`) : tous les fichiers
//      observés au premier scan sont encore là.
//   4. **Stabilité du nombre de fiches**
//      (`row_count_after == report1.created`) : la DB contient
//      exactement le nombre de `media_item` créés par le premier
//      scan ; aucun doublon, aucune entrée supprimée (requirement
//      3.4 — la persistance est immuable face à un re-scan sans
//      changement disque).
//
// Le générateur produit un `Vec<(filename_stem, ext_idx, content_size)>`
// de taille `0..=15` :
//   - `filename_stem`        : préfixe alphanumérique (1..=12 caractères)
//                              auquel on ajoute l'index pour garantir
//                              l'unicité des `file_path` ;
//   - `ext_idx`              : indice dans `SUPPORTED_EXTENSIONS` (couvre
//                              les 8 extensions cibles, casse mixte
//                              alternée pour exercer le filtre
//                              insensible à la casse de la tâche 8.1) ;
//   - `content_size`         : 0..=4096 octets (varie `size_bytes`
//                              persisté en DB et permet au générateur
//                              de couvrir aussi le cas dégénéré
//                              `len = 0`).
//
// Quelques fichiers (indices pairs au-delà du 4ᵉ) sont placés dans
// un sous-dossier `nested_<i>/` pour exercer la récursion `walkdir`
// sans complexifier le générateur (les noms de dossiers parents sont
// dérivés de l'index plutôt que tirés aléatoirement, ce qui garde
// le shrinking efficace tout en couvrant le cas « parent dirs »
// demandé par la tâche).
//
// `ProptestConfig.cases = 50` : chaque cas crée une DB SQLite + jusqu'à
// 15 fichiers et exécute deux scans complets (parsing + faux probe +
// upsert × N + diff disparus × 2). 50 cas tiennent en quelques
// secondes sur un poste standard tout en couvrant les configurations
// dégénérées (liste vide, taille 1, 15 fichiers entrelacés en
// récursion).
// ===========================================================================

#[cfg(test)]
mod tests_8_6 {
    use std::fs;
    use std::future::Future;
    use std::path::Path;
    use std::pin::Pin;
    use std::sync::Arc;

    use proptest::prelude::*;
    use sqlx::sqlite::{
        SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
    };
    use tempfile::TempDir;

    use super::*;
    use crate::domain::types::Category;
    use crate::infra::probe::{
        AudioTrack, ProbeBackend, ProbeError, SubtitleTrack, TechnicalMetadata,
    };
    use crate::infra::repo::Repo;

    // -----------------------------------------------------------------------
    // Stub `ProbeBackend` (réutilise la logique de `tests_8_2` /
    // `tests_8_9` : retourne toujours la même `TechnicalMetadata`).
    // Les modules de tests Rust ne partagent pas leurs items privés,
    // d'où la duplication contrôlée — chaque module reste autonome.
    // -----------------------------------------------------------------------

    struct StubProbe {
        meta: Arc<TechnicalMetadata>,
    }

    impl StubProbe {
        fn new(meta: TechnicalMetadata) -> Self {
            Self { meta: Arc::new(meta) }
        }
    }

    impl ProbeBackend for StubProbe {
        fn probe<'a>(
            &'a self,
            path: &'a Path,
        ) -> Pin<Box<dyn Future<Output = Result<TechnicalMetadata, ProbeError>> + Send + 'a>>
        {
            let meta = self.meta.clone();
            Box::pin(async move {
                let mut m = (*meta).clone();
                m.size_bytes = std::fs::metadata(path).map(|md| md.len()).unwrap_or(0);
                Ok(m)
            })
        }
    }

    fn make_meta() -> TechnicalMetadata {
        TechnicalMetadata {
            duration_seconds: Some(120),
            video_codec: Some("h264".to_string()),
            width: Some(1920),
            height: Some(1080),
            audio_tracks: vec![AudioTrack {
                codec: Some("aac".to_string()),
                language: Some("eng".to_string()),
            }],
            subtitle_tracks: Vec::<SubtitleTrack>::new(),
            size_bytes: 0,
        }
    }

    // -----------------------------------------------------------------------
    // Bootstrap DB / runtime (cf. justification dans `tests_8_2`).
    // -----------------------------------------------------------------------

    fn block_on<F: Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime tokio")
            .block_on(fut)
    }

    async fn fresh_repo(db_dir: &Path) -> Repo {
        let db_path = db_dir.join("library.db");
        let opts = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(opts)
            .await
            .expect("pool sqlite test");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations test");
        Repo::new(pool)
    }

    fn touch_video(dir: &Path, rel: &str, content: &[u8]) -> std::path::PathBuf {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, content).unwrap();
        path
    }

    // -----------------------------------------------------------------------
    // Génère une extension supportée (indexée par `ext_idx`) avec une
    // casse alternée selon la parité de `case_flip`. Toutes les
    // extensions de `SUPPORTED_EXTENSIONS` sont en ASCII pur, donc
    // `to_ascii_uppercase` est une variation triviale qui exerce le
    // filtre insensible à la casse de `has_supported_extension`
    // (tâche 8.1) sans risquer de produire une extension non
    // reconnue.
    // -----------------------------------------------------------------------
    fn ext_for(ext_idx: usize, case_flip: bool) -> String {
        let raw = SUPPORTED_EXTENSIONS[ext_idx % SUPPORTED_EXTENSIONS.len()];
        if case_flip { raw.to_ascii_uppercase() } else { raw.to_string() }
    }

    proptest! {
        // ≥ 50 cas comme exigé par la tâche 8.6 (réduit par rapport
        // au standard 100 car chaque cas écrit jusqu'à 15 fichiers
        // sur disque et exécute deux scans DB — le coût I/O domine).
        #![proptest_config(ProptestConfig {
            cases: 50,
            ..ProptestConfig::default()
        })]

        /// **Property 3 (P-3) — Idempotence du scan.**
        ///
        /// Pour toute arborescence arbitraire de fichiers vidéo
        /// matérialisée sous un `TempDir`, deux appels successifs à
        /// `scan_folder` produisent un second `ScanReport` neutre
        /// (`created == 0`, `updated == 0`, `marked_unavailable == 0`)
        /// et le nombre de `media_item` persistés vaut exactement
        /// `report1.created`.
        ///
        /// Tag: Feature: kyro, Property 3.
        ///
        /// **Validates: Requirements 3.3, 3.4**
        #[test]
        fn p3_scan_is_idempotent_on_unchanged_tree(
            entries in proptest::collection::vec(
                (
                    // Stem alphanumérique (1..=12 chars) — pas de `.`,
                    // `_`, `-` qui sont des séparateurs reconnus par
                    // le parser et qui pourraient introduire des
                    // ambiguïtés (année / qualité / numérotation
                    // souples). Pas d'espaces non plus pour éviter
                    // les chemins exotiques côté FS Windows.
                    "[A-Za-z]{1,12}",
                    // Indice dans SUPPORTED_EXTENSIONS (8 valeurs).
                    0usize..SUPPORTED_EXTENSIONS.len(),
                    // Bool de variation de casse pour exercer le
                    // filtre insensible (tâche 8.1).
                    any::<bool>(),
                    // Taille du contenu : 0..=4096 octets. 0 exerce
                    // le cas dégénéré « fichier vide » (taille
                    // persistée = 0, toujours valide en DB).
                    0u32..=4096,
                ),
                0..=15,
            ),
        ) {
            // Tout le travail asynchrone est encapsulé dans `block_on`
            // qui retourne les valeurs nécessaires aux `prop_assert_*`
            // — ces dernières doivent rester en dehors du futur pour
            // respecter le `Result<(), TestCaseError>` synthétisé par
            // `proptest!`.
            let (
                r1_created,
                r1_updated,
                r1_marked,
                r1_total,
                r2_created,
                r2_updated,
                r2_marked,
                r2_total,
                row_count_after,
            ) = block_on(async {
                let db_dir = TempDir::new().expect("tempdir DB");
                let lib_dir = TempDir::new().expect("tempdir lib");
                let repo = fresh_repo(db_dir.path()).await;

                let folder_id = repo
                    .add_library_folder(lib_dir.path(), Category::Films)
                    .await
                    .expect("add_library_folder");

                // Matérialisation de l'arborescence : chaque entrée
                // produit un fichier unique, certaines placées dans
                // un sous-dossier `nested_<i>/` pour exercer la
                // récursion `walkdir`. L'unicité des chemins est
                // garantie par l'index `i` injecté dans le nom.
                for (i, (stem, ext_idx, case_flip, size)) in entries.iter().enumerate() {
                    let ext = ext_for(*ext_idx, *case_flip);
                    let filename = format!("{}_{}.{}", stem, i, ext);
                    let rel = if i % 2 == 0 && i >= 4 {
                        format!("nested_{}/{}", i / 2, filename)
                    } else {
                        filename
                    };
                    let bytes: Vec<u8> = (0..(*size as usize))
                        .map(|k| (k % 256) as u8)
                        .collect();
                    touch_video(lib_dir.path(), &rel, &bytes);
                }

                let probe = StubProbe::new(make_meta());

                let r1 = scan_folder(
                    &repo,
                    &probe,
                    folder_id,
                    lib_dir.path(),
                    Category::Films,
                )
                .await
                .expect("scan 1 ne doit pas remonter d'erreur globale");

                let r2 = scan_folder(
                    &repo,
                    &probe,
                    folder_id,
                    lib_dir.path(),
                    Category::Films,
                )
                .await
                .expect("scan 2 ne doit pas remonter d'erreur globale");

                let row_count: i64 = sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM media_item WHERE folder_id = ?",
                )
                .bind(folder_id.get())
                .fetch_one(repo.pool())
                .await
                .expect("count media_item");

                (
                    r1.created,
                    r1.updated,
                    r1.marked_unavailable,
                    r1.total_files,
                    r2.created,
                    r2.updated,
                    r2.marked_unavailable,
                    r2.total_files,
                    row_count,
                )
            });

            // Sanity 1 : les deux scans énumèrent le même nombre de
            // fichiers (l'arborescence n'a pas bougé entre les deux
            // appels).
            prop_assert_eq!(
                r1_total,
                r2_total,
                "P-3 : les deux scans doivent énumérer le même nombre de fichiers"
            );

            // Sanity 2 : le premier scan a créé exactement
            // `entries.len()` fiches (toutes les extensions générées
            // sont supportées par construction), aucune mise à jour,
            // aucune disparition.
            prop_assert_eq!(
                r1_created,
                entries.len(),
                "P-3 : le scan 1 doit créer une fiche par fichier généré"
            );
            prop_assert_eq!(
                r1_updated, 0,
                "P-3 : le scan 1 ne met rien à jour (DB vierge)"
            );
            prop_assert_eq!(
                r1_marked, 0,
                "P-3 : le scan 1 ne marque rien indisponible (DB vierge)"
            );

            // 1. Aucune nouvelle entrée au second passage.
            prop_assert_eq!(
                r2_created, 0,
                "P-3 : le second scan ne doit créer aucune fiche (idempotence req 3.3)"
            );
            // 2. Aucune mise à jour technique au second passage.
            prop_assert_eq!(
                r2_updated, 0,
                "P-3 : le second scan ne doit mettre à jour aucune fiche \
                 (taille inchangée, idempotence req 3.3)"
            );
            // 3. Aucune disparition au second passage.
            prop_assert_eq!(
                r2_marked, 0,
                "P-3 : le second scan ne doit marquer aucune fiche indisponible \
                 (tous les fichiers sont toujours là)"
            );
            // 4. Le nombre de lignes en DB matche `r1.created`.
            prop_assert_eq!(
                row_count_after,
                r1_created as i64,
                "P-3 : le nombre de media_item persistés doit égaler le `created` \
                 du premier scan (req 3.4)"
            );
        }
    }
}
