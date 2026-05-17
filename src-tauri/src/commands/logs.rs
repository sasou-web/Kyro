//! Commande `#[tauri::command]` du domaine **journalisation** : exposition
//! au frontend du contenu du fichier de log le plus récent écrit par le
//! `RollingFileAppender` quotidien (`%APPDATA%\Kyro\logs\kyro.log.YYYY-MM-DD`).
//!
//! _Requirements: 18.3_
//!
//! # Architecture
//!
//! Comme pour `playback::install_lua_script`, la commande est scindée
//! en deux niveaux pour rester testable sans `tauri::AppHandle` :
//!
//! * [`read_latest_log_from`] — helper *pure-IO* qui prend un
//!   `logs_dir: &Path` arbitraire, sélectionne le fichier le plus
//!   récent (par `mtime` décroissant) et lit jusqu'à 5 MiB. C'est ce
//!   helper que ciblent les tests unitaires (via `tempfile::TempDir`).
//! * [`read_latest_log`] — `#[tauri::command]` qui résout
//!   `<app_data_dir>/logs` via le `PathResolver` Tauri v2 puis délègue
//!   au helper.
//!
//! # Garde-fous
//!
//! * **Lecture seule** : aucune modification du dossier ni du fichier
//!   cible. Le fichier reste écrit en parallèle par le
//!   `tracing_appender` non-bloquant — l'ouverture en lecture seule via
//!   `std::fs::File::open` n'interfère pas avec le writer.
//! * **Plafond 5 MiB** : un fichier journalier peut grossir si une
//!   boucle d'erreur génère beaucoup d'événements. Le frontend affiche
//!   le résultat dans un `<pre>` ; on tronque à 5 MiB pour éviter de
//!   saturer la WebView et le pont IPC (design § « Page logs »).
//! * **Décodage UTF-8 lossy** : `tracing_subscriber::fmt().json()` écrit
//!   en UTF-8 valide, mais une troncature au milieu d'un caractère
//!   multi-octets reste possible si le fichier dépasse 5 MiB. On
//!   utilise `String::from_utf8_lossy` pour absorber proprement ce cas
//!   (les octets invalides deviennent U+FFFD).
//!
//! La registration de cette commande sur le `tauri::Builder`
//! (`invoke_handler`) est explicitement déléguée à une tâche ultérieure.

use std::io::Read;
use std::path::Path;
use std::time::SystemTime;

use tauri::Manager;

use crate::error::KyroError;

/// Plafond de lecture côté backend : 5 MiB. Au-delà, le contenu est
/// tronqué silencieusement (design § « Page logs »).
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

