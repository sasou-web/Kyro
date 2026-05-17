//! Sauvegarde / restauration de la bibliothèque Kyro (v1.0).
//!
//! Permet à l'utilisateur d'exporter sa bibliothèque vers un fichier
//! `.kyro-backup.zip` portable, et de la restaurer sur la même machine
//! ou un autre poste. L'archive contient :
//!
//! ```text
//! kyro-backup.zip
//! ├── library.db        ← copie cohérente de la base SQLite (VACUUM INTO)
//! ├── posters/          ← affiches importées via PosterPicker
//! │   ├── media-12.jpg
//! │   └── episode-345.png
//! └── manifest.json     ← version applicative + horodatage + nb. d'items
//! ```
//!
//! # Cohérence de la copie SQLite
//!
//! On utilise [`VACUUM INTO`](https://www.sqlite.org/lang_vacuum.html#vacuuminto)
//! qui est **atomique** : SQLite produit un fichier indépendant en
//! tenant compte des transactions WAL en cours, sans bloquer les
//! écrivains. Le fichier résultant est défragmenté (compact) et
//! parfaitement portable.
//!
//! C'est strictement supérieur à un simple `std::fs::copy(library.db)`
//! qui pourrait produire un fichier corrompu si une transaction WAL
//! est en cours d'écriture au moment du copy.
//!
//! # Restauration
//!
//! La restauration **remplace intégralement** la base courante :
//!
//! 1. Validation du contenu de l'archive (présence `library.db` +
//!    `manifest.json`).
//! 2. Backup automatique de l'état courant vers
//!    `library.db.before-restore-<timestamp>` (sécurité utilisateur :
//!    si la restauration est faite par erreur on peut revenir).
//! 3. Extraction du `library.db` vers le chemin courant.
//! 4. Extraction des affiches dans `posters_dir/`.
//! 5. Application des migrations sur la base restaurée pour la mettre
//!    à jour si elle vient d'une version antérieure.
//!
//! # Note
//!
//! L'archive ne contient **pas** les fichiers vidéo eux-mêmes. La
//! restauration ne marche que si les chemins absolus stockés en DB
//! pointent vers des fichiers existants sur la nouvelle machine.
//! L'UI doit prévenir l'utilisateur de ce comportement (les fichiers
//! marqués `available = 0` après restore peuvent être restaurés via
//! un scan complet si les chemins matchent).

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use crate::error::KyroError;

/// Nom de fichier interne du dump SQLite dans l'archive ZIP.
const DB_ENTRY_NAME: &str = "library.db";

/// Préfixe des entrées d'affiches dans l'archive.
const POSTERS_ENTRY_PREFIX: &str = "posters/";

/// Nom du manifest dans l'archive.
const MANIFEST_ENTRY_NAME: &str = "manifest.json";

/// Manifest embarqué dans chaque archive de sauvegarde. Permet à la
/// restauration de vérifier la version applicative qui a produit le
/// backup et d'afficher des métadonnées dans l'UI (date du backup,
/// nombre d'items pour confirmation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifest {
    /// Version sémantique de Kyro qui a produit l'archive.
    pub kyro_version: String,
    /// Horodatage Unix (secondes UTC) de la création du backup.
    pub created_at_unix: i64,
    /// Nombre total de Media_Item dans la base au moment du backup.
    pub media_count: i64,
    /// Nombre total d'Episode_Item.
    pub episode_count: i64,
    /// Nombre d'affiches incluses dans l'archive.
    pub posters_count: usize,
}

