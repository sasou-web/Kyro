//! Tâche 7.7 — import d'affiche locale.
//!
//! Ce module implémente la copie d'une image fournie par l'utilisateur
//! depuis n'importe quel emplacement du système de fichiers vers le
//! répertoire de données applicatif (`%APPDATA%\Kyro\posters\` en
//! production), suivie de la mise à jour de la colonne `poster_path`
//! dans la table cible (`media_item` ou `episode_item`). Il couvre les
//! requirements 6.2 (édition manuelle, sélection d'un fichier image),
//! 6.3 (copie dans le répertoire de données local de Kyro) et 17.1
//! (persistance locale).
//!
//! ## Choix d'implémentation
//!
//! - **Cible polymorphe ([`PosterTarget`]).** Le DDL expose une colonne
//!   `poster_path` à la fois sur `media_item` et sur `episode_item`
//!   (cf. `migrations/0001_init.sql`). Pour rester cohérent avec le
//!   design § « Affiches copiées : `%APPDATA%\Kyro\posters\<media_id>.<ext>` »
//!   tout en supportant les épisodes, on introduit l'enum
//!   [`PosterTarget`] (volontairement distinct de
//!   [`crate::infra::repo::ProgressTarget`] : la sémantique est
//!   différente — ici on associe une affiche, pas un état de lecture).
//!   Les noms de fichiers sont préfixés par `media-` ou `episode-` pour
//!   éviter toute collision entre les deux espaces d'identifiants
//!   (`media_item.id` et `episode_item.id` sont des AUTOINCREMENT
//!   indépendants ; le préfixe rend l'écriture unique côté FS et
//!   facilite le diagnostic).
//!
//! - **`posters_dir` injectable.** La signature publique
//!   [`import_poster`] prend explicitement un `posters_dir: &Path`
//!   plutôt que de résoudre `%APPDATA%\Kyro\posters\` en interne. Cela
//!   permet aux tests unitaires de cibler un `tempfile::TempDir` sans
//!   toucher au système de fichiers utilisateur, et préserve le
//!   principe d'inversion des dépendances appliqué ailleurs dans la
//!   couche `infra` (cf. `ProbeBackend`, `ffprobe_locator`). Le
//!   `commands` Tauri (tâche 12.x) résoudra le chemin via
//!   `tauri::Manager::path()` puis le passera à cette fonction.
//!
//! - **Validation stricte de l'extension.** Le requirement 6.2 et la
//!   tâche 7.7 imposent l'allowlist `{jpg, jpeg, png, webp}`. La
//!   validation est appliquée **avant** toute opération FS ou DB
//!   (« fail fast ») et le résultat est journalisé via `tracing::warn!`
//!   en cas de refus pour faciliter le diagnostic. La comparaison est
//!   insensible à la casse — l'extension est ramenée en ASCII
//!   lowercase via `to_ascii_lowercase` avant le test.
//!
//! - **Ordre des opérations.** On enchaîne, dans cet ordre :
//!   1. Validation de l'extension du source.
//!   2. Validation de l'existence du fichier source (et que ce n'est
//!      pas un dossier).
//!   3. Validation de l'existence de la cible en DB (lecture seule).
//!   4. Création du dossier `posters_dir` si absent
//!      (`std::fs::create_dir_all`).
//!   5. Copie effective du fichier (`std::fs::copy`).
//!   6. UPDATE `poster_path` dans une transaction unique +
//!      `tx.commit()` — on ne signale succès qu'après commit
//!      (requirement 17.2).
//!
//!   Si l'étape 6 échoue, le fichier copié à l'étape 5 reste sur disque
//!   en orphelin. C'est un compromis assumé : on préfère ne pas
//!   inscrire un `poster_path` qui ne pointerait sur aucun fichier
//!   (ce qui casserait l'affichage de la grille — requirement 15.1).
//!   Un import ultérieur pour le même id écrasera l'orphelin.
//!
//! ## Tests
//!
//! Le module embarque un `#[cfg(test)] mod tests` qui couvre :
//!
//! 1. Succès copie + mise à jour `media_item.poster_path`.
//! 2. Succès copie + mise à jour `episode_item.poster_path`.
//! 3. Refus d'une extension non supportée (`KyroError::Validation`).
//! 4. Refus d'un fichier source absent (`KyroError::Validation`).
//! 5. Refus d'un id de cible inexistant en DB (`KyroError::Validation`).
//! 6. Acceptation des extensions en majuscules (normalisation lowercase).

