//! Commandes Tauri pour la sauvegarde / restauration de la
//! bibliothèque (v1.0).
//!
//! Expose deux commandes consommées par la page Paramètres :
//!
//! * [`export_library_backup`] — crée une archive ZIP de la base
//!   SQLite + des affiches importées, à un chemin choisi par
//!   l'utilisateur (sélecteur natif côté frontend).
//! * [`restore_library_backup`] — restaure une archive vers le
//!   `%APPDATA%\Kyro\` courant. **Requiert un redémarrage de
//!   l'application** pour rouvrir la base sur le nouveau fichier ;
//!   c'est explicitement signalé dans la valeur de retour pour que le
//!   frontend affiche un dialogue de confirmation.

use std::path::PathBuf;

use tauri::{Manager, Runtime};

use crate::error::KyroError;
use crate::infra::backup::{self, BackupManifest};

/// Résout le chemin `library.db` standard sous le data dir
/// applicatif (`%APPDATA%\Kyro\library.db` sur Windows).
fn resolve_db_path<R: Runtime>(app: &tauri::AppHandle<R>) -> Result<PathBuf, KyroError> {
    let dir = app.path().app_data_dir().map_err(|e| {
        KyroError::Config(format!("résolution app_data_dir: {e}"))
    })?;
    Ok(dir.join("library.db"))
}

/// Idem pour le dossier `posters/`.
fn resolve_posters_dir<R: Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<PathBuf, KyroError> {
    let dir = app.path().app_data_dir().map_err(|e| {
        KyroError::Config(format!("résolution app_data_dir: {e}"))
    })?;
    backup::resolve_default_posters_dir(&dir).map_err(KyroError::Io)
}

/// Commande Tauri : exporte la bibliothèque vers une archive ZIP.
///
/// Le chemin de destination est fourni par le frontend (typiquement
/// après un dialogue `tauri-plugin-dialog::save`). Le fichier sera
/// créé ou écrasé. Retourne le manifest pour que l'UI puisse afficher
/// un récap (« 42 films, 314 épisodes, 12 affiches sauvegardés »).
#[tauri::command]
pub async fn export_library_backup<R: Runtime>(
    app: tauri::AppHandle<R>,
    pool: tauri::State<'_, sqlx::SqlitePool>,
    dest_path: String,
) -> Result<BackupManifest, KyroError> {
    let posters_dir = resolve_posters_dir(&app)?;
    let dest = PathBuf::from(&dest_path);
    backup::export_backup(pool.inner(), &posters_dir, &dest).await
}

/// Commande Tauri : restaure une archive ZIP vers le data dir courant.
///
/// La fonction ne ferme pas le pool elle-même : c'est l'appelant
/// frontend qui doit appeler `window.location.reload()` après succès
/// pour que Tauri redémarre la fenêtre et rouvre la base. La
/// commande tente quand même un `pool.close()` côté backend pour
/// libérer les locks Windows avant de remplacer le fichier.
#[tauri::command]
pub async fn restore_library_backup<R: Runtime>(
    app: tauri::AppHandle<R>,
    pool: tauri::State<'_, sqlx::SqlitePool>,
    archive_path: String,
) -> Result<BackupManifest, KyroError> {
    let archive = PathBuf::from(&archive_path);
    if !archive.is_file() {
        return Err(KyroError::Validation(format!(
            "l'archive {} n'existe pas ou n'est pas un fichier",
            archive.display()
        )));
    }

    let db_path = resolve_db_path(&app)?;
    let posters_dir = resolve_posters_dir(&app)?;

    // Ferme le pool pour libérer les handles Windows sur library.db
    // avant de l'écraser. Le pool deviendra inutilisable mais sera
    // recréé au prochain boot après reload de la WebView.
    pool.close().await;

    // Restauration synchrone (la fonction est bornée IO disque, pas
    // d'await async).
    let manifest = tokio::task::spawn_blocking(move || {
        backup::restore_backup(&archive, &db_path, &posters_dir)
    })
    .await
    .map_err(|e| KyroError::Config(format!("task spawn: {e}")))??;

    Ok(manifest)
}