/// Crée un backup complet de la bibliothèque dans `dest_archive`.
///
/// Étapes :
/// 1. `VACUUM INTO` un fichier temporaire qui devient la copie de la
///    base.
/// 2. Création de l'archive ZIP cible avec compression `Deflated`.
/// 3. Ajout de la copie DB + de tous les fichiers de `posters_dir`.
/// 4. Ajout du manifest JSON.
/// 5. Cleanup du fichier temporaire.
///
/// `dest_archive` doit être un chemin absolu vers un fichier qui sera
/// créé (ou écrasé). Le dossier parent doit exister.
pub async fn export_backup(
    pool: &SqlitePool,
    posters_dir: &Path,
    dest_archive: &Path,
) -> Result<BackupManifest, KyroError> {
    // 1. VACUUM INTO un fichier temporaire à côté de la cible.
    //    SQLite garantit que le fichier produit est cohérent même
    //    si des transactions WAL sont en cours.
    let parent = dest_archive.parent().ok_or_else(|| {
        KyroError::Validation(format!(
            "le chemin {} n'a pas de dossier parent",
            dest_archive.display()
        ))
    })?;
    std::fs::create_dir_all(parent)?;

    let tmp_db = parent.join(format!(
        ".kyro-backup-tmp-{}.db",
        std::process::id()
    ));
    // Best-effort cleanup d'un éventuel résidu de lancement précédent.
    let _ = std::fs::remove_file(&tmp_db);

    // VACUUM INTO accepte un littéral string SQL. On échappe les `'`
    // dans le chemin (rare sur Windows mais ~). Sous Windows on a aussi
    // les backslashes : SQLite les accepte tels quels dans un literal,
    // pas besoin de traitement spécifique.
    let tmp_str = tmp_db.to_string_lossy().replace('\'', "''");
    let vacuum_sql = format!("VACUUM INTO '{}'", tmp_str);
    sqlx::query(&vacuum_sql).execute(pool).await?;

    // Lecture des métadonnées pour le manifest.
    let media_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM media_item")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let episode_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM episode_item")
        .fetch_one(pool)
        .await
        .unwrap_or(0);

    // 2. Création du writer ZIP.
    let file = File::create(dest_archive)?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(6));

    // 3. Ajout du dump DB.
    {
        zip.start_file(DB_ENTRY_NAME, options).map_err(zip_to_kyro)?;
        let mut db_file = File::open(&tmp_db)?;
        std::io::copy(&mut db_file, &mut zip)?;
    }

    // 4. Ajout des affiches (si le dossier existe).
    let mut posters_count = 0usize;
    if posters_dir.is_dir() {
        for entry in std::fs::read_dir(posters_dir)? {
            let entry = match entry {
                Ok(e) => e,
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        "Kyro_Backup: échec lecture entrée posters/ — ignorée"
                    );
                    continue;
                }
            };
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let file_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };
            let entry_name = format!("{}{}", POSTERS_ENTRY_PREFIX, file_name);
            zip.start_file(&entry_name, options)
                .map_err(zip_to_kyro)?;
            let mut f = File::open(&path)?;
            std::io::copy(&mut f, &mut zip)?;
            posters_count += 1;
        }
    }

    // 5. Manifest JSON.
    let manifest = BackupManifest {
        kyro_version: env!("CARGO_PKG_VERSION").to_string(),
        created_at_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
        media_count,
        episode_count,
        posters_count,
    };
    zip.start_file(MANIFEST_ENTRY_NAME, options)
        .map_err(zip_to_kyro)?;
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| KyroError::Config(format!("sérialisation manifest: {e}")))?;
    zip.write_all(&manifest_bytes)?;

    zip.finish().map_err(zip_to_kyro)?;

    // Cleanup du fichier temporaire.
    let _ = std::fs::remove_file(&tmp_db);

    tracing::info!(
        dest = %dest_archive.display(),
        media_count = manifest.media_count,
        episode_count = manifest.episode_count,
        posters_count = manifest.posters_count,
        "Kyro_Backup: archive de sauvegarde créée"
    );

    Ok(manifest)
}