use std::path::{Path, PathBuf};

use sqlx::SqlitePool;

use crate::domain::types::{EpisodeItemId, MediaItemId};
use crate::error::KyroError;

/// Liste blanche des extensions d'image acceptées pour une affiche
/// locale (requirement 6.2). Comparaison insensible à la casse — les
/// valeurs sont stockées en lowercase pour faciliter la comparaison
/// directe via `slice::contains`.
pub const SUPPORTED_POSTER_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp"];

/// Cible d'un import d'affiche.
///
/// Distingue explicitement un `media_item` (Film, racine de Series ou
/// d'Anime) d'un `episode_item` afin d'éviter toute confusion dans la
/// signature des commandes Tauri (tâche 12.x) et d'aligner la sémantique
/// sur la colonne `poster_path` qui existe sur les deux tables (cf.
/// DDL `migrations/0001_init.sql`).
///
/// L'enum est volontairement distinct de
/// [`crate::infra::repo::ProgressTarget`] : la cible d'import d'affiche
/// n'a pas la même sémantique qu'une cible de mise à jour de
/// progression (requirement 6.3 vs 8.x), et un type dédié évite les
/// confusions au site d'appel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PosterTarget {
    /// Affiche associée à un `media_item` (Film, racine de Series ou
    /// d'Anime).
    Media(MediaItemId),
    /// Affiche associée à un `episode_item` (la colonne existe au
    /// niveau de l'épisode dans le DDL — cf. requirement 6.5).
    Episode(EpisodeItemId),
}

impl PosterTarget {
    /// Préfixe court à utiliser dans le nom du fichier copié, afin
    /// d'éviter toute collision entre les espaces d'identifiants
    /// `media_item` et `episode_item` (les deux sont AUTOINCREMENT
    /// indépendants ; sans préfixe, `MediaItemId(1)` et
    /// `EpisodeItemId(1)` produiraient le même `1.jpg`).
    fn file_prefix(self) -> &'static str {
        match self {
            PosterTarget::Media(_) => "media",
            PosterTarget::Episode(_) => "episode",
        }
    }

    /// Identifiant numérique sous-jacent (clé primaire SQLite).
    fn id_value(self) -> i64 {
        match self {
            PosterTarget::Media(id) => id.get(),
            PosterTarget::Episode(id) => id.get(),
        }
    }
}