/// Helper pure-IO : lit le contenu du fichier le plus récent dans
/// `logs_dir`, plafonné à [`MAX_LOG_BYTES`].
///
/// Étapes :
/// 1. Énumère les entrées de `logs_dir` (non récursif). Les
///    sous-dossiers et entrées non-fichiers sont ignorés silencieusement
///    (le `RollingFileAppender` n'écrit qu'à plat).
/// 2. Trie les fichiers par `mtime` décroissant et retient le premier.
///    À défaut de `mtime` disponible (FS exotique), on retombe sur
///    `SystemTime::UNIX_EPOCH` pour rester déterministe.
/// 3. Si `logs_dir` est absent ou ne contient aucun fichier régulier,
///    renvoie une `String` vide — le frontend affichera simplement un
///    `<pre>` vide (état nominal au tout premier lancement, avant que
///    le `tracing_appender` n'ait flushé sa première ligne).
/// 4. Ouvre le fichier sélectionné en lecture seule et lit au plus
///    [`MAX_LOG_BYTES`] octets via `Read::take`. Le handle est libéré à
///    la sortie de fonction (RAII) — pas d'interférence avec le writer.
/// 5. Décode en UTF-8 *lossy* pour tolérer une troncature en milieu de
///    caractère multi-octets et toute corruption ponctuelle.
///
/// Retourne `KyroError::Io` si l'énumération du dossier ou la lecture
/// du fichier sélectionné échouent pour une raison autre que « dossier
/// inexistant ».
pub(crate) fn read_latest_log_from(logs_dir: &Path) -> Result<String, KyroError> {
    // Cas « dossier inexistant » : traité comme « aucun log » plutôt
    // que comme une erreur — c'est l'état attendu au tout premier
    // lancement avant que le `RollingFileAppender` n'ait créé son
    // premier fichier (le dossier lui-même est créé par `main.rs` au
    // setup, mais on reste défensif).
    let read_dir = match std::fs::read_dir(logs_dir) {
        Ok(rd) => rd,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(String::new());
        }
        Err(err) => return Err(KyroError::from(err)),
    };

    // Sélection du fichier le plus récent. On évite `unwrap` sur le
    // `mtime` car certains FS (réseau, anciens) peuvent renvoyer une
    // erreur ; dans ce cas on retombe sur `UNIX_EPOCH` pour garder un
    // ordre total déterministe — le fichier avec un `mtime` valide
    // gagne face à un fichier sans `mtime`.
    let mut newest: Option<(SystemTime, std::path::PathBuf)> = None;
    for entry in read_dir {
        let entry = entry?;
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue, // entrée transitoire ou inaccessible
        };
        if !file_type.is_file() {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let mtime = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let path = entry.path();
        match &newest {
            Some((cur_mtime, _)) if *cur_mtime >= mtime => {}
            _ => newest = Some((mtime, path)),
        }
    }

    let target = match newest {
        Some((_, p)) => p,
        None => return Ok(String::new()),
    };

    // Lecture plafonnée à 5 MiB via `Read::take`. On utilise
    // `OpenOptions::read(true)` explicitement plutôt que `File::open`
    // pour expliciter le contrat « lecture seule ». La cible peut être
    // ouverte simultanément en écriture par le `tracing_appender` —
    // les opérations IO ne se gênent pas (Windows partage en lecture
    // par défaut sur les fichiers ouverts par `tracing_appender`).
    let file = std::fs::OpenOptions::new().read(true).open(&target)?;
    let mut buf = Vec::with_capacity(MAX_LOG_BYTES.min(64 * 1024) as usize);
    file.take(MAX_LOG_BYTES).read_to_end(&mut buf)?;

    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Commande Tauri exposée au frontend : retourne le contenu (UTF-8
/// lossy) du fichier de log le plus récent écrit par le
/// `RollingFileAppender` dans `<app_data_dir>/logs/`.
///
/// Tronqué à [`MAX_LOG_BYTES`] octets (5 MiB) — au-delà, le frontend
/// reçoit les 5 premiers MiB suivis d'éventuels octets de remplacement
/// `U+FFFD` si la troncature tombe au milieu d'un caractère
/// multi-octets.
///
/// _Requirements: 18.3_
#[tauri::command]
pub async fn read_latest_log(app: tauri::AppHandle) -> Result<String, KyroError> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| KyroError::Config(format!("résolution app_data_dir: {e}")))?;
    let logs_dir = app_data_dir.join("logs");
    read_latest_log_from(&logs_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::TempDir;

    /// Dossier vide : le helper renvoie une `String` vide (état
    /// nominal avant la première écriture du `RollingFileAppender`).
    #[test]
    fn returns_empty_string_for_empty_directory() {
        let tmp = TempDir::new().expect("tempdir");
        let out = read_latest_log_from(tmp.path()).expect("read ok");
        assert_eq!(out, "");
    }

    /// Dossier inexistant : assimilé à « aucun log » (renvoie `""`)
    /// plutôt qu'à une erreur — voir commentaire dans
    /// `read_latest_log_from`.
    #[test]
    fn returns_empty_string_for_missing_directory() {
        let tmp = TempDir::new().expect("tempdir");
        let missing = tmp.path().join("does_not_exist");
        let out = read_latest_log_from(&missing).expect("read ok");
        assert_eq!(out, "");
    }

    /// Un seul fichier dans le dossier : son contenu est renvoyé tel
    /// quel.
    #[test]
    fn returns_content_of_single_file() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("kyro.log.2025-01-01");
        std::fs::write(&path, b"line1\nline2\n").expect("seed file");

        let out = read_latest_log_from(tmp.path()).expect("read ok");
        assert_eq!(out, "line1\nline2\n");
    }

    /// Plusieurs fichiers : le helper retourne le contenu du plus
    /// récent (par `mtime`). On force un écart temporel suffisant
    /// (≥ 1 s) pour rester portable même sur des FS dont la résolution
    /// `mtime` est de l'ordre de la seconde (ex. FAT32).
    #[test]
    fn returns_content_of_newest_file_by_mtime() {
        let tmp = TempDir::new().expect("tempdir");
        let older = tmp.path().join("kyro.log.2025-01-01");
        let newer = tmp.path().join("kyro.log.2025-01-02");

        std::fs::write(&older, b"OLDER").expect("seed older");
        // Petite pause pour garantir un `mtime` strictement supérieur
        // sur tous les FS supportés (la résolution la plus grossière
        // observée en pratique sur Windows est ~10 ms ; 1 s couvre
        // FAT32 par sécurité).
        std::thread::sleep(Duration::from_millis(1100));
        std::fs::write(&newer, b"NEWER").expect("seed newer");

        let out = read_latest_log_from(tmp.path()).expect("read ok");
        assert_eq!(out, "NEWER");
    }

    /// Sous-dossiers et entrées non-fichiers ignorés : un sous-dossier
    /// (par exemple un dossier de rotation futur) ne doit pas perturber
    /// la sélection du fichier régulier le plus récent.
    #[test]
    fn ignores_subdirectories() {
        let tmp = TempDir::new().expect("tempdir");
        std::fs::create_dir(tmp.path().join("rotated")).expect("mkdir");
        let log = tmp.path().join("kyro.log.2025-01-01");
        std::fs::write(&log, b"REGULAR").expect("seed");

        let out = read_latest_log_from(tmp.path()).expect("read ok");
        assert_eq!(out, "REGULAR");
    }

    /// Plafond 5 MiB : un fichier strictement plus grand est tronqué
    /// aux 5 premiers MiB. On vérifie la taille en octets après
    /// décodage — le contenu de seed est ASCII (1 octet par caractère)
    /// donc `len()` du `String` vaut bien le nombre d'octets lus.
    #[test]
    fn truncates_file_larger_than_5_mib() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("kyro.log.2025-01-01");

        // 5 MiB + 1024 octets d'ASCII pour pousser la troncature.
        let total = (MAX_LOG_BYTES as usize) + 1024;
        let payload = vec![b'a'; total];
        std::fs::write(&path, &payload).expect("seed large file");

        let out = read_latest_log_from(tmp.path()).expect("read ok");
        assert_eq!(out.len(), MAX_LOG_BYTES as usize);
        // Le préfixe doit correspondre au contenu d'origine.
        assert!(out.bytes().all(|b| b == b'a'));
    }

    /// Plafond 5 MiB : un fichier strictement plus petit n'est pas
    /// tronqué (cas frontière inverse — garantit qu'on ne coupe pas un
    /// fichier journalier normal).
    #[test]
    fn does_not_truncate_file_smaller_than_5_mib() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("kyro.log.2025-01-01");

        // 5 MiB - 1 octet : juste sous le plafond.
        let size = (MAX_LOG_BYTES as usize) - 1;
        let payload = vec![b'b'; size];
        std::fs::write(&path, &payload).expect("seed under-cap file");

        let out = read_latest_log_from(tmp.path()).expect("read ok");
        assert_eq!(out.len(), size);
    }
}