/// Restaure une archive de sauvegarde.
///
/// Étapes :
/// 1. Ouvre et valide l'archive (présence `library.db` + `manifest.json`).
/// 2. Lit le manifest pour vérifications préalables.
/// 3. Sauvegarde l'état courant vers
///    `db_path` + ".before-restore-<unix>" (filet de sécurité).
/// 4. Extrait `library.db` vers `db_path`.
/// 5. Vide `posters_dir/` puis extrait les affiches de l'archive.
///
/// **Important** : le caller doit fermer **toutes** les connexions
/// SQLite vers `db_path` avant d'appeler cette fonction (sinon l'OS
/// refuse l'écrasement). En production, la commande Tauri associée
/// drop le pool, restaure, puis demande un reload de la fenêtre pour
/// que le pool soit recréé sur la nouvelle base.
pub fn restore_backup(
    archive_path: &Path,
    db_path: &Path,
    posters_dir: &Path,
) -> Result<BackupManifest, KyroError> {
    // 1. Ouverture archive.
    let file = File::open(archive_path)?;
    let mut archive = ZipArchive::new(file).map_err(zip_to_kyro)?;

    // Validation : entrées requises présentes.
    if archive.by_name(MANIFEST_ENTRY_NAME).is_err() {
        return Err(KyroError::Validation(format!(
            "archive {} : manifest.json absent — ce n'est probablement pas un backup Kyro",
            archive_path.display()
        )));
    }
    if archive.by_name(DB_ENTRY_NAME).is_err() {
        return Err(KyroError::Validation(format!(
            "archive {} : library.db absent — backup invalide",
            archive_path.display()
        )));
    }

    // 2. Lecture manifest.
    let manifest: BackupManifest = {
        let mut entry = archive
            .by_name(MANIFEST_ENTRY_NAME)
            .map_err(zip_to_kyro)?;
        let mut buf = String::new();
        entry.read_to_string(&mut buf)?;
        serde_json::from_str(&buf)
            .map_err(|e| KyroError::Validation(format!("manifest invalide: {e}")))?
    };

    // 3. Backup du state courant (filet de sécurité).
    if db_path.exists() {
        let unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let safety_path = db_path.with_extension(format!("db.before-restore-{unix}"));
        if let Err(err) = std::fs::copy(db_path, &safety_path) {
            tracing::warn!(
                error = %err,
                safety_path = %safety_path.display(),
                "Kyro_Backup: échec création filet de sécurité (ignoré, restore continue)"
            );
        } else {
            tracing::info!(
                safety_path = %safety_path.display(),
                "Kyro_Backup: filet de sécurité créé avant restore"
            );
        }

        // Aussi déplacer les fichiers WAL/SHM associés s'ils existent
        // (sinon SQLite peut être confus à l'ouverture après restore).
        let wal = db_path.with_extension("db-wal");
        let shm = db_path.with_extension("db-shm");
        let _ = std::fs::remove_file(&wal);
        let _ = std::fs::remove_file(&shm);
    }

    // 4. Extraction library.db.
    {
        let mut entry = archive.by_name(DB_ENTRY_NAME).map_err(zip_to_kyro)?;
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = File::create(db_path)?;
        std::io::copy(&mut entry, &mut out)?;
    }

    // 5. Reset du dossier posters/ puis extraction des entrées.
    if posters_dir.exists() {
        // Best-effort : on supprime les fichiers qu'on ne va pas
        // restaurer. On ne supprime pas le dossier lui-même (peut
        // être watché ou contenir des sous-dossiers utilisateur).
        if let Ok(rd) = std::fs::read_dir(posters_dir) {
            for entry in rd.flatten() {
                let p = entry.path();
                if p.is_file() {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
    } else {
        std::fs::create_dir_all(posters_dir)?;
    }

    // Liste des entrées posters dans l'archive.
    let len = archive.len();
    for i in 0..len {
        let mut entry = archive.by_index(i).map_err(zip_to_kyro)?;
        let name = entry.name().to_string();
        if let Some(rel) = name.strip_prefix(POSTERS_ENTRY_PREFIX) {
            if rel.is_empty() {
                continue;
            }
            // Sécurité : refuser tout chemin contenant `..` ou un
            // séparateur (zip slip protection — l'archive vient du
            // disque local mais on défend quand même).
            if rel.contains("..") || rel.contains('/') || rel.contains('\\') {
                tracing::warn!(
                    entry = %name,
                    "Kyro_Backup: entrée poster ignorée (chemin suspect)"
                );
                continue;
            }
            let dest = posters_dir.join(rel);
            let mut out = File::create(&dest)?;
            std::io::copy(&mut entry, &mut out)?;
        }
    }

    tracing::info!(
        archive = %archive_path.display(),
        media_count = manifest.media_count,
        episode_count = manifest.episode_count,
        posters_count = manifest.posters_count,
        kyro_version = %manifest.kyro_version,
        "Kyro_Backup: restauration terminée"
    );

    Ok(manifest)
}

/// Convertit une `zip::result::ZipError` vers `KyroError`. Ces erreurs
/// sont **internes** (corruption d'archive, format invalide) — on les
/// projette en `KyroError::Validation` pour qu'elles remontent
/// proprement vers le frontend.
fn zip_to_kyro(err: zip::result::ZipError) -> KyroError {
    KyroError::Validation(format!("erreur archive ZIP: {err}"))
}

// ---------------------------------------------------------------------------
// Helper de résolution du dossier posters par défaut. Réutilisé par
// les commandes Tauri pour ne pas dupliquer la logique présente dans
// `commands::library::resolve_posters_dir`.
// ---------------------------------------------------------------------------

/// Résout le dossier `posters/` standard sous le data dir applicatif.
/// Crée le dossier s'il n'existe pas.
pub fn resolve_default_posters_dir(app_data_dir: &Path) -> std::io::Result<PathBuf> {
    let p = app_data_dir.join("posters");
    std::fs::create_dir_all(&p)?;
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{
        SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
    };
    use tempfile::TempDir;

    fn block_on<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(fut)
    }

    async fn fresh_pool(db_dir: &Path) -> SqlitePool {
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
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[test]
    fn export_then_restore_round_trips_data() {
        block_on(async {
            // 1. Setup : DB avec un media_item + 1 affiche.
            let db_dir = TempDir::new().unwrap();
            let posters_dir = TempDir::new().unwrap();
            let archive_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;

            sqlx::query(
                "INSERT INTO library_folder (path, category) VALUES (?, 'Films')",
            )
            .bind("C:/test")
            .execute(&pool)
            .await
            .unwrap();

            sqlx::query(
                "INSERT INTO media_item (folder_id, type, file_path, title) \
                 VALUES (1, 'Film', 'C:/test/A.mp4', 'TestFilm')",
            )
            .execute(&pool)
            .await
            .unwrap();

            // Affiche factice.
            std::fs::write(posters_dir.path().join("media-1.jpg"), b"FAKEJPG").unwrap();

            // 2. Export.
            let archive_path = archive_dir.path().join("backup.zip");
            let manifest = export_backup(&pool, posters_dir.path(), &archive_path)
                .await
                .expect("export OK");

            assert!(archive_path.exists());
            assert_eq!(manifest.media_count, 1);
            assert_eq!(manifest.posters_count, 1);

            // 3. Restore vers une nouvelle DB / nouveau dossier posters.
            //    On ferme le pool d'origine pour libérer les locks
            //    Windows.
            pool.close().await;

            let restore_db_dir = TempDir::new().unwrap();
            let restore_posters_dir = TempDir::new().unwrap();
            let restored_db = restore_db_dir.path().join("library.db");

            let restored_manifest = restore_backup(
                &archive_path,
                &restored_db,
                restore_posters_dir.path(),
            )
            .expect("restore OK");

            assert_eq!(restored_manifest.media_count, 1);

            // 4. Vérifie que la DB restaurée est exploitable.
            let restored_pool = SqlitePoolOptions::new()
                .connect_with(
                    SqliteConnectOptions::new()
                        .filename(&restored_db)
                        .create_if_missing(false),
                )
                .await
                .expect("ouverture DB restaurée");

            let title: String =
                sqlx::query_scalar("SELECT title FROM media_item WHERE id = 1")
                    .fetch_one(&restored_pool)
                    .await
                    .expect("media_item retrouvé");
            assert_eq!(title, "TestFilm");

            // L'affiche a été restaurée.
            assert!(restore_posters_dir.path().join("media-1.jpg").is_file());
        });
    }

    #[test]
    fn restore_rejects_invalid_archive() {
        let archive_dir = TempDir::new().unwrap();
        let bogus = archive_dir.path().join("bogus.zip");
        // ZIP vide (pas un backup Kyro).
        let f = File::create(&bogus).unwrap();
        let mut z = ZipWriter::new(f);
        z.start_file::<&str, ()>("foo.txt", SimpleFileOptions::default())
            .unwrap();
        z.write_all(b"hello").unwrap();
        z.finish().unwrap();

        let db = archive_dir.path().join("db");
        let posters = archive_dir.path().join("posters");
        let err =
            restore_backup(&bogus, &db, &posters).expect_err("doit refuser archive non-Kyro");
        assert!(matches!(err, KyroError::Validation(_)));
    }
}