/// Importe un fichier image local comme affiche d'un `media_item` ou
/// d'un `episode_item`.
///
/// Étapes (cf. doc de module pour la justification de l'ordre) :
/// 1. Validation de l'extension du `source_path` (allowlist
///    [`SUPPORTED_POSTER_EXTENSIONS`], insensible à la casse).
/// 2. Vérification que `source_path` désigne un fichier existant.
/// 3. Vérification que la cible existe en base (`SELECT id FROM ...`).
/// 4. Création du dossier `posters_dir` si absent.
/// 5. Copie de `source_path` vers `posters_dir/<prefix>-<id>.<ext>`.
/// 6. UPDATE `poster_path` dans une transaction puis `tx.commit()`.
///
/// Retourne le chemin de destination effectivement écrit dans la
/// colonne `poster_path`.
///
/// # Erreurs
///
/// - [`KyroError::Validation`] si l'extension est manquante, hors
///   allowlist, si le fichier source n'existe pas / n'est pas un
///   fichier, ou si l'identifiant cible n'a aucune ligne correspondante.
/// - [`KyroError::Io`] si la création du dossier ou la copie échoue
///   (permissions, disque plein, etc.).
/// - [`KyroError::Db`] si l'UPDATE ou le `COMMIT` échoue.
pub async fn import_poster(
    pool: &SqlitePool,
    target: PosterTarget,
    source_path: &Path,
    posters_dir: &Path,
) -> Result<PathBuf, KyroError> {
    // 1. Validation extension (insensible à la casse).
    let ext = source_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .ok_or_else(|| {
            tracing::warn!(
                path = %source_path.display(),
                "Kyro: import_poster — extension manquante"
            );
            KyroError::Validation(format!(
                "extension d'affiche manquante pour {}",
                source_path.display()
            ))
        })?;
    if !SUPPORTED_POSTER_EXTENSIONS.contains(&ext.as_str()) {
        tracing::warn!(
            path = %source_path.display(),
            ext = %ext,
            "Kyro: import_poster — extension non supportée"
        );
        return Err(KyroError::Validation(format!(
            "extension d'affiche non supportée: {ext} (attendu: jpg, jpeg, png, webp)"
        )));
    }

    // 2. Vérification du fichier source.
    let metadata = std::fs::metadata(source_path).map_err(|err| {
        tracing::warn!(
            error = %err,
            path = %source_path.display(),
            "Kyro: import_poster — source inaccessible"
        );
        KyroError::Validation(format!(
            "fichier source d'affiche introuvable: {}",
            source_path.display()
        ))
    })?;
    if !metadata.is_file() {
        tracing::warn!(
            path = %source_path.display(),
            "Kyro: import_poster — source n'est pas un fichier"
        );
        return Err(KyroError::Validation(format!(
            "le chemin source n'est pas un fichier: {}",
            source_path.display()
        )));
    }

    // 3. Vérification que la cible existe en DB (lecture seule).
    let id_value = target.id_value();
    let exists: Option<i64> = match target {
        PosterTarget::Media(_) => {
            sqlx::query_scalar::<_, i64>("SELECT id FROM media_item WHERE id = ?")
                .bind(id_value)
                .fetch_optional(pool)
                .await?
        }
        PosterTarget::Episode(_) => {
            sqlx::query_scalar::<_, i64>("SELECT id FROM episode_item WHERE id = ?")
                .bind(id_value)
                .fetch_optional(pool)
                .await?
        }
    };
    if exists.is_none() {
        let table = match target {
            PosterTarget::Media(_) => "media_item",
            PosterTarget::Episode(_) => "episode_item",
        };
        tracing::warn!(
            table = table,
            id = id_value,
            "Kyro: import_poster — cible introuvable en base"
        );
        return Err(KyroError::Validation(format!(
            "cible d'affiche introuvable: {table} id={id_value}"
        )));
    }

    // 4. Création du dossier posters_dir si absent.
    std::fs::create_dir_all(posters_dir)?;

    // 5. Copie du fichier source vers <prefix>-<id>.<ext>.
    let dest_filename = format!("{}-{}.{}", target.file_prefix(), id_value, ext);
    let dest = posters_dir.join(&dest_filename);
    std::fs::copy(source_path, &dest)?;

    // 6. UPDATE poster_path dans une transaction unique. On ne
    //    signale succès qu'après commit (requirement 17.2).
    let dest_str = dest.to_string_lossy().into_owned();
    let mut tx = pool.begin().await?;
    let sql = match target {
        PosterTarget::Media(_) => "UPDATE media_item SET poster_path = ? WHERE id = ?",
        PosterTarget::Episode(_) => "UPDATE episode_item SET poster_path = ? WHERE id = ?",
    };
    sqlx::query(sql)
        .bind(&dest_str)
        .bind(id_value)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    tracing::info!(
        target = ?target,
        source = %source_path.display(),
        dest = %dest.display(),
        "Kyro: import_poster — affiche copiée et poster_path mis à jour"
    );

    Ok(dest)
}

