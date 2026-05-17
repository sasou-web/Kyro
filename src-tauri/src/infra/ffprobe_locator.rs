//! Tâche 6.1 — résolveur du binaire `ffprobe` (requirements 5.3, 5.4).
//!
//! Le design § « Stratégie de packaging ffprobe » impose l'ordre de
//! résolution suivant :
//!
//! 1. **Sidecar bundlé** — `tauri.conf.json` déclare
//!    `bundle.externalBin = ["binaries/ffprobe"]` ; Tauri copie le binaire
//!    à côté de l'exécutable installé sous le nom `ffprobe.exe`
//!    (ou `ffprobe-<triple>.exe` selon la configuration de bundling).
//! 2. **Chemin utilisateur** — valeur stockée dans `setting_value` sous la
//!    clé `ffprobe_path` (table définie par la migration 0001_init.sql,
//!    cf. tâche 2.2).
//! 3. **PATH système** — `which::which("ffprobe")`.
//!
//! Si aucun candidat n'est trouvable / exécutable, l'événement Tauri
//! `ffprobe-missing` est émis (le frontend affiche alors l'avertissement
//! persistant prévu au requirement 5.4) et la fonction retourne
//! `Err(KyroError::Config)`.
//!
//! ## Déviation Tauri 2 documentée
//!
//! Le pseudocode du design (lignes 230-232) référence
//! `tauri::api::process::Command::new_sidecar("ffprobe")`. Cette API a
//! disparu en Tauri 2 : la résolution du sidecar passe désormais par le
//! plugin officiel `tauri-plugin-shell` (`app.shell().sidecar(...)`).
//! Activer ce plugin ouvrirait une surface réseau / process supplémentaire
//! et imposerait des permissions de capability équivalentes à
//! `shell:open` — ce que la tâche 1.1 interdit explicitement
//! (« allowlist restreinte (...) shell:open désactivé »).
//!
//! On reproduit donc manuellement la convention de bundling Tauri :
//! `tauri-bundler` copie l'externalBin déclaré à côté de l'exécutable
//! principal sous le nom `ffprobe.exe`. On résout le dossier de l'exe via
//! `std::env::current_exe()` et on teste l'existence des deux noms
//! standards (`ffprobe.exe` et `ffprobe-<triple>.exe`). Cette approche
//! n'ouvre aucune permission supplémentaire et reste compatible avec une
//! migration future vers `tauri-plugin-shell`.
//!
//! ## Conception « pure-ish »
//!
//! La fonction principale [`locate_ffprobe`] est volontairement *pure
//! côté Tauri* : elle ne prend ni `AppHandle` ni `Window` et ne dépend
//! que de l'IO système (`std::env::current_exe`, `which::which`) et d'un
//! foncteur d'accès aux paramètres injecté par l'appelant. Cela permet :
//!
//! - de la tester sans environnement Tauri (cf. module `tests` ci-bas) ;
//! - de différer le câblage repository → `setting_value(key='ffprobe_path')`
//!   à la tâche 7.5 sans rien changer ici ;
//! - d'isoler l'effet de bord « émettre `ffprobe-missing` » dans le
//!   wrapper [`locate_ffprobe_and_emit`], qui est seul à dépendre de
//!   `tauri`.

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Emitter, Runtime};

use crate::error::KyroError;

/// Nom de l'événement Tauri émis lorsque ffprobe est introuvable
/// (requirement 5.4). Le frontend l'écoute via
/// `listen("ffprobe-missing", ...)` (cf. design § « Frontend stores »).
pub const EVENT_FFPROBE_MISSING: &str = "ffprobe-missing";

/// Noms de fichier candidats à tester dans le dossier de l'exécutable
/// pour récupérer le sidecar bundlé. L'ordre est déterministe :
/// `ffprobe.exe` (nom court produit par `tauri-bundler` lors de
/// l'installation) puis le nom triple-suffixé (présent en `cargo run`
/// pendant le développement).
const SIDECAR_CANDIDATES: &[&str] = &[
    "ffprobe.exe",
    "ffprobe-x86_64-pc-windows-msvc.exe",
];

/// Clé de réglage utilisateur stockée dans `setting_value` qui contient
/// le chemin explicite vers `ffprobe.exe` (priorité 2).
///
/// Re-export de la constante canonique [`crate::infra::settings_keys::SETTING_KEY_FFPROBE_PATH`]
/// (tâche 7.5) pour préserver la stabilité de l'API du module
/// `ffprobe_locator` tout en garantissant une seule source de vérité
/// pour le littéral.
pub const SETTING_KEY_FFPROBE_PATH: &str =
    crate::infra::settings_keys::SETTING_KEY_FFPROBE_PATH;

/// Localise le binaire `ffprobe` selon l'ordre design (sidecar →
/// settings → `which`).
///
/// `settings_lookup` est appelé avec la clé `ffprobe_path` (constante
/// [`SETTING_KEY_FFPROBE_PATH`]). Il doit retourner `Some(path)` si la
/// clé est présente dans `setting_value`, `None` sinon. Cette signature
/// permet :
///
/// - aux tests unitaires de fournir un stub direct (`|_| Some("...")`) ;
/// - au futur câblage de la tâche 7.5 d'injecter un appel réel au
///   repository sans dépendre ici de `sqlx`.
///
/// Retourne :
/// - `Ok(PathBuf)` pointant sur un binaire vérifié comme fichier
///   existant ;
/// - `Err(KyroError::Config)` si aucun candidat n'est valide. L'appelant
///   `locate_ffprobe_and_emit` se chargera alors d'émettre l'événement
///   Tauri `ffprobe-missing` (requirement 5.4).
pub fn locate_ffprobe<F>(settings_lookup: F) -> Result<PathBuf, KyroError>
where
    F: FnOnce(&str) -> Option<String>,
{
    let exe_dir = current_exe_dir().ok();
    locate_ffprobe_inner(exe_dir.as_deref(), settings_lookup, which_ffprobe)
}

/// Variante de [`locate_ffprobe`] qui émet l'événement Tauri
/// `ffprobe-missing` en cas d'échec, puis propage l'erreur.
///
/// On garde la résolution pure dans `locate_ffprobe` et on isole ici
/// l'unique effet de bord côté Tauri. Si l'émission elle-même échoue
/// (cas rare : runtime Tauri arrêté), on journalise mais on retourne
/// quand même l'erreur originale `KyroError::Config` pour ne jamais
/// masquer la cause racine au caller.
pub fn locate_ffprobe_and_emit<R, F>(
    app: &AppHandle<R>,
    settings_lookup: F,
) -> Result<PathBuf, KyroError>
where
    R: Runtime,
    F: FnOnce(&str) -> Option<String>,
{
    match locate_ffprobe(settings_lookup) {
        Ok(path) => {
            tracing::info!(
                ffprobe_path = %path.display(),
                "Kyro: binaire ffprobe localisé"
            );
            Ok(path)
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                "Kyro: ffprobe introuvable — émission de l'événement {}",
                EVENT_FFPROBE_MISSING
            );
            if let Err(emit_err) = app.emit(EVENT_FFPROBE_MISSING, ()) {
                tracing::error!(
                    error = %emit_err,
                    "Kyro: échec d'émission de l'événement {}",
                    EVENT_FFPROBE_MISSING
                );
            }
            Err(err)
        }
    }
}

// ---------------------------------------------------------------------------
// Implémentation interne — découpée pour permettre l'injection complète
// des dépendances (dossier de l'exécutable, lookup settings, recherche
// dans le PATH) depuis les tests unitaires.
// ---------------------------------------------------------------------------

/// Cœur testable de la résolution. `exe_dir` peut être `None` quand
/// `std::env::current_exe()` échoue ; dans ce cas on saute la branche
/// sidecar et on passe directement à la branche settings.
fn locate_ffprobe_inner<F, W>(
    exe_dir: Option<&Path>,
    settings_lookup: F,
    which_lookup: W,
) -> Result<PathBuf, KyroError>
where
    F: FnOnce(&str) -> Option<String>,
    W: FnOnce() -> Option<PathBuf>,
{
    // Priorité 1 : sidecar bundlé à côté de l'exécutable principal.
    if let Some(dir) = exe_dir {
        for name in SIDECAR_CANDIDATES {
            let candidate = dir.join(name);
            if is_existing_file(&candidate) {
                return Ok(candidate);
            }
        }
    }

    // Priorité 2 : chemin explicite stocké dans `setting_value`.
    if let Some(raw) = settings_lookup(SETTING_KEY_FFPROBE_PATH) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let candidate = PathBuf::from(trimmed);
            if is_existing_file(&candidate) {
                return Ok(candidate);
            }
        }
    }

    // Priorité 3 : recherche dans le PATH système.
    if let Some(path) = which_lookup() {
        if is_existing_file(&path) {
            return Ok(path);
        }
    }

    Err(KyroError::Config(
        "binaire ffprobe introuvable (sidecar, ffprobe_path, PATH)".to_string(),
    ))
}