// ---------------------------------------------------------------------------
// Tests unitaires.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{
        SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
    };
    use tempfile::TempDir;

    /// Construit un runtime `tokio` `current_thread` dédié au test
    /// (cf. `infra::repo::tests::block_on` pour la même stratégie).
    fn block_on<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("création d'un runtime tokio current_thread")
            .block_on(fut)
    }

    /// Ouvre un pool SQLite sur un fichier temporaire et applique les
    /// migrations versionnées. Reproduit fidèlement la configuration
    /// `init_sqlite_pool` (`main.rs`).
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
            .expect("ouverture du pool SQLite de test");

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("exécution des migrations sur la DB de test");

        pool
    }

    /// Insère un `media_item` Film minimal et retourne son id.
    async fn insert_media_item(pool: &SqlitePool, title: &str) -> MediaItemId {
        let id = sqlx::query_scalar::<_, i64>(
            "INSERT INTO media_item (type, title, available) VALUES ('Film', ?, 1) \
             RETURNING id",
        )
        .bind(title)
        .fetch_one(pool)
        .await
        .expect("insertion media_item de test");
        MediaItemId(id)
    }

    /// Insère une série + un épisode rattaché et retourne l'id de
    /// l'épisode. Couvre la branche `PosterTarget::Episode`.
    async fn insert_episode_item(pool: &SqlitePool, file_path: &str) -> EpisodeItemId {
        let series_id = sqlx::query_scalar::<_, i64>(
            "INSERT INTO media_item (type, title, available) VALUES ('Series', 'S', 1) \
             RETURNING id",
        )
        .fetch_one(pool)
        .await
        .expect("insertion série de test");

        let id = sqlx::query_scalar::<_, i64>(
            "INSERT INTO episode_item ( \
                 series_id, season_number, episode_number, file_path, available \
             ) VALUES (?, 1, 1, ?, 1) RETURNING id",
        )
        .bind(series_id)
        .bind(file_path)
        .fetch_one(pool)
        .await
        .expect("insertion episode_item de test");
        EpisodeItemId(id)
    }

    /// Lit la colonne `poster_path` d'un `media_item`.
    async fn read_media_poster_path(pool: &SqlitePool, id: i64) -> Option<String> {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT poster_path FROM media_item WHERE id = ?",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("lecture media_item.poster_path")
    }

    /// Lit la colonne `poster_path` d'un `episode_item`.
    async fn read_episode_poster_path(pool: &SqlitePool, id: i64) -> Option<String> {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT poster_path FROM episode_item WHERE id = ?",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("lecture episode_item.poster_path")
    }

    /// Crée un fichier image factice (extension donnée) et renvoie son
    /// chemin absolu.
    fn write_fake_image(dir: &Path, name: &str, contents: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, contents).expect("écriture image factice");
        path
    }

    #[test]
    fn import_poster_copies_image_and_updates_media_row() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let scratch = TempDir::new().unwrap();
            let posters_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;

            let media_id = insert_media_item(&pool, "Inception").await;
            let src = write_fake_image(scratch.path(), "cover.jpg", b"\xff\xd8\xff\xe0fake");

            let dest = import_poster(
                &pool,
                PosterTarget::Media(media_id),
                &src,
                posters_dir.path(),
            )
            .await
            .expect("import doit réussir pour un media valide");

            // Le fichier copié existe et son contenu correspond à la
            // source (la copie est binaire fidèle).
            let copied = std::fs::read(&dest).expect("dest lisible");
            let original = std::fs::read(&src).expect("src lisible");
            assert_eq!(copied, original);

            // Le nom du fichier est préfixé `media-<id>.jpg`.
            let expected_name = format!("media-{}.jpg", media_id.get());
            assert_eq!(
                dest.file_name().and_then(|n| n.to_str()),
                Some(expected_name.as_str()),
                "le nom doit suivre le format `media-<id>.<ext>`"
            );

            // La colonne `poster_path` est mise à jour avec ce chemin.
            let stored = read_media_poster_path(&pool, media_id.get()).await;
            assert_eq!(stored.as_deref(), Some(dest.to_string_lossy().as_ref()));
        });
    }

    #[test]
    fn import_poster_copies_image_and_updates_episode_row() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let scratch = TempDir::new().unwrap();
            let posters_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;

            let ep_id = insert_episode_item(&pool, "C:/series/S01E01.mkv").await;
            let src = write_fake_image(scratch.path(), "ep-cover.png", b"\x89PNGfake");

            let dest = import_poster(
                &pool,
                PosterTarget::Episode(ep_id),
                &src,
                posters_dir.path(),
            )
            .await
            .expect("import doit réussir pour un episode valide");

            let expected_name = format!("episode-{}.png", ep_id.get());
            assert_eq!(
                dest.file_name().and_then(|n| n.to_str()),
                Some(expected_name.as_str()),
                "le nom doit suivre le format `episode-<id>.<ext>`"
            );

            let stored = read_episode_poster_path(&pool, ep_id.get()).await;
            assert_eq!(stored.as_deref(), Some(dest.to_string_lossy().as_ref()));
        });
    }

    #[test]
    fn import_poster_creates_posters_dir_if_missing() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let scratch = TempDir::new().unwrap();
            let posters_parent = TempDir::new().unwrap();
            // Sous-dossier `posters/` non créé : `import_poster` doit
            // le matérialiser via `create_dir_all`.
            let posters_dir = posters_parent.path().join("posters");
            assert!(!posters_dir.exists());

            let pool = fresh_pool(db_dir.path()).await;
            let media_id = insert_media_item(&pool, "T").await;
            let src = write_fake_image(scratch.path(), "img.webp", b"webpfake");

            let dest = import_poster(
                &pool,
                PosterTarget::Media(media_id),
                &src,
                &posters_dir,
            )
            .await
            .expect("import doit créer le dossier posters/ si absent");

            assert!(posters_dir.is_dir(), "le dossier posters/ doit avoir été créé");
            assert!(dest.is_file(), "le fichier copié doit exister");
        });
    }

    #[test]
    fn import_poster_accepts_uppercase_extension() {
        block_on(async {
            // `cover.JPEG` doit être accepté ; l'extension stockée est
            // normalisée en lowercase dans le nom de destination.
            let db_dir = TempDir::new().unwrap();
            let scratch = TempDir::new().unwrap();
            let posters_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;

            let media_id = insert_media_item(&pool, "T").await;
            let src = write_fake_image(scratch.path(), "cover.JPEG", b"jpegfake");

            let dest = import_poster(
                &pool,
                PosterTarget::Media(media_id),
                &src,
                posters_dir.path(),
            )
            .await
            .expect("extension JPEG (majuscule) doit être acceptée");

            assert_eq!(
                dest.extension().and_then(|e| e.to_str()),
                Some("jpeg"),
                "l'extension de destination doit être normalisée en lowercase"
            );
        });
    }

    #[test]
    fn import_poster_rejects_unsupported_extension() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let scratch = TempDir::new().unwrap();
            let posters_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;

            let media_id = insert_media_item(&pool, "T").await;
            // BMP / GIF / autres formats ne sont pas dans l'allowlist.
            let src = write_fake_image(scratch.path(), "cover.bmp", b"bmpfake");

            let err = import_poster(
                &pool,
                PosterTarget::Media(media_id),
                &src,
                posters_dir.path(),
            )
            .await
            .expect_err("une extension hors allowlist doit être rejetée");

            match err {
                KyroError::Validation(msg) => {
                    assert!(
                        msg.contains("bmp"),
                        "le message doit citer l'extension refusée (got: {msg:?})"
                    );
                }
                other => panic!("expected Validation, got {other:?}"),
            }

            // La colonne `poster_path` ne doit pas avoir été mutée.
            assert_eq!(read_media_poster_path(&pool, media_id.get()).await, None);
        });
    }

    #[test]
    fn import_poster_rejects_extensionless_source() {
        block_on(async {
            // Edge case explicite : un fichier sans extension (`cover`)
            // ne peut pas être validé contre l'allowlist.
            let db_dir = TempDir::new().unwrap();
            let scratch = TempDir::new().unwrap();
            let posters_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;

            let media_id = insert_media_item(&pool, "T").await;
            let src = write_fake_image(scratch.path(), "cover", b"noextfake");

            let err = import_poster(
                &pool,
                PosterTarget::Media(media_id),
                &src,
                posters_dir.path(),
            )
            .await
            .expect_err("un fichier sans extension doit être rejeté");
            assert!(matches!(err, KyroError::Validation(_)));
        });
    }

    #[test]
    fn import_poster_rejects_missing_source_file() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let scratch = TempDir::new().unwrap();
            let posters_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;

            let media_id = insert_media_item(&pool, "T").await;
            // Chemin inexistant — sous-dossier jamais créé d'un TempDir.
            let bogus = scratch.path().join("does-not-exist.png");

            let err = import_poster(
                &pool,
                PosterTarget::Media(media_id),
                &bogus,
                posters_dir.path(),
            )
            .await
            .expect_err("un fichier source absent doit être rejeté");

            match err {
                KyroError::Validation(msg) => {
                    assert!(
                        msg.contains("does-not-exist"),
                        "le message doit citer le chemin source (got: {msg:?})"
                    );
                }
                other => panic!("expected Validation, got {other:?}"),
            }

            // Aucune écriture FS ni DB.
            assert_eq!(read_media_poster_path(&pool, media_id.get()).await, None);
        });
    }

    #[test]
    fn import_poster_rejects_directory_as_source() {
        block_on(async {
            // Un dossier ne peut pas être copié comme image, même s'il
            // a une extension `.png` dans son nom (cas exotique mais
            // possible côté FS).
            let db_dir = TempDir::new().unwrap();
            let scratch = TempDir::new().unwrap();
            let posters_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;

            let media_id = insert_media_item(&pool, "T").await;
            let dir_with_ext = scratch.path().join("looks-like.png");
            std::fs::create_dir(&dir_with_ext).unwrap();

            let err = import_poster(
                &pool,
                PosterTarget::Media(media_id),
                &dir_with_ext,
                posters_dir.path(),
            )
            .await
            .expect_err("un dossier ne doit pas être accepté");
            assert!(matches!(err, KyroError::Validation(_)));
        });
    }

    #[test]
    fn import_poster_rejects_unknown_target_id() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let scratch = TempDir::new().unwrap();
            let posters_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;

            // Aucun media_item inséré → id 99999 introuvable.
            let src = write_fake_image(scratch.path(), "cover.png", b"pngfake");

            let err = import_poster(
                &pool,
                PosterTarget::Media(MediaItemId(99_999)),
                &src,
                posters_dir.path(),
            )
            .await
            .expect_err("un id de cible inexistant doit être rejeté");

            match err {
                KyroError::Validation(msg) => {
                    assert!(
                        msg.contains("99999") && msg.contains("media_item"),
                        "le message doit citer l'id et la table (got: {msg:?})"
                    );
                }
                other => panic!("expected Validation, got {other:?}"),
            }

            // Aucun fichier ne doit avoir été copié.
            let entries = std::fs::read_dir(posters_dir.path())
                .map(|it| it.count())
                .unwrap_or(0);
            assert_eq!(entries, 0, "aucun fichier ne doit être copié si la cible est invalide");
        });
    }

    #[test]
    fn import_poster_rejects_unknown_episode_target() {
        block_on(async {
            // Symétrie de `import_poster_rejects_unknown_target_id`
            // mais sur la branche `Episode`.
            let db_dir = TempDir::new().unwrap();
            let scratch = TempDir::new().unwrap();
            let posters_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;

            let src = write_fake_image(scratch.path(), "cover.png", b"pngfake");
            let err = import_poster(
                &pool,
                PosterTarget::Episode(EpisodeItemId(42_424)),
                &src,
                posters_dir.path(),
            )
            .await
            .expect_err("un id d'épisode inexistant doit être rejeté");

            match err {
                KyroError::Validation(msg) => {
                    assert!(
                        msg.contains("42424") && msg.contains("episode_item"),
                        "le message doit citer l'id et la table (got: {msg:?})"
                    );
                }
                other => panic!("expected Validation, got {other:?}"),
            }
        });
    }

    #[test]
    fn import_poster_overwrites_existing_destination() {
        block_on(async {
            // Importer deux fois pour le même id doit écraser le
            // fichier précédent et mettre à jour `poster_path` (cas
            // typique : l'utilisateur change l'affiche).
            let db_dir = TempDir::new().unwrap();
            let scratch = TempDir::new().unwrap();
            let posters_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;

            let media_id = insert_media_item(&pool, "T").await;
            let src_first = write_fake_image(scratch.path(), "v1.jpg", b"first");
            let src_second = write_fake_image(scratch.path(), "v2.jpg", b"second");

            let dest1 = import_poster(
                &pool,
                PosterTarget::Media(media_id),
                &src_first,
                posters_dir.path(),
            )
            .await
            .expect("premier import");
            let dest2 = import_poster(
                &pool,
                PosterTarget::Media(media_id),
                &src_second,
                posters_dir.path(),
            )
            .await
            .expect("second import");

            assert_eq!(dest1, dest2, "le chemin de destination est déterministe");
            let bytes = std::fs::read(&dest2).unwrap();
            assert_eq!(bytes, b"second", "le contenu doit refléter la dernière source");
        });
    }
}