/// Retourne le dossier contenant l'exécutable courant. Encapsulé pour
/// pouvoir évoluer vers `tauri::path::PathResolver::resource_dir()` si
/// la convention de bundling change.
fn current_exe_dir() -> std::io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    Ok(exe
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".")))
}

/// Recherche `ffprobe` dans le PATH système. Wrappe `which::which` pour
/// uniformiser la signature `FnOnce() -> Option<PathBuf>` attendue par
/// [`locate_ffprobe_inner`].
fn which_ffprobe() -> Option<PathBuf> {
    which::which("ffprobe").ok()
}

/// Vérifie qu'un chemin pointe sur un fichier existant. On évite
/// `Path::exists()` seul car il accepterait un dossier portant le même
/// nom — ce qui n'arrivera jamais en pratique mais ferait échouer
/// l'invocation `tokio::process::Command` plus tard avec un message
/// moins clair.
fn is_existing_file(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.is_file())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Tests unitaires.
//
// Ils ciblent uniquement `locate_ffprobe_inner` puisque c'est lui qui
// porte la logique de priorité ; l'enveloppe `locate_ffprobe` se contente
// d'injecter `current_exe_dir` et `which_ffprobe`. L'effet de bord
// d'émission Tauri (`locate_ffprobe_and_emit`) nécessite un AppHandle
// runtime — son test est porté par les tests d'intégration de la tâche
// 6.4 (« Tests unitaires pour le parsing ffprobe »).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Crée un fichier vide et exécutable au chemin demandé. Utilisé
    /// pour simuler la présence du sidecar dans le dossier de l'exe.
    fn touch(path: &Path) {
        fs::write(path, b"").expect("touch fixture file");
    }

    fn no_settings(_: &str) -> Option<String> {
        None
    }

    fn no_which() -> Option<PathBuf> {
        None
    }

    #[test]
    fn finds_sidecar_first() {
        let exe_dir = TempDir::new().unwrap();
        let sidecar = exe_dir.path().join("ffprobe.exe");
        touch(&sidecar);

        // Settings et which sont présents mais doivent être ignorés
        // puisque le sidecar a la priorité.
        let other = exe_dir.path().join("from-settings.exe");
        touch(&other);
        let other_path = other.clone();

        let resolved = locate_ffprobe_inner(
            Some(exe_dir.path()),
            move |key| {
                assert_eq!(key, SETTING_KEY_FFPROBE_PATH);
                Some(other_path.to_string_lossy().into_owned())
            },
            || panic!("which ne doit pas être appelé quand le sidecar existe"),
        )
        .expect("sidecar doit être résolu");

        assert_eq!(resolved, sidecar);
    }

    #[test]
    fn finds_sidecar_with_triple_suffix() {
        let exe_dir = TempDir::new().unwrap();
        // Cas dev (`cargo run`) : le binaire conserve son suffixe triple.
        let sidecar = exe_dir
            .path()
            .join("ffprobe-x86_64-pc-windows-msvc.exe");
        touch(&sidecar);

        let resolved = locate_ffprobe_inner(Some(exe_dir.path()), no_settings, no_which)
            .expect("sidecar suffixé doit être résolu");

        assert_eq!(resolved, sidecar);
    }

    #[test]
    fn falls_back_to_settings_when_no_sidecar() {
        // Dossier exe vide => pas de sidecar.
        let exe_dir = TempDir::new().unwrap();
        // Chemin pointé par les settings : un fichier réel ailleurs.
        let elsewhere = TempDir::new().unwrap();
        let user_path = elsewhere.path().join("custom-ffprobe.exe");
        touch(&user_path);

        let user_path_str = user_path.to_string_lossy().into_owned();
        let resolved = locate_ffprobe_inner(
            Some(exe_dir.path()),
            move |key| {
                assert_eq!(key, SETTING_KEY_FFPROBE_PATH);
                Some(user_path_str)
            },
            || panic!("which ne doit pas être appelé quand settings résout"),
        )
        .expect("settings doit résoudre");

        assert_eq!(resolved, user_path);
    }

    #[test]
    fn ignores_blank_settings_value() {
        let exe_dir = TempDir::new().unwrap();
        let elsewhere = TempDir::new().unwrap();
        let which_target = elsewhere.path().join("which-ffprobe.exe");
        touch(&which_target);

        let which_target_clone = which_target.clone();
        let resolved = locate_ffprobe_inner(
            Some(exe_dir.path()),
            // Valeur stockée mais vide / whitespace : on doit l'ignorer
            // et passer à la branche `which`.
            |_| Some("   ".to_string()),
            move || Some(which_target_clone.clone()),
        )
        .expect("which doit prendre le relais");

        assert_eq!(resolved, which_target);
    }

    #[test]
    fn ignores_settings_path_that_does_not_exist() {
        let exe_dir = TempDir::new().unwrap();
        let elsewhere = TempDir::new().unwrap();
        let which_target = elsewhere.path().join("which-ffprobe.exe");
        touch(&which_target);

        let which_target_clone = which_target.clone();
        let resolved = locate_ffprobe_inner(
            Some(exe_dir.path()),
            |_| Some("Z:/nonexistent/ffprobe.exe".to_string()),
            move || Some(which_target_clone.clone()),
        )
        .expect("which doit prendre le relais quand settings pointe sur un fichier inexistant");

        assert_eq!(resolved, which_target);
    }

    #[test]
    fn falls_back_to_which_when_no_sidecar_no_settings() {
        let exe_dir = TempDir::new().unwrap();
        let elsewhere = TempDir::new().unwrap();
        let which_target = elsewhere.path().join("which-ffprobe.exe");
        touch(&which_target);

        let which_target_clone = which_target.clone();
        let resolved = locate_ffprobe_inner(
            Some(exe_dir.path()),
            no_settings,
            move || Some(which_target_clone.clone()),
        )
        .expect("which doit résoudre");

        assert_eq!(resolved, which_target);
    }

    #[test]
    fn returns_config_error_when_all_strategies_fail() {
        let exe_dir = TempDir::new().unwrap();
        let err = locate_ffprobe_inner(Some(exe_dir.path()), no_settings, no_which)
            .expect_err("aucun candidat ne doit être résolu");

        match err {
            KyroError::Config(msg) => {
                assert!(
                    msg.contains("ffprobe"),
                    "le message d'erreur doit citer ffprobe (got: {msg:?})"
                );
            }
            other => panic!("expected KyroError::Config, got {other:?}"),
        }
    }

    #[test]
    fn rejects_directory_named_like_sidecar() {
        // Bord rare : un dossier nommé `ffprobe.exe` ne doit pas être
        // accepté comme binaire valide.
        let exe_dir = TempDir::new().unwrap();
        fs::create_dir(exe_dir.path().join("ffprobe.exe")).unwrap();

        let err = locate_ffprobe_inner(Some(exe_dir.path()), no_settings, no_which)
            .expect_err("un dossier homonyme ne doit pas être accepté");
        assert!(matches!(err, KyroError::Config(_)));
    }

    #[test]
    fn skips_sidecar_when_exe_dir_is_unknown() {
        // Si `current_exe()` échoue, on saute la branche sidecar mais on
        // doit toujours pouvoir résoudre via settings ou which.
        let elsewhere = TempDir::new().unwrap();
        let user_path = elsewhere.path().join("user-ffprobe.exe");
        touch(&user_path);

        let user_path_str = user_path.to_string_lossy().into_owned();
        let resolved =
            locate_ffprobe_inner(None, move |_| Some(user_path_str), no_which)
                .expect("settings doit résoudre même sans exe_dir");

        assert_eq!(resolved, user_path);
    }
}
