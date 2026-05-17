//! Tâche 7.1 — couche `Repository` (sqlx) : CRUD `library_folder`.
//!
//! Ce module porte la première itération de la couche d'accès DB décrite
//! au design § « Repository (sqlx) ». L'objectif de cette tâche est
//! limité aux trois opérations sur la table `library_folder` :
//! `add_library_folder`, `remove_library_folder`, `list_library_folders`
//! (requirements 2.1 → 2.5). Les autres opérations du trait
//! `Repository` (médias, épisodes, settings…) sont apportées par les
//! tâches 7.2 → 7.18.
//!
//! ## Choix d'implémentation
//!
//! - **`SqlitePool` partagé.** La structure publique [`Repo`] détient
//!   un `sqlx::SqlitePool` cloné depuis le pool initialisé dans
//!   `main.rs::init_sqlite_pool`. Le clone est bon marché (Arc interne
//!   sqlx) — les `commands` Tauri pourront récupérer un `&Repo` depuis
//!   le state en plus du pool brut.
//! - **`sqlx::query` runtime, pas `query!`.** Le projet n'utilise
//!   **pas** `DATABASE_URL` à la compile (cf. design § « Garantie zéro
//!   réseau » + Cargo.toml : feature `macros` activée mais aucun
//!   workflow `sqlx prepare`). On utilise donc `sqlx::query`,
//!   `sqlx::query_scalar`, `sqlx::query_as` avec mapping manuel via
//!   `FromRow` ou `try_get`. Cela évite toute dépendance au cache
//!   `.sqlx/` et garde la migration pilotée par le runner runtime
//!   (`sqlx::migrate!`).
//! - **Validation pré-INSERT.** Le requirement 2.4 impose un refus
//!   d'ajout si le chemin n'existe pas sur le système de fichiers.
//!   `add_library_folder` appelle `std::fs::metadata` puis vérifie
//!   `is_dir()` avant tout accès DB ; en cas d'échec on retourne
//!   `KyroError::Validation` en citant le chemin (l'erreur d'origine
//!   éventuelle est journalisée par `tracing::warn!`). Le chemin est
//!   ensuite **canonicalisé** (`std::fs::canonicalize`) pour respecter
//!   le requirement 2.2 (« chemin absolu ») et garantir l'unicité de
//!   `library_folder.path UNIQUE` même si l'utilisateur renseigne
//!   plusieurs variantes relatives du même dossier.
//! - **Transaction sur `remove_library_folder`.** Le requirement 2.3
//!   exige que la suppression d'un dossier marque tous les `Media_Item`
//!   et `Episode_Item` rattachés comme indisponibles, **sans les
//!   supprimer**. On enchaîne dans une seule transaction `pool.begin()`
//!   trois UPDATE/DELETE puis `tx.commit()`. Les deux UPDATE sont
//!   exécutés **avant** le `DELETE` sur `library_folder` afin de
//!   pouvoir résoudre `media_item.folder_id = ?` (la contrainte FK est
//!   `ON DELETE SET NULL`, donc lire après le DELETE perdrait les
//!   lignes).
//!
//! ## Tests
//!
//! Le module embarque un `#[cfg(test)] mod tests` qui crée un pool
//! SQLite temporaire (fichier dans un `TempDir`), applique les
//! migrations versionnées (`sqlx::migrate!`) puis valide :
//!
//! 1. Round-trip d'ajout / lecture (requirement 2.2 + 2.5).
//! 2. Refus d'un chemin inexistant avec `KyroError::Validation`
//!    (requirement 2.4).
//! 3. Retrait : `media_item.available` passe à 0, ligne `library_folder`
//!    supprimée, item NON supprimé (requirement 2.3).
//!
//! Les tests de propriété complets (round-trip arbitraire, retrait
//! marque indisponibles, etc.) sont matérialisés par les tâches 7.8 et
//! 7.9.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

use crate::domain::resume::{self, ResumeAction};
use crate::domain::status::{ParseStatusError, Status};
use crate::domain::types::{Category, EpisodeItemId, MediaItem, MediaItemId, MediaType};
use crate::error::KyroError;

// ---------------------------------------------------------------------------
// Types publics — projetés depuis la table `library_folder`.
// ---------------------------------------------------------------------------

/// Identifiant typé d'une ligne `library_folder`. Wrappe le `INTEGER
/// PRIMARY KEY AUTOINCREMENT` SQLite pour éviter les confusions avec
/// `MediaItemId` ou `EpisodeItemId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FolderId(pub i64);

impl FolderId {
    /// Accès à l'`i64` sous-jacent pour les requêtes paramétrées.
    pub fn get(self) -> i64 {
        self.0
    }
}

/// Projection lecture seule d'une ligne `library_folder`.
///
/// Reproduite à l'identique du DDL `migrations/0001_init.sql` (colonnes
/// `id`, `path`, `category`). La colonne `created_at` n'est pas exposée
/// ici car aucun consommateur de la tâche 7.1 ne l'utilise — elle pourra
/// être ajoutée ultérieurement sans changement de signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryFolder {
    pub id: FolderId,
    pub path: String,
    pub category: Category,
}

// ---------------------------------------------------------------------------
// Inputs d'upsert (tâche 7.2).
//
// Ces types portent l'ensemble des champs **techniques + d'identification**
// que le scanner (tâche 8) connaît au moment de l'insertion ou de la
// mise à jour d'une fiche. Ils sont distincts du `domain::types::MediaItem`,
// qui est une vue minimale dédiée aux fonctions pures `filters` /
// `search`. Le scanner construit ces inputs à partir du résultat
// combiné de `Kyro_Parser` + `Kyro_Probe` + `std::fs::metadata` puis
// les passe à `Repo::upsert_media_item` / `Repo::upsert_episode_item`.
//
// Convention :
//
// * Les champs `Option<...>` correspondent aux colonnes nullables du
//   DDL ou aux valeurs que le probe/parser n'a pas pu extraire (ex.
//   ffprobe absent → `duration_seconds = None`).
// * `numbering_unknown` reflète directement le drapeau du parser
//   (requirement 4.7) ; le scanner remontera `false` pour les Films
//   pour lesquels la numérotation n'a pas de sens.
// * **Champs édités manuellement** (titre, original_title, year,
//   synopsis, studio, user_rating, poster_path, watch_progress_seconds,
//   status, favorite, to_watch) : seuls le `title` et le `year`
//   peuvent être renseignés par le scanner via le parser ; sur un
//   upsert d'une fiche **déjà existante** ces champs ne sont **pas**
//   réécrits par cette méthode (cf. requirement 3.3 — l'entrée
//   existante est conservée). L'override explicite passera plus tard
//   par les commandes `update_media_item` (tâche 12.x).
// ---------------------------------------------------------------------------

/// Données d'entrée pour `upsert_media_item` (tâche 7.2).
///
/// Forme « scanner → repo » : tous les champs techniques nécessaires
/// pour insérer une nouvelle fiche `media_item` ou rafraîchir les
/// champs techniques d'une fiche existante après changement de taille.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpsertMediaInput {
    /// Dossier de bibliothèque parent. `None` ⇒ orpheline (cas `À
    /// classer`, ou retrait ultérieur du dossier qui aura mis le FK à
    /// NULL via `ON DELETE SET NULL`).
    pub folder_id: Option<FolderId>,
    /// Type de média (Film / Series / Anime).
    pub media_type: MediaType,
    /// Chemin absolu sur disque — clé d'unicité (cf. requirement 3.3).
    pub file_path: String,
    /// Taille en octets renvoyée par `std::fs::metadata` (requirement
    /// 5.2).
    pub size_bytes: i64,
    /// Titre dérivé du nom de fichier par `Kyro_Parser` (requirement
    /// 4.1). Conservé à l'identique au premier insert ; **non** réécrit
    /// au refresh d'un upsert (requirement 3.3).
    pub title: String,
    /// Année dérivée du nom de fichier (requirement 4.2). Idem :
    /// premier insert seulement.
    pub year: Option<i64>,
    /// Qualité dérivée du nom de fichier (requirement 4.3).
    pub quality: Option<String>,
    /// Durée totale (s) renvoyée par ffprobe (requirement 5.1).
    pub duration_seconds: Option<i64>,
    /// Codec vidéo principal (requirement 5.1).
    pub video_codec: Option<String>,
    /// Largeur en pixels (requirement 5.1).
    pub width: Option<i64>,
    /// Hauteur en pixels (requirement 5.1).
    pub height: Option<i64>,
    /// Numérotation reconnue ? Toujours `false` pour un Film.
    pub numbering_unknown: bool,
}

/// Données d'entrée pour `upsert_episode_item` (tâche 7.2).
///
/// Forme « scanner → repo » pour un épisode. La série de rattachement
/// (`series_id`) doit déjà exister côté `media_item` (le scanner
/// matérialisera la série en amont via `upsert_media_item`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpsertEpisodeInput {
    /// Identifiant de la série/anime parent (FK `episode_item.series_id`).
    pub series_id: MediaItemId,
    /// Numéro de saison. Forcé à `1` par défaut pour les patterns
    /// souples (requirement 4.6).
    pub season_number: i64,
    /// Numéro d'épisode (requirements 4.4–4.6).
    pub episode_number: i64,
    /// Chemin absolu sur disque — clé d'unicité (UNIQUE en DDL).
    pub file_path: String,
    /// Taille en octets renvoyée par `std::fs::metadata`.
    pub size_bytes: i64,
    /// Durée totale (s) renvoyée par ffprobe.
    pub duration_seconds: Option<i64>,
    /// Codec vidéo principal.
    pub video_codec: Option<String>,
    /// Largeur en pixels.
    pub width: Option<i64>,
    /// Hauteur en pixels.
    pub height: Option<i64>,
    /// Numérotation inconnue (`true` si aucun motif d'épisode n'a été
    /// reconnu, requirement 4.7).
    pub numbering_unknown: bool,
}

// ---------------------------------------------------------------------------
// Repo : porteur du SqlitePool partagé.
// ---------------------------------------------------------------------------

/// Façade `Repository` (sqlx) — pour l'instant restreinte aux
/// opérations sur `library_folder` (tâche 7.1). Les méthodes des tâches
/// 7.2 → 7.18 viendront s'ajouter sur ce même type.
///
/// Le `SqlitePool` interne provient de `main.rs::init_sqlite_pool`
/// (pragmas WAL / synchronous=NORMAL / foreign_keys=ON appliqués). Il
/// est cloné par valeur car `sqlx::SqlitePool` est conceptuellement un
/// `Arc<Pool>` — le clone est bon marché et les états applicatifs
/// peuvent en détenir plusieurs copies.
#[derive(Debug, Clone)]
pub struct Repo {
    pool: SqlitePool,
}

impl Repo {
    /// Construit un `Repo` à partir d'un pool déjà ouvert.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Accès lecture seule au pool sous-jacent — utile aux tâches
    /// futures qui voudraient composer plusieurs repos sur le même
    /// pool.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    // -----------------------------------------------------------------------
    // Requirements 2.x — `library_folder`.
    // -----------------------------------------------------------------------

    /// Ajoute un dossier de bibliothèque (requirements 2.2, 2.4, 2.5).
    ///
    /// Étapes :
    /// 1. `std::fs::metadata(path)?.is_dir()` — refus immédiat avec
    ///    `KyroError::Validation` si le chemin n'existe pas, n'est pas
    ///    accessible, ou ne pointe pas sur un dossier (requirement 2.4).
    /// 2. Canonicalisation pour stocker un chemin absolu (`requirement
    ///    2.2 : chemin absolu`). Si la canonicalisation échoue (cas
    ///    rare : chemin trop long, droits refusés), on retombe sur le
    ///    chemin tel que fourni — l'INSERT pourra alors échouer sur
    ///    l'unicité, ce qui sera traduit en erreur DB classique.
    /// 3. INSERT paramétré + `last_insert_rowid()` pour récupérer
    ///    l'identifiant de la ligne insérée.
    ///
    /// Le mapping `Category` → `TEXT` reprend `Category::as_str` qui
    /// produit exactement les littéraux acceptés par le `CHECK
    /// (category IN ('Films','Series','Animes'))`.
    pub async fn add_library_folder(
        &self,
        path: &Path,
        category: Category,
    ) -> Result<FolderId, KyroError> {
        // 1. Validation FS — requirement 2.4.
        let metadata = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    path = %path.display(),
                    "Kyro: ajout dossier refusé — métadonnées FS inaccessibles"
                );
                return Err(KyroError::Validation(format!(
                    "le chemin {} n'est pas un dossier valide",
                    path.display()
                )));
            }
        };
        if !metadata.is_dir() {
            tracing::warn!(
                path = %path.display(),
                "Kyro: ajout dossier refusé — le chemin n'est pas un dossier"
            );
            return Err(KyroError::Validation(format!(
                "le chemin {} n'est pas un dossier valide",
                path.display()
            )));
        }

        // 2. Canonicalisation — requirement 2.2 (chemin absolu).
        let canonical = canonicalize_or_keep(path);
        let stored = canonical.to_string_lossy().into_owned();

        // 3. INSERT + récupération de l'id.
        let id: i64 = sqlx::query_scalar::<_, i64>(
            "INSERT INTO library_folder (path, category) VALUES (?, ?) RETURNING id",
        )
        .bind(&stored)
        .bind(category.as_str())
        .fetch_one(&self.pool)
        .await?;

        tracing::info!(
            folder_id = id,
            path = %stored,
            category = category.as_str(),
            "Kyro: dossier de bibliothèque ajouté"
        );

        Ok(FolderId(id))
    }

    /// Supprime un dossier de bibliothèque et marque ses items
    /// indisponibles (requirement 2.3).
    ///
    /// Effets, dans une **unique transaction** :
    ///
    /// 1. `UPDATE episode_item SET available = 0` pour tous les
    ///    épisodes dont la série référence le dossier (`series_id IN
    ///    SELECT media_item.id WHERE folder_id = ?`).
    /// 2. `UPDATE media_item SET available = 0 WHERE folder_id = ?`.
    /// 3. `DELETE FROM library_folder WHERE id = ?`.
    /// 4. `tx.commit()` — succès uniquement après commit (requirement
    ///    17.2).
    ///
    /// L'ordre 1 → 2 → 3 est volontaire : le `DELETE` sur
    /// `library_folder` déclenche `ON DELETE SET NULL` sur
    /// `media_item.folder_id` ; si on faisait le DELETE en premier, les
    /// items perdraient leur lien et l'UPDATE 2 serait incapable de les
    /// retrouver. De même, l'UPDATE 1 doit s'appuyer sur les
    /// `media_item` encore liés, donc avant l'UPDATE 2 (qui n'en
    /// modifie pas la liste, mais on garde la séquence stable).
    ///
    /// Si aucune ligne `library_folder` ne correspond à `id`, le DELETE
    /// final n'affecte aucune ligne — la fonction réussit silencieusement
    /// (idempotence côté UI : un retrait répété n'échoue pas).
    pub async fn remove_library_folder(&self, id: FolderId) -> Result<(), KyroError> {
        let id_value = id.get();
        let mut tx = self.pool.begin().await?;

        // 1. Marquer indisponibles les épisodes des séries du dossier.
        sqlx::query(
            "UPDATE episode_item \
               SET available = 0 \
             WHERE series_id IN ( \
                 SELECT id FROM media_item WHERE folder_id = ? \
             )",
        )
        .bind(id_value)
        .execute(&mut *tx)
        .await?;

        // 2. Marquer indisponibles les media_item rattachés.
        sqlx::query("UPDATE media_item SET available = 0 WHERE folder_id = ?")
            .bind(id_value)
            .execute(&mut *tx)
            .await?;

        // 3. Supprimer la ligne library_folder.
        sqlx::query("DELETE FROM library_folder WHERE id = ?")
            .bind(id_value)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        tracing::info!(
            folder_id = id_value,
            "Kyro: dossier de bibliothèque retiré (items marqués indisponibles)"
        );

        Ok(())
    }

    /// Liste tous les dossiers de bibliothèque (requirement 2.1).
    ///
    /// Ordonne par `id ASC` pour rendre la sortie déterministe — utile
    /// aux tests et à l'UI Settings qui affiche les dossiers dans leur
    /// ordre d'ajout.
    pub async fn list_library_folders(&self) -> Result<Vec<LibraryFolder>, KyroError> {
        let rows = sqlx::query("SELECT id, path, category FROM library_folder ORDER BY id ASC")
            .fetch_all(&self.pool)
            .await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(library_folder_from_row(&row)?);
        }
        Ok(out)
    }

    // -----------------------------------------------------------------------
    // Tâche 7.2 — Upsert `media_item` / `episode_item`.
    //
    // Sémantique commune (requirements 3.3, 3.4, 5.2) :
    //
    // 1. **Lookup** par `file_path` (clé d'unicité de fait : `media_item`
    //    n'a pas de UNIQUE explicite côté DDL mais le scanner garantit
    //    l'unicité ; `episode_item.file_path UNIQUE NOT NULL`).
    // 2. **Existant + même `size_bytes`** → idempotent : on ne touche
    //    rien et on renvoie l'`id` existant. C'est le cas nominal d'un
    //    re-scan d'un fichier inchangé (requirement 3.3).
    // 3. **Existant + `size_bytes` différent** → le fichier a été
    //    remuxé/réencodé/transcodé : on rafraîchit uniquement les
    //    **champs techniques** (`size_bytes`, `duration_seconds`,
    //    `video_codec`, `width`, `height`, `last_scanned_at`,
    //    `available=1`) sans toucher aux **champs édités manuellement**
    //    par l'utilisateur (titre, original_title, year, synopsis,
    //    studio, user_rating, poster_path, watch_progress_seconds,
    //    status, favorite, to_watch). Cf. requirement 3.3 « conserver
    //    l'entrée existante ».
    // 4. **Pas existant** → INSERT complet avec valeurs par défaut sur
    //    les champs utilisateur (`status='non_vu'`, `favorite=0`,
    //    `to_watch=0`, `watch_progress_seconds=0`, `available=1`).
    // -----------------------------------------------------------------------

    /// Upsert d'un `media_item` (Film, racine de Series ou racine
    /// d'Anime) selon la sémantique de la tâche 7.2.
    ///
    /// Retourne l'identifiant typé de la fiche après opération. Toutes
    /// les écritures se font dans une transaction unique pour ne
    /// signaler succès qu'après `COMMIT` (requirement 17.2).
    pub async fn upsert_media_item(
        &self,
        input: &UpsertMediaInput,
    ) -> Result<MediaItemId, KyroError> {
        let mut tx = self.pool.begin().await?;

        // 1. Recherche par chemin absolu — clé d'unicité applicative.
        let existing: Option<(i64, Option<i64>)> = sqlx::query_as::<_, (i64, Option<i64>)>(
            "SELECT id, size_bytes FROM media_item WHERE file_path = ?",
        )
        .bind(&input.file_path)
        .fetch_optional(&mut *tx)
        .await?;

        let id = match existing {
            // 2. Existant avec même taille → no-op idempotent.
            Some((id, Some(size))) if size == input.size_bytes => {
                tracing::debug!(
                    media_item_id = id,
                    file_path = %input.file_path,
                    "Kyro: upsert_media_item — skip (file_path + size_bytes inchangés)"
                );
                id
            }
            // 3. Existant mais taille différente (ou taille NULL) →
            //    refresh des champs techniques uniquement.
            //
            //    Liste explicite des champs rafraîchis (cf. tâche 7.2) :
            //    `size_bytes`, `duration_seconds`, `video_codec`,
            //    `width`, `height`, `last_scanned_at`, `available`.
            //
            //    Les autres colonnes — y compris celles dérivées du
            //    parser comme `quality`, `year`, `title`, ou le drapeau
            //    `numbering_unknown` — sont volontairement laissées
            //    intactes : elles ont pu être corrigées manuellement
            //    par l'utilisateur après le premier scan, et un
            //    re-scan ne doit pas écraser ces corrections
            //    (requirement 3.3 — « conserver l'entrée existante »).
            Some((id, _)) => {
                sqlx::query(
                    "UPDATE media_item \
                        SET size_bytes = ?, \
                            duration_seconds = ?, \
                            video_codec = ?, \
                            width = ?, \
                            height = ?, \
                            available = 1, \
                            last_scanned_at = CURRENT_TIMESTAMP \
                      WHERE id = ?",
                )
                .bind(input.size_bytes)
                .bind(input.duration_seconds)
                .bind(input.video_codec.as_deref())
                .bind(input.width)
                .bind(input.height)
                .bind(id)
                .execute(&mut *tx)
                .await?;

                tracing::info!(
                    media_item_id = id,
                    file_path = %input.file_path,
                    "Kyro: upsert_media_item — refresh des champs techniques"
                );
                id
            }
            // 4. Inexistant → INSERT complet avec valeurs par défaut
            //    utilisateur.
            None => {
                let inserted: i64 = sqlx::query_scalar::<_, i64>(
                    "INSERT INTO media_item ( \
                         folder_id, type, file_path, size_bytes, title, year, quality, \
                         duration_seconds, video_codec, width, height, \
                         watch_progress_seconds, status, favorite, to_watch, available, \
                         numbering_unknown, last_scanned_at \
                     ) VALUES ( \
                         ?, ?, ?, ?, ?, ?, ?, \
                         ?, ?, ?, ?, \
                         0, 'non_vu', 0, 0, 1, \
                         ?, CURRENT_TIMESTAMP \
                     ) RETURNING id",
                )
                .bind(input.folder_id.map(|f| f.get()))
                .bind(input.media_type.as_str())
                .bind(&input.file_path)
                .bind(input.size_bytes)
                .bind(&input.title)
                .bind(input.year)
                .bind(input.quality.as_deref())
                .bind(input.duration_seconds)
                .bind(input.video_codec.as_deref())
                .bind(input.width)
                .bind(input.height)
                .bind(bool_to_i64(input.numbering_unknown))
                .fetch_one(&mut *tx)
                .await?;

                tracing::info!(
                    media_item_id = inserted,
                    file_path = %input.file_path,
                    media_type = input.media_type.as_str(),
                    "Kyro: upsert_media_item — nouvelle fiche insérée"
                );
                inserted
            }
        };

        tx.commit().await?;
        Ok(MediaItemId(id))
    }

    /// Upsert d'un `episode_item` selon la sémantique de la tâche 7.2.
    ///
    /// Variante stricte de `upsert_media_item` adaptée au DDL
    /// `episode_item` : pas de `type`, mais `series_id`, `season_number`,
    /// `episode_number`. La clé d'unicité de la table est explicitement
    /// `file_path UNIQUE NOT NULL`.
    pub async fn upsert_episode_item(
        &self,
        input: &UpsertEpisodeInput,
    ) -> Result<EpisodeItemId, KyroError> {
        let mut tx = self.pool.begin().await?;

        let existing: Option<(i64, Option<i64>)> = sqlx::query_as::<_, (i64, Option<i64>)>(
            "SELECT id, size_bytes FROM episode_item WHERE file_path = ?",
        )
        .bind(&input.file_path)
        .fetch_optional(&mut *tx)
        .await?;

        let id = match existing {
            Some((id, Some(size))) if size == input.size_bytes => {
                tracing::debug!(
                    episode_item_id = id,
                    file_path = %input.file_path,
                    "Kyro: upsert_episode_item — skip (file_path + size_bytes inchangés)"
                );
                id
            }
            Some((id, _)) => {
                // Refresh strict des champs techniques :
                // `size_bytes`, `duration_seconds`, `video_codec`,
                // `width`, `height`, `last_scanned_at`, `available`.
                //
                // Les colonnes dérivées du parser (`season_number`,
                // `episode_number`, `numbering_unknown`) sont laissées
                // intactes au refresh : elles ont pu être corrigées
                // manuellement par l'utilisateur après le premier scan
                // (requirement 3.3 — « conserver l'entrée existante »).
                sqlx::query(
                    "UPDATE episode_item \
                        SET size_bytes = ?, \
                            duration_seconds = ?, \
                            video_codec = ?, \
                            width = ?, \
                            height = ?, \
                            available = 1, \
                            last_scanned_at = CURRENT_TIMESTAMP \
                      WHERE id = ?",
                )
                .bind(input.size_bytes)
                .bind(input.duration_seconds)
                .bind(input.video_codec.as_deref())
                .bind(input.width)
                .bind(input.height)
                .bind(id)
                .execute(&mut *tx)
                .await?;

                tracing::info!(
                    episode_item_id = id,
                    file_path = %input.file_path,
                    "Kyro: upsert_episode_item — refresh des champs techniques"
                );
                id
            }
            None => {
                let inserted: i64 = sqlx::query_scalar::<_, i64>(
                    "INSERT INTO episode_item ( \
                         series_id, season_number, episode_number, file_path, size_bytes, \
                         duration_seconds, video_codec, width, height, \
                         watch_progress_seconds, status, available, numbering_unknown, \
                         last_scanned_at \
                     ) VALUES ( \
                         ?, ?, ?, ?, ?, \
                         ?, ?, ?, ?, \
                         0, 'non_vu', 1, ?, \
                         CURRENT_TIMESTAMP \
                     ) RETURNING id",
                )
                .bind(input.series_id.get())
                .bind(input.season_number)
                .bind(input.episode_number)
                .bind(&input.file_path)
                .bind(input.size_bytes)
                .bind(input.duration_seconds)
                .bind(input.video_codec.as_deref())
                .bind(input.width)
                .bind(input.height)
                .bind(bool_to_i64(input.numbering_unknown))
                .fetch_one(&mut *tx)
                .await?;

                tracing::info!(
                    episode_item_id = inserted,
                    file_path = %input.file_path,
                    series_id = input.series_id.get(),
                    "Kyro: upsert_episode_item — nouvel épisode inséré"
                );
                inserted
            }
        };

        tx.commit().await?;
        Ok(EpisodeItemId(id))
    }

    // -----------------------------------------------------------------------
    // Tâche 7.3 — set_watch_progress + force_status (requirements 8.1–8.4, 17.2).
    // -----------------------------------------------------------------------

    /// Met à jour la progression de lecture d'un Media_Item ou d'un
    /// Episode_Item et recalcule son statut via
    /// [`crate::domain::status::compute_status`].
    ///
    /// Étapes (ouverte dans une **unique transaction** pour ne signaler
    /// succès qu'après `COMMIT` — requirement 17.2) :
    /// 1. Lire `duration_seconds` de la ligne ciblée.
    /// 2. Calculer `compute_status(position, duration, None)` — le
    ///    `manual_override = None` garantit qu'une valeur forcée
    ///    précédemment via [`Repo::force_status`] **ne survit pas** à
    ///    une nouvelle progression (cf. doc `force_status` ci-dessous).
    /// 3. UPDATE `watch_progress_seconds` + `status`.
    /// 4. COMMIT.
    ///
    /// Si la cible n'existe pas, retourne `KyroError::Validation` —
    /// l'appelant doit valider l'identifiant en amont (cas typique :
    /// IPC pipe d'mpv reçu après suppression manuelle de la fiche).
    pub async fn set_watch_progress(
        &self,
        target: ProgressTarget,
        position: u32,
    ) -> Result<crate::domain::status::Status, KyroError> {
        let mut tx = self.pool.begin().await?;

        // 1. Lecture de la durée existante.
        let duration: Option<i64> = match target {
            ProgressTarget::Media(id) => {
                sqlx::query_scalar::<_, Option<i64>>(
                    "SELECT duration_seconds FROM media_item WHERE id = ?",
                )
                .bind(id.get())
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(|| {
                    KyroError::Validation(format!(
                        "media_item id={} introuvable pour set_watch_progress",
                        id.get()
                    ))
                })?
            }
            ProgressTarget::Episode(id) => {
                sqlx::query_scalar::<_, Option<i64>>(
                    "SELECT duration_seconds FROM episode_item WHERE id = ?",
                )
                .bind(id.get())
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(|| {
                    KyroError::Validation(format!(
                        "episode_item id={} introuvable pour set_watch_progress",
                        id.get()
                    ))
                })?
            }
        };

        // 2. Calcul du statut. `compute_status` accepte `u32` ; durée
        //    inconnue (NULL en DB) ou 0 retombe sur NonVu.
        let duration_u32 = duration.and_then(|d| u32::try_from(d).ok()).unwrap_or(0);
        let status = crate::domain::status::compute_status(position, duration_u32, None);

        // 3. UPDATE selon la cible.
        match target {
            ProgressTarget::Media(id) => {
                sqlx::query(
                    "UPDATE media_item \
                        SET watch_progress_seconds = ?, status = ? \
                      WHERE id = ?",
                )
                .bind(i64::from(position))
                .bind(status.as_str())
                .bind(id.get())
                .execute(&mut *tx)
                .await?;
            }
            ProgressTarget::Episode(id) => {
                sqlx::query(
                    "UPDATE episode_item \
                        SET watch_progress_seconds = ?, status = ? \
                      WHERE id = ?",
                )
                .bind(i64::from(position))
                .bind(status.as_str())
                .bind(id.get())
                .execute(&mut *tx)
                .await?;
            }
        }

        tx.commit().await?;

        tracing::debug!(
            target = ?target,
            position,
            duration = duration_u32,
            status = status.as_str(),
            "Kyro: set_watch_progress committé"
        );

        Ok(status)
    }

    /// Force manuellement le statut d'un Media_Item ou Episode_Item
    /// (requirement 8.4).
    ///
    /// Note de design : l'override n'est **pas** persisté comme un
    /// drapeau séparé. La valeur forcée est simplement écrite dans la
    /// colonne `status` ; toute mise à jour ultérieure de la
    /// progression via [`Repo::set_watch_progress`] passera par
    /// `compute_status(position, duration, None)` et écrasera ce
    /// statut manuel selon le ratio. Ce comportement est cohérent
    /// avec le paramètre `manual_override: Option<Status>` de
    /// `compute_status` qui n'est consulté qu'au moment du calcul,
    /// pas mémorisé. Si un futur requirement nécessite une override
    /// persistante, il faudra ajouter une colonne booléenne dédiée.
    pub async fn force_status(
        &self,
        target: ProgressTarget,
        status: crate::domain::status::Status,
    ) -> Result<(), KyroError> {
        let mut tx = self.pool.begin().await?;

        let affected = match target {
            ProgressTarget::Media(id) => {
                sqlx::query("UPDATE media_item SET status = ? WHERE id = ?")
                    .bind(status.as_str())
                    .bind(id.get())
                    .execute(&mut *tx)
                    .await?
                    .rows_affected()
            }
            ProgressTarget::Episode(id) => {
                sqlx::query("UPDATE episode_item SET status = ? WHERE id = ?")
                    .bind(status.as_str())
                    .bind(id.get())
                    .execute(&mut *tx)
                    .await?
                    .rows_affected()
            }
        };

        if affected == 0 {
            return Err(KyroError::Validation(format!(
                "cible {target:?} introuvable pour force_status"
            )));
        }

        tx.commit().await?;

        tracing::info!(
            target = ?target,
            status = status.as_str(),
            "Kyro: force_status appliqué"
        );

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Tâche 7.4 — Listings et requêtes secondaires.
    //
    // Ces méthodes alimentent les pages frontend `/films`, `/series`,
    // `/animes`, `/favoris`, `/a-regarder`, `/detail/:id` ainsi que les
    // contrôles de recherche / suivi des séries (requirements 7.2, 7.3,
    // 7.4, 8.5, 11.4, 11.5, 12.1).
    //
    // Conventions communes :
    //
    // * Les listings de `media_item` projettent vers la vue minimale
    //   [`crate::domain::types::MediaItem`] consommée par
    //   `domain::filters` et `domain::search`. Les genres sont chargés
    //   en *batch* via une seconde requête sur `media_genre`/`genre`
    //   afin d'éviter le « N+1 » SQL et de garder un seul aller-retour
    //   par listing.
    // * Les listings de `episode_item` retournent une vue minimale
    //   `EpisodeRow` portant strictement les champs requis par
    //   `domain::resume::next_resume` (tâche 4.2) plus l'identifiant
    //   typé. Le tri `(season_number ASC, episode_number ASC)` est
    //   demandé directement à SQLite via l'index
    //   `idx_episode_series` (cf. migration 0001) — pas de tri en RAM.
    // * `search` n'utilise **pas** un `LIKE` SQL : la spécification
    //   exige une recherche insensible à la casse **et aux accents**
    //   (requirement 13.2), ce que SQLite ne sait pas faire seul sans
    //   collation custom. On charge donc l'ensemble des fiches puis on
    //   filtre via `domain::search::matches` (déjà couvert par les
    //   propriétés P-28 / P-29). Le coût reste très contenu : la cible
    //   13.3 borne la bibliothèque à 5 000 items, et le pipeline
    //   normalisation + sous-chaîne est O(|titre| + |query|).
    // -----------------------------------------------------------------------

    /// Liste les Media_Item d'une catégorie donnée (requirement 7.2).
    ///
    /// Mappe `Category` → `MediaType` selon la correspondance naturelle
    /// `Films → Film`, `Series → Series`, `Animes → Anime`. Cf.
    /// `media_item.type` qui utilise les variantes singulier (cf.
    /// migration 0001 — CHECK `type IN ('Film','Series','Anime')`).
    /// Tri stable par `id ASC` pour rendre la sortie déterministe et
    /// faciliter les tests.
    ///
    /// **Filtrage par disponibilité** : on n'affiche que les fiches
    /// dont `available = 1`. Pour un Film, c'est le drapeau direct
    /// du `media_item`. Pour une Série / un Anime (qui n'a pas de
    /// `file_path` propre), on considère la fiche disponible si
    /// **au moins un épisode** rattaché reste disponible
    /// (`episode_item.available = 1`). Les séries dont tous les
    /// épisodes ont disparu (utilisateur a déplacé / supprimé les
    /// fichiers) sont automatiquement masquées.
    pub async fn list_by_category(
        &self,
        category: Category,
    ) -> Result<Vec<MediaItem>, KyroError> {
        let media_type = category_to_media_type(category);
        let rows = sqlx::query(
            "SELECT id, title, original_title, year, type, status, poster_path \
               FROM media_item \
              WHERE type = ? \
                AND available = 1 \
                AND ( \
                  type = 'Film' \
                  OR (file_path IS NOT NULL AND file_path != '') \
                  OR EXISTS ( \
                    SELECT 1 FROM episode_item \
                     WHERE episode_item.series_id = media_item.id \
                       AND episode_item.available = 1 \
                  ) \
                ) \
              ORDER BY id ASC",
        )
        .bind(media_type.as_str())
        .fetch_all(&self.pool)
        .await?;

        self.materialize_media_items(rows).await
    }

    /// Recherche par titre / titre original, insensible à la casse et
    /// aux accents (requirement 13.2).
    ///
    /// Implémentation v1.0 : utilise l'index FTS5 `media_item_fts`
    /// (migration 0002) avec le tokenizer `unicode61 remove_diacritics 2`
    /// pour une recherche O(log N) sur la bibliothèque. Les tokens de
    /// la query sont passés en mode prefix (`tok*`) pour préserver la
    /// sémantique « contient » historique (Inception → match « Incep »).
    ///
    /// La requête vide retourne **toutes** les fiches (Requirement 13.4),
    /// short-circuit avant la requête FTS pour rester O(N) sur le seul
    /// SELECT principal.
    pub async fn search(&self, query: &str) -> Result<Vec<MediaItem>, KyroError> {
        let trimmed = query.trim();

        // Requête vide : on retourne tout (Requirement 13.4).
        if trimmed.is_empty() {
            let rows = sqlx::query(
                "SELECT id, title, original_title, year, type, status, poster_path \
                   FROM media_item \
                  ORDER BY id ASC",
            )
            .fetch_all(&self.pool)
            .await?;
            return self.materialize_media_items(rows).await;
        }

        // Construction du `MATCH 'tok1* tok2* tok3*'` : chaque token de
        // la query devient un préfixe FTS5. On échappe les guillemets
        // doubles dans chaque token (FTS5 utilise `""` pour échapper),
        // on filtre les caractères réservés FTS (`* ( ) " :`) qui
        // pourraient déclencher des erreurs de parsing, et on enrobe
        // chaque token dans des guillemets doubles pour être à l'abri
        // de tokens commençant par `AND`/`OR`/`NOT` (mots-clés FTS).
        let fts_query = build_fts_match_query(trimmed);
        if fts_query.is_empty() {
            // Query non-vide mais ne contenant que des caractères
            // réservés / blancs après nettoyage → aucun résultat.
            return Ok(Vec::new());
        }

        let rows = sqlx::query(
            "SELECT m.id, m.title, m.original_title, m.year, m.type, m.status, \
                    m.poster_path \
               FROM media_item m \
               JOIN media_item_fts f ON f.rowid = m.id \
              WHERE media_item_fts MATCH ? \
              ORDER BY m.id ASC",
        )
        .bind(&fts_query)
        .fetch_all(&self.pool)
        .await?;

        self.materialize_media_items(rows).await
    }

    /// Liste les Media_Item marqués `favorite = 1` (requirement 11.4).
    pub async fn list_favorites(&self) -> Result<Vec<MediaItem>, KyroError> {
        let rows = sqlx::query(
            "SELECT id, title, original_title, year, type, status, poster_path \
               FROM media_item \
              WHERE favorite = 1 \
              ORDER BY id ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        self.materialize_media_items(rows).await
    }

    /// Liste les Media_Item marqués `to_watch = 1` (requirement 11.5).
    pub async fn list_to_watch(&self) -> Result<Vec<MediaItem>, KyroError> {
        let rows = sqlx::query(
            "SELECT id, title, original_title, year, type, status, poster_path \
               FROM media_item \
              WHERE to_watch = 1 \
              ORDER BY id ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        self.materialize_media_items(rows).await
    }

    /// Liste les Episode_Item d'une série, triés par
    /// `(season_number ASC, episode_number ASC)` (requirements 7.3,
    /// 7.4).
    ///
    /// Le `ORDER BY` exploite directement l'index composite
    /// `idx_episode_series (series_id, season_number, episode_number)`
    /// défini dans `0001_init.sql`. La projection inclut `available`
    /// pour permettre à `next_resume_episode` (ci-dessous) d'invoquer
    /// `domain::resume::next_resume` qui filtre sur ce drapeau.
    pub async fn list_episodes(
        &self,
        series_id: MediaItemId,
    ) -> Result<Vec<EpisodeRow>, KyroError> {
        let rows = sqlx::query(
            "SELECT id, season_number, episode_number, status, available, \
                    watch_progress_seconds, duration_seconds \
               FROM episode_item \
              WHERE series_id = ? \
              ORDER BY season_number ASC, episode_number ASC",
        )
        .bind(series_id.get())
        .fetch_all(&self.pool)
        .await?;

        rows.iter().map(episode_row_from_row).collect()
    }

    /// Compte les épisodes d'une série regroupés par statut
    /// (requirement 8.5 — affichage « N vus / total »).
    ///
    /// Les trois clés `non_vu` / `en_cours` / `vu` sont **toujours**
    /// présentes dans la map résultat, même si leur compte vaut 0,
    /// afin que le frontend puisse afficher uniformément le ratio sans
    /// se soucier de clés manquantes. Le total `card(episodes)` peut
    /// être recomposé via `counts.values().sum::<i64>()`.
    pub async fn count_episodes_by_status(
        &self,
        series_id: MediaItemId,
    ) -> Result<HashMap<Status, i64>, KyroError> {
        let rows = sqlx::query(
            "SELECT status, COUNT(*) as cnt \
               FROM episode_item \
              WHERE series_id = ? \
              GROUP BY status",
        )
        .bind(series_id.get())
        .fetch_all(&self.pool)
        .await?;

        let mut out: HashMap<Status, i64> = HashMap::with_capacity(3);
        // Initialiser les trois variantes à 0 pour garantir la présence
        // systématique des clés (cf. doc ci-dessus).
        for s in [Status::NonVu, Status::EnCours, Status::Vu] {
            out.insert(s, 0);
        }
        for row in rows {
            let status_str: String = row.try_get("status")?;
            let cnt: i64 = row.try_get("cnt")?;
            let status: Status = status_str
                .parse()
                .map_err(|err: ParseStatusError| {
                    sqlx::Error::Decode(
                        format!(
                            "statut inconnu dans episode_item.status: {}",
                            err.value
                        )
                        .into(),
                    )
                })?;
            out.insert(status, cnt);
        }
        Ok(out)
    }

    /// Calcule l'action « Reprendre » / « Revoir depuis le début » pour
    /// une série (requirement 12.1).
    ///
    /// Charge les épisodes disponibles **et** indisponibles via
    /// [`Self::list_episodes`] puis les projette vers la vue
    /// `domain::resume::EpisodeItem` consommée par
    /// `domain::resume::next_resume`. Cette dernière filtre elle-même
    /// sur `available` (cf. tâche 4.2) — on délègue intégralement le
    /// calcul au domaine pur sans dupliquer la logique.
    ///
    /// La méthode retourne `Option<EpisodeItemId>` plutôt que
    /// `ResumeAction` afin d'exposer **uniquement l'identifiant ciblé**
    /// par le bouton (que ce soit pour `Reprendre` ou
    /// `Revoir depuis le début`) — c'est ce que les commandes Tauri et
    /// la commande `play_media` (tâche 12.4) consomment réellement.
    /// Si aucune reprise n'est proposable (`ResumeAction::None`), on
    /// renvoie `None`.
    pub async fn next_resume_episode(
        &self,
        series_id: MediaItemId,
    ) -> Result<Option<EpisodeItemId>, KyroError> {
        // 1. Charger les épisodes triés par (season, episode).
        let rows = self.list_episodes(series_id).await?;

        // 2. Projeter vers la vue domaine consommée par next_resume.
        let episodes: Vec<resume::EpisodeItem> =
            rows.iter().map(episode_row_to_domain).collect();

        // 3. Déléguer le calcul au domaine pur (req 12.1, 12.3).
        let action = resume::next_resume(&episodes);
        Ok(match action {
            ResumeAction::Resume(e) => Some(EpisodeItemId(e.id)),
            ResumeAction::RewatchFromStart(Some(e)) => Some(EpisodeItemId(e.id)),
            ResumeAction::RewatchFromStart(None) | ResumeAction::None => None,
        })
    }

    // -----------------------------------------------------------------------
    // Helpers internes — projection `media_item` row → `MediaItem` du
    // domaine, batch des genres, etc.
    // -----------------------------------------------------------------------

    /// Matérialise un lot de lignes `media_item` (projection
    /// `id, title, original_title, year, type, status`) en
    /// `Vec<MediaItem>` du domaine, en chargeant les genres associés en
    /// **un seul** aller-retour SQL pour éviter le N+1.
    async fn materialize_media_items(
        &self,
        rows: Vec<SqliteRow>,
    ) -> Result<Vec<MediaItem>, KyroError> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }

        // 1. Décoder l'en-tête (id, title, original_title, year, type,
        //    status) sans toucher aux genres.
        let mut bare: Vec<MediaItemBare> = Vec::with_capacity(rows.len());
        let mut ids: Vec<i64> = Vec::with_capacity(rows.len());
        for row in &rows {
            let item = media_item_bare_from_row(row)?;
            ids.push(item.id);
            bare.push(item);
        }

        // 2. Charger en bloc les genres pour ces ids. On construit
        //    dynamiquement la liste de placeholders `?, ?, ?, ...` car
        //    `sqlx` ne propose pas d'helper portable pour `IN (...)` en
        //    SQLite.
        let placeholders = vec!["?"; ids.len()].join(",");
        let sql = format!(
            "SELECT mg.media_item_id, g.name \
               FROM media_genre mg \
               JOIN genre g ON g.id = mg.genre_id \
              WHERE mg.media_item_id IN ({placeholders}) \
              ORDER BY mg.media_item_id ASC, g.name ASC"
        );
        let mut q = sqlx::query(&sql);
        for id in &ids {
            q = q.bind(*id);
        }
        let genre_rows = q.fetch_all(&self.pool).await?;

        let mut genres_by_id: HashMap<i64, Vec<String>> = HashMap::new();
        for row in genre_rows {
            let mid: i64 = row.try_get("media_item_id")?;
            let name: String = row.try_get("name")?;
            genres_by_id.entry(mid).or_default().push(name);
        }

        // 3. Reconstruire les MediaItem du domaine en attachant les
        //    genres. L'ordre des fiches reste celui des rows d'entrée.
        let out = bare
            .into_iter()
            .map(|b| {
                let genres = genres_by_id.remove(&b.id).unwrap_or_default();
                MediaItem {
                    id: MediaItemId(b.id),
                    title: b.title,
                    original_title: b.original_title,
                    year: b.year,
                    media_type: b.media_type,
                    status: b.status,
                    genres,
                    poster_path: b.poster_path,
                }
            })
            .collect();
        Ok(out)
    }

    // -----------------------------------------------------------------------
    // Tâche 7.5 — settings key/value (requirements 2.1, 7.1, 10.1, 10.2,
    // 16.1, 16.14).
    //
    // La table `setting_value` est un dictionnaire (clé `TEXT PRIMARY
    // KEY`, valeur `TEXT NOT NULL`) commun à plusieurs sous-systèmes
    // (résolveur ffprobe, lecteur externe, thème, scan au démarrage,
    // dernière catégorie). On expose deux primitives volontairement
    // étroites :
    //
    // * `upsert_setting(key, value)` — INSERT OR REPLACE après
    //   validation de `key` contre l'allowlist
    //   [`settings_keys::RESERVED_KEYS`]. La valeur est conservée à
    //   l'identique (pas de `trim`, pas de normalisation) — chaque
    //   sous-système est responsable du format qu'il attend (booléen
    //   stringifié, chemin absolu, littéral de catégorie, etc.).
    // * `get_setting(key)` — SELECT, retourne `Option<String>`. La
    //   clé n'est **pas** validée contre l'allowlist : lire une clé
    //   inconnue retourne simplement `None` (cas équivalent à « pas
    //   de valeur stockée »), ce qui évite de casser une migration
    //   pendant laquelle une nouvelle clé serait introduite avant
    //   d'être ajoutée à `RESERVED_KEYS`.
    //
    // L'opération `upsert_setting` est encapsulée dans une transaction
    // d'une seule instruction afin de respecter le requirement 17.2
    // (« persistance avant signal de succès ») de manière homogène avec
    // les autres mutations du repo.
    // -----------------------------------------------------------------------

    /// Insère ou remplace la valeur d'un réglage applicatif.
    ///
    /// `key` doit appartenir à
    /// [`crate::infra::settings_keys::RESERVED_KEYS`] ; sinon retourne
    /// `KyroError::Validation` en citant la clé refusée.
    ///
    /// La valeur est stockée telle quelle dans `setting_value.value`
    /// (NOT NULL côté DDL). Si l'appelant veut signifier l'absence
    /// d'une préférence, il doit ne **pas** appeler cette méthode (la
    /// table SQLite distingue déjà absence et chaîne vide via
    /// `Option<String>` en lecture).
    ///
    /// L'INSERT utilise `INSERT INTO ... ON CONFLICT(key) DO UPDATE`
    /// (équivalent UPSERT SQLite) pour rester atomique sur la clé
    /// primaire `key`.
    pub async fn upsert_setting(&self, key: &str, value: &str) -> Result<(), KyroError> {
        if !crate::infra::settings_keys::is_reserved(key) {
            tracing::warn!(
                key = %key,
                "Kyro: upsert_setting refusé — clé hors allowlist"
            );
            return Err(KyroError::Validation(format!(
                "clé de réglage non reconnue: {key:?}"
            )));
        }

        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO setting_value (key, value) VALUES (?, ?) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        // Note de confidentialité : on logue la clé (énumération fixe,
        // pas de PII) mais **pas** la valeur — `external_player_path`
        // ou `ffprobe_path` peuvent contenir un nom d'utilisateur
        // Windows. Le scénario debug peut être levé via `tracing` à
        // niveau `trace` si nécessaire.
        tracing::debug!(key = %key, "Kyro: setting_value upserté");
        Ok(())
    }

    /// Lit la valeur d'un réglage applicatif. Retourne `None` si la
    /// clé n'est pas présente en base. Aucune validation n'est faite
    /// sur `key` (cf. doc du module `settings_keys`).
    pub async fn get_setting(&self, key: &str) -> Result<Option<String>, KyroError> {
        let value: Option<String> =
            sqlx::query_scalar::<_, String>("SELECT value FROM setting_value WHERE key = ?")
                .bind(key)
                .fetch_optional(&self.pool)
                .await?;
        Ok(value)
    }

    // -----------------------------------------------------------------------
    // Tâche 7.6 — Toggles favori / à_regarder (requirements 11.1, 11.2,
    // 11.3).
    //
    // Sémantique « involution garantie par construction » : la mise à
    // jour SQL applique `colonne = 1 - colonne` sur des colonnes
    // contraintes par DDL à `IN (0, 1)`. La fonction `x ↦ 1 - x` est
    // une involution au sens strict sur `{0, 1}` puisque
    // `1 - (1 - x) == x` pour tout `x ∈ {0, 1}`. Ainsi, deux appels
    // successifs ramènent **systématiquement** la fiche à son état
    // initial — sans lecture préalable, sans condition de course
    // possible entre un `SELECT` et un `UPDATE` (l'opération est
    // atomique en une seule requête, encapsulée dans une transaction
    // pour respecter le requirement 17.2 « persistance avant signal de
    // succès »).
    //
    // Pourquoi pas un `NOT colonne` ? SQLite stocke les booléens en
    // `INTEGER` ; `NOT 0` vaut `1` mais `NOT 5` vaut `0` aussi, ce qui
    // briserait l'involution si une valeur hors `{0, 1}` se glissait
    // (CHECK désactivé, migration corrompue). `1 - x` donne `1` pour
    // `x=0` et `0` pour `x=1`, et un éventuel `2` deviendrait `-1` —
    // on préfère donc **renforcer** la garantie via la clause `RETURNING`
    // qui re-lit la valeur écrite, et via le CHECK SQL qui rejette les
    // écritures hors `{0, 1}` (la transaction serait alors avortée
    // sans modifier la fiche, ce qui correspond bien au comportement
    // attendu d'une base corrompue).
    //
    // La méthode retourne le **nouvel état** du drapeau (`bool`) après
    // bascule, ce qui permet au frontend de mettre à jour la grille /
    // la fiche sans aller-retour supplémentaire (cf. requirements 11.2,
    // 11.3 — toggles depuis grille **et** vue détaillée).
    // -----------------------------------------------------------------------

    /// Bascule le drapeau `favorite` d'un Media_Item (requirements
    /// 11.1, 11.2). Retourne le nouvel état (`true` si le média est
    /// désormais marqué comme favori).
    ///
    /// Si l'identifiant n'existe pas, retourne `KyroError::Validation`
    /// — l'appelant est responsable de la cohérence avec l'UI.
    pub async fn toggle_favorite(&self, id: MediaItemId) -> Result<bool, KyroError> {
        toggle_media_flag(&self.pool, id, "favorite").await
    }

    /// Bascule le drapeau `to_watch` d'un Media_Item (requirements
    /// 11.1, 11.3). Retourne le nouvel état (`true` si le média est
    /// désormais dans la liste à regarder).
    ///
    /// Si l'identifiant n'existe pas, retourne `KyroError::Validation`.
    pub async fn toggle_to_watch(&self, id: MediaItemId) -> Result<bool, KyroError> {
        toggle_media_flag(&self.pool, id, "to_watch").await
    }

    // -----------------------------------------------------------------------
    // Tâche 7.7 — Import d'affiche locale (requirements 6.2, 6.3, 17.1).
    //
    // L'implémentation effective vit dans `infra::posters` ; cette
    // méthode n'est qu'une façade fine qui injecte le pool partagé et
    // expose la signature usuelle « target + source + posters_dir »
    // attendue par les commandes Tauri (tâche 12.x). Le `posters_dir`
    // reste un paramètre injectable pour permettre aux tests d'éviter
    // toute écriture dans `%APPDATA%\Kyro\posters\` ; en production le
    // chemin sera résolu via `tauri::Manager::path()` côté commands.
    // -----------------------------------------------------------------------

    /// Importe une affiche locale pour un Media_Item ou un Episode_Item.
    ///
    /// Cf. [`crate::infra::posters::import_poster`] pour la sémantique
    /// complète (validation extension ∈ {jpg, jpeg, png, webp},
    /// vérification existence du fichier source et de la cible, copie
    /// vers `posters_dir/<prefix>-<id>.<ext>`, UPDATE atomique de
    /// `poster_path`).
    pub async fn import_poster(
        &self,
        target: crate::infra::posters::PosterTarget,
        source_path: &Path,
        posters_dir: &Path,
    ) -> Result<PathBuf, KyroError> {
        crate::infra::posters::import_poster(&self.pool, target, source_path, posters_dir)
            .await
    }

    // -----------------------------------------------------------------------
    // Tâche 7.12 — Mise à jour de la note utilisateur (requirement 6.6).
    //
    // L'édition d'une fiche (tâche 12.x côté commands) passera à terme
    // par un `update_media_item` plus complet ; pour la propriété 12, on
    // expose ici l'opération minimale qui matérialise l'invariant
    // `user_rating ∈ [0, 10]` côté repo : un point d'entrée dédié,
    // strictement validé en amont du SQL, qui retourne
    // `KyroError::Validation` (avec un message indiquant la plage
    // autorisée) **avant** toute écriture si la valeur est hors plage.
    //
    // Le DDL porte déjà la contrainte `CHECK (user_rating BETWEEN 0
    // AND 10)` — la validation applicative en amont ajoute deux
    // garanties que la contrainte SQL ne donne pas seule :
    //
    //   1. Un message d'erreur explicite en français mentionnant la
    //      plage autorisée (requirement 6.6 — « afficher un message
    //      indiquant la plage autorisée »), au lieu du message brut de
    //      sqlx (« CHECK constraint failed »).
    //   2. Un refus immédiat **sans ouvrir de transaction**, ce qui
    //      garantit qu'aucune ligne n'est touchée en base — utile pour
    //      la propriété 12 qui exige que la fiche persistée reste
    //      strictement inchangée après un rejet.
    //
    // `rating = None` est explicitement accepté : le requirement 6.1
    // permet à l'utilisateur de laisser la note vide (colonne
    // nullable). Seules les valeurs `Some(v)` avec `v < 0` ou `v > 10`
    // sont rejetées.
    // -----------------------------------------------------------------------

    /// Met à jour la note utilisateur d'un Media_Item après validation
    /// stricte de la plage `[0, 10]` (requirement 6.6).
    ///
    /// Retourne `KyroError::Validation` si :
    ///
    /// * `rating = Some(v)` et `v` est hors de l'intervalle `[0, 10]`,
    ///   sans toucher à la base ;
    /// * la fiche `id` n'existe pas (cohérence avec
    ///   `set_watch_progress` / `force_status`).
    ///
    /// La mutation est encapsulée dans une transaction : succès
    /// uniquement après `COMMIT` (requirement 17.2).
    pub async fn update_user_rating(
        &self,
        id: MediaItemId,
        rating: Option<i64>,
    ) -> Result<(), KyroError> {
        // 1. Validation applicative en amont — aucun SQL exécuté si la
        //    valeur est hors plage. Le message inclut la plage exacte
        //    pour que le frontend puisse l'afficher tel quel
        //    (requirement 6.6).
        if let Some(value) = rating {
            if !(0..=10).contains(&value) {
                return Err(KyroError::Validation(format!(
                    "la note utilisateur {} est hors de la plage autorisée [0, 10]",
                    value
                )));
            }
        }

        // 2. UPDATE paramétré dans une transaction. `rows_affected()`
        //    permet de détecter l'absence de la ligne (id inconnu) et
        //    de remonter une `KyroError::Validation` cohérente avec
        //    les autres mutations ciblées par id.
        let mut tx = self.pool.begin().await?;
        let affected = sqlx::query("UPDATE media_item SET user_rating = ? WHERE id = ?")
            .bind(rating)
            .bind(id.get())
            .execute(&mut *tx)
            .await?
            .rows_affected();

        if affected == 0 {
            // On annule explicitement la transaction : aucune autre
            // mutation n'a été émise mais on reste cohérent avec le
            // pattern « commit uniquement en succès ».
            tx.rollback().await?;
            return Err(KyroError::Validation(format!(
                "media_item id={} introuvable pour update_user_rating",
                id.get()
            )));
        }

        tx.commit().await?;
        tracing::debug!(
            media_item_id = id.get(),
            rating = ?rating,
            "Kyro: update_user_rating committé"
        );
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Tâche 8.2 — helpers minimaux consommés par `infra::scanner`.
    //
    // Ces méthodes existent **uniquement** pour permettre au scanner
    // d'effectuer le diff « existants vs disparus » et de marquer les
    // entrées orphelines `available = 0`. Elles sont volontairement
    // étroites : pas de pagination, pas de filtres avancés (ces besoins
    // seront couverts par les commandes Tauri lecture des tâches 12.x).
    //
    // Les requêtes ciblent l'ensemble des items rattachés à un dossier
    // donné, en suivant les FKs :
    //
    // * `media_item.folder_id = ?` — direct.
    // * `episode_item.series_id IN (SELECT id FROM media_item WHERE
    //    folder_id = ?)` — un épisode appartient à une série, qui
    //    appartient à un dossier.
    //
    // Pourquoi `pub(crate)` ? Le scanner et le repo vivent dans le
    // même crate ; aucun consommateur externe (frontend, commands) ne
    // doit voir ces helpers, qui exposent des types brut (`String`
    // file_path, `i64` size_bytes) plutôt que des projections
    // domaine.
    // -----------------------------------------------------------------------

    /// Liste les `(id, file_path, size_bytes)` de tous les `media_item`
    /// rattachés au dossier `folder_id`, indépendamment de leur statut
    /// `available` (le scanner ré-évalue la disponibilité à chaque
    /// passe). Les fiches sans `file_path` (Series/Anime racine, cf.
    /// commentaire DDL `media_item.file_path TEXT` non NOT NULL) sont
    /// **exclues** : elles ne représentent pas un fichier sur disque.
    ///
    /// Visibilité restreinte : `pub(crate)` réservé au scanner.
    pub(crate) async fn list_media_under_folder(
        &self,
        folder_id: FolderId,
    ) -> Result<Vec<(MediaItemId, String, Option<i64>)>, KyroError> {
        let rows = sqlx::query(
            "SELECT id, file_path, size_bytes \
               FROM media_item \
              WHERE folder_id = ? AND file_path IS NOT NULL",
        )
        .bind(folder_id.get())
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let id: i64 = row.try_get("id")?;
            let file_path: String = row.try_get("file_path")?;
            let size_bytes: Option<i64> = row.try_get("size_bytes")?;
            out.push((MediaItemId(id), file_path, size_bytes));
        }
        Ok(out)
    }

    /// Liste les `(id, file_path, size_bytes)` de tous les
    /// `episode_item` dont la série parent est rattachée au dossier
    /// `folder_id`. Suit la chaîne `episode_item.series_id →
    /// media_item.folder_id`.
    ///
    /// Visibilité restreinte : `pub(crate)` réservé au scanner.
    pub(crate) async fn list_episodes_under_folder(
        &self,
        folder_id: FolderId,
    ) -> Result<Vec<(EpisodeItemId, String, Option<i64>)>, KyroError> {
        let rows = sqlx::query(
            "SELECT e.id AS id, e.file_path AS file_path, e.size_bytes AS size_bytes \
               FROM episode_item e \
               JOIN media_item m ON m.id = e.series_id \
              WHERE m.folder_id = ?",
        )
        .bind(folder_id.get())
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let id: i64 = row.try_get("id")?;
            let file_path: String = row.try_get("file_path")?;
            let size_bytes: Option<i64> = row.try_get("size_bytes")?;
            out.push((EpisodeItemId(id), file_path, size_bytes));
        }
        Ok(out)
    }

    /// Marque une fiche `media_item` indisponible (`available = 0`)
    /// **sans la supprimer** (requirement 3.5). Idempotent : appel
    /// répété conserve l'état `0`.
    ///
    /// Visibilité restreinte : `pub(crate)` réservé au scanner.
    pub(crate) async fn mark_media_unavailable(
        &self,
        id: MediaItemId,
    ) -> Result<(), KyroError> {
        sqlx::query("UPDATE media_item SET available = 0 WHERE id = ?")
            .bind(id.get())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Marque un épisode indisponible (`available = 0`) sans le
    /// supprimer (requirement 3.5).
    ///
    /// Visibilité restreinte : `pub(crate)` réservé au scanner.
    pub(crate) async fn mark_episode_unavailable(
        &self,
        id: EpisodeItemId,
    ) -> Result<(), KyroError> {
        sqlx::query("UPDATE episode_item SET available = 0 WHERE id = ?")
            .bind(id.get())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Réhabilite une fiche `media_item` (`available = 1`) et bump
    /// `last_scanned_at`. Utilisé par le scanner pour le « touch »
    /// idempotent quand un fichier précédemment indexé est revu sur
    /// disque sans changement de taille (typiquement après un
    /// rename/delete-puis-restore d'un sous-dossier de bibliothèque).
    /// Retourne `true` si la fiche était précédemment indisponible et
    /// vient d'être réhabilitée — utile au scanner pour incrémenter le
    /// compteur de réhabilitations dans `ScanReport`.
    ///
    /// Visibilité restreinte : `pub(crate)` réservé au scanner.
    pub(crate) async fn touch_media_available(
        &self,
        id: MediaItemId,
    ) -> Result<bool, KyroError> {
        let was_unavailable: Option<i64> = sqlx::query_scalar::<_, i64>(
            "SELECT available FROM media_item WHERE id = ?",
        )
        .bind(id.get())
        .fetch_optional(&self.pool)
        .await?;
        let restored = matches!(was_unavailable, Some(0));

        sqlx::query(
            "UPDATE media_item \
                SET available = 1, last_scanned_at = CURRENT_TIMESTAMP \
              WHERE id = ?",
        )
        .bind(id.get())
        .execute(&self.pool)
        .await?;
        Ok(restored)
    }

    /// Réhabilite un `episode_item` (`available = 1`) et bump
    /// `last_scanned_at`. Pendant strict de [`Self::touch_media_available`]
    /// pour la table `episode_item`. Retourne `true` si l'épisode
    /// était précédemment indisponible.
    ///
    /// Visibilité restreinte : `pub(crate)` réservé au scanner.
    pub(crate) async fn touch_episode_available(
        &self,
        id: EpisodeItemId,
    ) -> Result<bool, KyroError> {
        let was_unavailable: Option<i64> = sqlx::query_scalar::<_, i64>(
            "SELECT available FROM episode_item WHERE id = ?",
        )
        .bind(id.get())
        .fetch_optional(&self.pool)
        .await?;
        let restored = matches!(was_unavailable, Some(0));

        sqlx::query(
            "UPDATE episode_item \
                SET available = 1, last_scanned_at = CURRENT_TIMESTAMP \
              WHERE id = ?",
        )
        .bind(id.get())
        .execute(&self.pool)
        .await?;
        Ok(restored)
    }

    /// Résout un chemin de fichier absolu (forme stockée DB) vers la
    /// cible de progression correspondante (`Episode` ou `Media`).
    ///
    /// Stratégie : on cherche d'abord dans `episode_item.file_path`
    /// (UNIQUE NOT NULL) puis, en l'absence de match, dans
    /// `media_item.file_path` (utile pour les Films, dont la fiche
    /// porte directement le `file_path` du fichier vidéo).
    ///
    /// Retourne `None` si aucun enregistrement ne porte ce chemin
    /// (cas typique : mpv playlist-next vers un fichier hors
    /// bibliothèque, ou chemin avec préfixe `\\?\` qui ne matche pas
    /// la forme canonicalisée — l'appelant doit alors ignorer le
    /// switch et continuer à écrire sur la cible courante).
    ///
    /// Utilisé par le `Player_Bridge` quand mpv signale un changement
    /// de fichier en cours de session IPC (playlist-next ou
    /// auto-load-next), afin de basculer la cible de
    /// `set_watch_progress` vers le bon épisode.
    pub async fn find_progress_target_by_path(
        &self,
        file_path: &str,
    ) -> Result<Option<ProgressTarget>, KyroError> {
        // Canonicaliser le chemin reçu de mpv vers la même forme que
        // celle persistée en base (cf. `add_library_folder` / scanner
        // qui utilisent `canonicalize_or_keep`). Sans ça un même
        // fichier peut avoir deux représentations distinctes :
        //   - mpv : `D:\anime\E02.mkv`
        //   - DB  : `\\?\D:\anime\E02.mkv` (préfixe UNC retourné par
        //     std::fs::canonicalize sur Windows)
        // Le LIKE final couvre aussi le cas où mpv aurait remplacé
        // les backslashes par des forward slashes (rare mais observé).
        let canonical = canonicalize_or_keep(Path::new(file_path));
        let canonical_str = canonical.to_string_lossy().into_owned();

        // Helper pour faire un lookup avec deux variantes : exacte et
        // canonicalisée. On essaie le path tel quel d'abord (cas
        // nominal), puis le canonicalisé en fallback.
        let candidates: Vec<&str> = if canonical_str == file_path {
            vec![file_path]
        } else {
            vec![file_path, canonical_str.as_str()]
        };

        for candidate in &candidates {
            let ep: Option<i64> = sqlx::query_scalar::<_, i64>(
                "SELECT id FROM episode_item WHERE file_path = ? LIMIT 1",
            )
            .bind(candidate)
            .fetch_optional(&self.pool)
            .await?;
            if let Some(id) = ep {
                return Ok(Some(ProgressTarget::Episode(EpisodeItemId(id))));
            }

            let media: Option<i64> = sqlx::query_scalar::<_, i64>(
                "SELECT id FROM media_item WHERE file_path = ? LIMIT 1",
            )
            .bind(candidate)
            .fetch_optional(&self.pool)
            .await?;
            if let Some(id) = media {
                return Ok(Some(ProgressTarget::Media(MediaItemId(id))));
            }
        }

        // Dernier recours : LIKE sur le **nom de fichier final** pour
        // couvrir des écarts de préfixe (UNC, lecteur réseau remappé,
        // chemins relatifs/absolus). Risque de faux positifs très
        // limité : deux épisodes ne portent pas le même nom dans une
        // bibliothèque normale.
        if let Some(file_name) = canonical.file_name().and_then(|n| n.to_str()) {
            let pattern = format!("%{}", file_name);
            let ep: Option<i64> = sqlx::query_scalar::<_, i64>(
                "SELECT id FROM episode_item WHERE file_path LIKE ? LIMIT 1",
            )
            .bind(&pattern)
            .fetch_optional(&self.pool)
            .await?;
            if let Some(id) = ep {
                return Ok(Some(ProgressTarget::Episode(EpisodeItemId(id))));
            }

            let media: Option<i64> = sqlx::query_scalar::<_, i64>(
                "SELECT id FROM media_item WHERE file_path LIKE ? LIMIT 1",
            )
            .bind(&pattern)
            .fetch_optional(&self.pool)
            .await?;
            if let Some(id) = media {
                return Ok(Some(ProgressTarget::Media(MediaItemId(id))));
            }
        }

        Ok(None)
    }

    /// Liste les séries/animes (`media_item.type IN ('Series','Anime')`)
    /// rattachées à un dossier de bibliothèque. Ne retourne que les
    /// fiches « racine de série » (sans `file_path` propre, ce qui les
    /// distingue des Films). Retourne `(id, available_actuel)` pour que
    /// la réconciliation puisse n'écrire que si l'état change.
    ///
    /// Inclut la série placeholder « À classer » : sa logique de
    /// disponibilité est identique aux séries réelles (disponible ssi
    /// au moins un épisode rattaché est disponible).
    ///
    /// Visibilité restreinte : `pub(crate)` réservé au scanner.
    pub(crate) async fn list_series_under_folder(
        &self,
        folder_id: FolderId,
    ) -> Result<Vec<(MediaItemId, bool)>, KyroError> {
        let rows = sqlx::query(
            "SELECT id, available \
               FROM media_item \
              WHERE folder_id = ? \
                AND type IN ('Series','Anime') \
                AND (file_path IS NULL OR file_path = '')",
        )
        .bind(folder_id.get())
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let id: i64 = row.try_get("id")?;
            let available: i64 = row.try_get("available")?;
            out.push((MediaItemId(id), available != 0));
        }
        Ok(out)
    }

    /// Compte les épisodes d'une série dont `available = 1`. Utilisé
    /// par la réconciliation du scanner pour décider de la
    /// disponibilité d'une série/anime parente.
    ///
    /// Visibilité restreinte : `pub(crate)` réservé au scanner.
    pub(crate) async fn count_available_episodes(
        &self,
        series_id: MediaItemId,
    ) -> Result<i64, KyroError> {
        let n: i64 = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM episode_item \
              WHERE series_id = ? AND available = 1",
        )
        .bind(series_id.get())
        .fetch_one(&self.pool)
        .await?;
        Ok(n)
    }

    /// Trouve ou crée la **série placeholder « À classer »** rattachée
    /// au dossier `folder_id`, dans laquelle on range les épisodes
    /// dont la numérotation n'a pas pu être reconnue (requirement 4.7).
    ///
    /// Sémantique :
    /// - **Un placeholder par dossier** (et par catégorie, mais la
    ///   catégorie est dérivée du dossier — pas besoin de clé
    ///   composée). Cela évite de mélanger des épisodes Series et
    ///   Animes dans une même série fictive si l'utilisateur a deux
    ///   dossiers de catégories différentes.
    /// - Le titre est `À classer`, le `media_type` matérialisé est
    ///   `MediaType::Series` (catégorie `Series`) ou `MediaType::Anime`
    ///   (catégorie `Animes`). Pour la catégorie `Films`, ce
    ///   placeholder n'a pas de sens et la fonction retourne
    ///   `KyroError::Validation`.
    /// - La série placeholder n'a **pas de `file_path`** (NULL) : elle
    ///   ne représente pas un fichier réel, donc
    ///   `list_media_under_folder` (qui filtre `file_path IS NOT NULL`)
    ///   ne la verra pas, ce qui évite que le scanner la marque
    ///   « disparue ».
    ///
    /// Visibilité restreinte : `pub(crate)` réservé au scanner.
    pub(crate) async fn ensure_unsorted_series(
        &self,
        folder_id: FolderId,
        category: Category,
    ) -> Result<MediaItemId, KyroError> {
        let media_type = match category {
            Category::Series => MediaType::Series,
            Category::Animes => MediaType::Anime,
            Category::Films => {
                return Err(KyroError::Validation(
                    "le pseudo-dossier « À classer » ne s'applique pas aux Films"
                        .to_string(),
                ));
            }
        };

        // Cherche d'abord une série placeholder existante : titre `À
        // classer`, type adéquat, rattachée au même `folder_id`, sans
        // `file_path`. La triple condition garantit qu'on ne réutilise
        // pas par erreur une fiche utilisateur qui aurait `À classer`
        // pour titre.
        let existing: Option<i64> = sqlx::query_scalar::<_, i64>(
            "SELECT id FROM media_item \
              WHERE folder_id = ? AND type = ? AND title = ? AND file_path IS NULL \
              LIMIT 1",
        )
        .bind(folder_id.get())
        .bind(media_type.as_str())
        .bind(UNSORTED_SERIES_TITLE)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(id) = existing {
            return Ok(MediaItemId(id));
        }

        // Pas trouvé → on insère. `file_path = NULL`, `size_bytes =
        // NULL`, valeurs par défaut DDL pour le reste.
        let id: i64 = sqlx::query_scalar::<_, i64>(
            "INSERT INTO media_item (folder_id, type, title, available) \
             VALUES (?, ?, ?, 1) RETURNING id",
        )
        .bind(folder_id.get())
        .bind(media_type.as_str())
        .bind(UNSORTED_SERIES_TITLE)
        .fetch_one(&self.pool)
        .await?;

        tracing::info!(
            folder_id = folder_id.get(),
            media_item_id = id,
            category = category.as_str(),
            "Kyro: série placeholder « À classer » créée"
        );
        Ok(MediaItemId(id))
    }
}

/// Titre exact du pseudo-dossier « À classer » pour les épisodes dont
/// la numérotation n'a pas pu être reconnue (requirement 4.7, design
/// § « Kyro_Parser »). Constante exposée pub(crate) afin que le
/// scanner et les tests puissent la référencer sans dupliquer le
/// littéral.
pub(crate) const UNSORTED_SERIES_TITLE: &str = "À classer";

// ---------------------------------------------------------------------------
// Helpers FTS5 — recherche v1.0.
// ---------------------------------------------------------------------------

/// Caractères réservés par la syntaxe FTS5 qui doivent être supprimés
/// d'un token avant de l'enrober dans des guillemets — sinon SQLite
/// peut les interpréter comme des opérateurs (`*`, `(`, `)`) ou comme
/// des séparateurs de colonne (`:`).
///
/// Le guillemet double `"` est traité séparément (FTS5 l'échappe en
/// le doublant à l'intérieur d'une string littérale).
fn fts_strip_reserved(c: char) -> bool {
    matches!(c, '*' | '(' | ')' | ':')
}

/// Construit une requête FTS5 `MATCH` à partir d'une chaîne libre.
///
/// Stratégie :
///   1. Split sur les espaces blancs Unicode.
///   2. Pour chaque token, retire les caractères FTS réservés et
///      échappe les guillemets doubles (`"` → `""`).
///   3. Si le token résiduel est vide, on l'ignore.
///   4. On enrobe chaque token dans des guillemets doubles (protège
///      les mots-clés FTS comme `AND`/`OR`/`NOT`).
///   5. On suffixe `*` pour rendre chaque token un préfixe (FTS5
///      considère `"foo"*` comme « tous les termes commençant par
///      foo »), ce qui préserve la sémantique « contient au début
///      de mot » historique de Kyro (cf. `domain::search::matches`).
///   6. Les tokens nettoyés sont joints par espace : FTS5 traite
///      l'espace comme un AND implicite (tous les tokens doivent
///      matcher la fiche).
///
/// Retourne `""` si la query nettoyée ne contient aucun token utile —
/// l'appelant doit alors retourner une liste vide sans tenter la
/// requête FTS (sinon SQLite renvoie une erreur de parsing).
pub(crate) fn build_fts_match_query(query: &str) -> String {
    let mut parts = Vec::new();
    for raw_tok in query.split_whitespace() {
        let cleaned: String = raw_tok
            .chars()
            .filter(|c| !fts_strip_reserved(*c))
            .collect::<String>()
            .replace('"', "\"\"");
        if cleaned.is_empty() {
            continue;
        }
        parts.push(format!("\"{}\"*", cleaned));
    }
    parts.join(" ")
}

/// Implémentation partagée des toggles `favorite` / `to_watch` (tâche
/// 7.6). Le nom de colonne provient d'une **allowlist statique** côté
/// appelant (`"favorite"` ou `"to_watch"`) — il n'est jamais issu d'une
/// entrée utilisateur, donc l'interpolation directe dans le SQL ne
/// crée pas de risque d'injection. On garde malgré tout un `match`
/// défensif pour rendre l'intention explicite et empêcher toute
/// extension future qui passerait un nom arbitraire.
async fn toggle_media_flag(
    pool: &SqlitePool,
    id: MediaItemId,
    column: &'static str,
) -> Result<bool, KyroError> {
    let sql = match column {
        "favorite" => {
            "UPDATE media_item SET favorite = 1 - favorite WHERE id = ? RETURNING favorite"
        }
        "to_watch" => {
            "UPDATE media_item SET to_watch = 1 - to_watch WHERE id = ? RETURNING to_watch"
        }
        // Ce cas est impossible — il signifierait un appel interne
        // erroné. On le matérialise comme erreur de validation pour
        // garder la signature `Result` cohérente.
        other => {
            return Err(KyroError::Validation(format!(
                "colonne de toggle non supportée: {other:?}"
            )));
        }
    };

    let mut tx = pool.begin().await?;
    let new_value: Option<i64> = sqlx::query_scalar::<_, i64>(sql)
        .bind(id.get())
        .fetch_optional(&mut *tx)
        .await?;
    let value = new_value.ok_or_else(|| {
        KyroError::Validation(format!(
            "media_item id={} introuvable pour toggle_{}",
            id.get(),
            column
        ))
    })?;
    tx.commit().await?;

    let new_state = value != 0;
    tracing::debug!(
        media_item_id = id.get(),
        column,
        new_state,
        "Kyro: toggle_media_flag committé"
    );
    Ok(new_state)
}

/// Cible d'une mutation de progression / statut.
///
/// Distingue explicitement un Media_Item (Film, racine de Series ou
/// d'Anime) d'un Episode_Item afin d'éviter toute confusion dans la
/// signature des commandes Tauri (tâche 12.x).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressTarget {
    Media(MediaItemId),
    Episode(EpisodeItemId),
}

/// Vue lecture seule d'une ligne `episode_item` retournée par
/// [`Repo::list_episodes`] (tâche 7.4).
///
/// La projection est volontairement minimale : seuls les champs requis
/// par les pages frontend `/detail/:id` (affichage de la liste des
/// épisodes ordonnée saison/épisode + statut + indicateur
/// d'indisponibilité) et par
/// [`crate::domain::resume::next_resume`] sont exposés. Les champs
/// techniques (durée, codec, watch_progress…) seront ajoutés
/// ultérieurement à la demande, ou consommés via une projection
/// dédiée des commandes Tauri.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpisodeRow {
    /// Identifiant typé de l'épisode (`episode_item.id`).
    pub id: EpisodeItemId,
    /// Numéro de saison (`episode_item.season_number`).
    pub season_number: i64,
    /// Numéro d'épisode dans la saison (`episode_item.episode_number`).
    pub episode_number: i64,
    /// Statut de visionnage courant (cf. `domain::status::Status`).
    pub status: Status,
    /// Drapeau de disponibilité (`episode_item.available`).
    pub available: bool,
    /// Position de lecture courante en secondes
    /// (`episode_item.watch_progress_seconds`). Permet à l'UI
    /// d'afficher « X min restantes » sans requête supplémentaire.
    pub watch_progress_seconds: i64,
    /// Durée totale en secondes (`episode_item.duration_seconds`,
    /// `NULL` tant que ffprobe n'a pas pu la mesurer).
    pub duration_seconds: Option<i64>,
}

// ---------------------------------------------------------------------------
// Helpers internes.
// ---------------------------------------------------------------------------

/// Canonicalise `path` ; en cas d'échec, retourne le chemin original
/// converti en `PathBuf`. La canonicalisation est best-effort :
/// requirement 2.2 demande un « chemin absolu », pas une forme
/// canonique stricte. Sur Windows on évite ainsi les surprises liées
/// aux préfixes UNC `\\?\` qui apparaissent parfois et qui pourraient
/// gêner un affichage utilisateur.
///
/// Visibilité `pub(crate)` afin que `infra::scanner` (tâche 8.x) puisse
/// produire des chaînes de chemin **identiques** à celles persistées
/// par `add_library_folder` / `upsert_media_item`. Sans ce partage, un
/// fichier réel et son entrée DB n'auraient pas la même clé `file_path`
/// et le diff scanner deviendrait incorrect.
pub(crate) fn canonicalize_or_keep(path: &Path) -> PathBuf {
    match std::fs::canonicalize(path) {
        Ok(p) => p,
        Err(err) => {
            tracing::debug!(
                error = %err,
                path = %path.display(),
                "Kyro: canonicalize a échoué — conservation du chemin original"
            );
            path.to_path_buf()
        }
    }
}

/// Mappe une `SqliteRow` (`id`, `path`, `category`) vers
/// [`LibraryFolder`]. Une catégorie illisible (valeur hors CHECK)
/// produit une `KyroError::Db(sqlx::Error::Decode)` synthétique afin
/// que le caller puisse la propager naturellement vers
/// `KyroError::Db`.
fn library_folder_from_row(row: &SqliteRow) -> Result<LibraryFolder, KyroError> {
    let id: i64 = row.try_get("id")?;
    let path: String = row.try_get("path")?;
    let category_str: String = row.try_get("category")?;
    let category = parse_category(&category_str).ok_or_else(|| {
        // On encapsule via `sqlx::Error::Decode` pour rester dans la
        // famille `KyroError::Db` (cohérent avec un défaut de schéma
        // côté DB plutôt qu'une erreur de validation utilisateur).
        sqlx::Error::Decode(
            format!("catégorie inconnue dans library_folder.category: {category_str:?}").into(),
        )
    })?;
    Ok(LibraryFolder {
        id: FolderId(id),
        path,
        category,
    })
}

/// Parse la chaîne stockée en base vers une variante de `Category`.
/// Les littéraux sont strictement ceux du `CHECK SQL`.
fn parse_category(raw: &str) -> Option<Category> {
    match raw {
        "Films" => Some(Category::Films),
        "Series" => Some(Category::Series),
        "Animes" => Some(Category::Animes),
        _ => None,
    }
}

/// Convertit un `bool` Rust vers le `INTEGER` 0/1 attendu par les
/// colonnes booléennes du DDL (CHECK `IN (0,1)`).
fn bool_to_i64(b: bool) -> i64 {
    if b {
        1
    } else {
        0
    }
}

/// Mapping `Category` (catégorie de dossier de bibliothèque, pluriel)
/// → `MediaType` (type de Media_Item, singulier) utilisé par
/// `list_by_category` (tâche 7.4). La correspondance est 1-1 et fixée
/// par le requirement 7.2 : un dossier de catégorie `Films` contient
/// uniquement des Media_Item de type `Film`, etc.
fn category_to_media_type(category: Category) -> MediaType {
    match category {
        Category::Films => MediaType::Film,
        Category::Series => MediaType::Series,
        Category::Animes => MediaType::Anime,
    }
}

/// Vue intermédiaire utilisée par
/// [`Repo::materialize_media_items`] : agrège les colonnes `media_item`
/// projetées depuis SQLite avant l'attachement des genres (chargés en
/// batch dans une seconde requête).
struct MediaItemBare {
    id: i64,
    title: String,
    original_title: Option<String>,
    year: Option<u16>,
    media_type: MediaType,
    status: Status,
    poster_path: Option<String>,
}

/// Mappe une ligne `(id, title, original_title, year, type, status)`
/// projetée depuis `media_item` vers la vue intermédiaire
/// `MediaItemBare`. Toute valeur incohérente (type ou status hors
/// CHECK SQL) est traduite en `sqlx::Error::Decode` pour être propagée
/// via `KyroError::Db` — ce qui correspondrait à une migration
/// corrompue ou à une intervention manuelle sur la base.
fn media_item_bare_from_row(row: &SqliteRow) -> Result<MediaItemBare, KyroError> {
    let id: i64 = row.try_get("id")?;
    let title: String = row.try_get("title")?;
    let original_title: Option<String> = row.try_get("original_title")?;
    let year_i64: Option<i64> = row.try_get("year")?;
    let type_str: String = row.try_get("type")?;
    let status_str: String = row.try_get("status")?;

    let media_type = parse_media_type(&type_str).ok_or_else(|| {
        sqlx::Error::Decode(
            format!("type inconnu dans media_item.type: {type_str:?}").into(),
        )
    })?;
    let status: Status = status_str.parse().map_err(|err: ParseStatusError| {
        sqlx::Error::Decode(
            format!("statut inconnu dans media_item.status: {}", err.value).into(),
        )
    })?;

    // Le DDL n'impose pas de bornes [1900, 2099] côté DB ; on tente la
    // conversion en u16 pour respecter le type du domaine et on tombe
    // sur `None` en cas de débordement (cohérent avec une fiche dont
    // l'année a été nettoyée par migration).
    let year = year_i64.and_then(|v| u16::try_from(v).ok());

    // `poster_path` est une colonne optionnelle ajoutée à la
    // projection des listings : `None` quand aucune affiche n'a été
    // importée, sinon le chemin absolu local persisté par
    // `import_poster`. Si la colonne est absente de la projection
    // (cas legacy), on retombe silencieusement sur `None`.
    let poster_path: Option<String> = row
        .try_get::<Option<String>, _>("poster_path")
        .unwrap_or(None);

    Ok(MediaItemBare {
        id,
        title,
        original_title,
        year,
        media_type,
        status,
        poster_path,
    })
}

/// Parse la chaîne stockée dans `media_item.type` vers `MediaType`.
/// Litéraux strictement alignés sur le `CHECK SQL` (`Film | Series |
/// Anime`).
fn parse_media_type(raw: &str) -> Option<MediaType> {
    match raw {
        "Film" => Some(MediaType::Film),
        "Series" => Some(MediaType::Series),
        "Anime" => Some(MediaType::Anime),
        _ => None,
    }
}

/// Mappe une `SqliteRow` (`id, season_number, episode_number, status,
/// available`) vers [`EpisodeRow`].
fn episode_row_from_row(row: &SqliteRow) -> Result<EpisodeRow, KyroError> {
    let id: i64 = row.try_get("id")?;
    let season_number: i64 = row.try_get("season_number")?;
    let episode_number: i64 = row.try_get("episode_number")?;
    let status_str: String = row.try_get("status")?;
    let available_int: i64 = row.try_get("available")?;
    let status: Status = status_str.parse().map_err(|err: ParseStatusError| {
        sqlx::Error::Decode(
            format!("statut inconnu dans episode_item.status: {}", err.value).into(),
        )
    })?;
    // Champs optionnels — projetés uniquement par les requêtes les plus
    // récentes (ex. `list_episodes`). Pour les anciennes projections
    // qui ne demandent pas ces colonnes, `try_get` retourne `Err` et on
    // retombe sur des valeurs neutres (progression = 0, durée = None)
    // pour rester rétrocompatible avec les call-sites historiques.
    let watch_progress_seconds: i64 = row
        .try_get::<i64, _>("watch_progress_seconds")
        .unwrap_or(0);
    let duration_seconds: Option<i64> = row
        .try_get::<Option<i64>, _>("duration_seconds")
        .unwrap_or(None);
    Ok(EpisodeRow {
        id: EpisodeItemId(id),
        season_number,
        episode_number,
        status,
        available: available_int != 0,
        watch_progress_seconds,
        duration_seconds,
    })
}

/// Convertit une vue `EpisodeRow` (couche infra) vers la vue
/// `domain::resume::EpisodeItem` consommée par `next_resume`. Les
/// numéros de saison/épisode sont saturés à `u16::MAX` plutôt que de
/// faire échouer la conversion : aucune série/anime réelle n'atteint
/// ces bornes, et un débordement isolé ne doit pas casser le calcul de
/// reprise pour les autres épisodes.
fn episode_row_to_domain(row: &EpisodeRow) -> resume::EpisodeItem {
    resume::EpisodeItem {
        id: row.id.get(),
        season_number: u16::try_from(row.season_number).unwrap_or(u16::MAX),
        episode_number: u16::try_from(row.episode_number).unwrap_or(u16::MAX),
        status: row.status,
        available: row.available,
    }
}

// ---------------------------------------------------------------------------
// Tests unitaires.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use sqlx::sqlite::{
        SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
    };
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Construit un runtime `tokio` `current_thread` dédié au test. On
    /// n'utilise pas `#[tokio::test]` car la feature `macros` de `tokio`
    /// n'est pas activée dans `Cargo.toml` (cf. `tests/migrations.rs`
    /// pour la même stratégie côté tests d'intégration).
    fn block_on<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("création d'un runtime tokio current_thread")
            .block_on(fut)
    }

    /// Ouvre un pool SQLite sur un fichier temporaire, applique les
    /// migrations, et retourne le pool prêt pour les tests. On
    /// reproduit fidèlement les pragmas de `init_sqlite_pool`
    /// (`main.rs`) afin que les tests exercent la même configuration
    /// que la production (notamment `foreign_keys=ON` pour le
    /// `ON DELETE SET NULL` sur `media_item.folder_id`).
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

    /// Insère un `media_item` minimal lié au folder donné, avec
    /// `available = 1`. Retourne l'id de l'item inséré.
    async fn insert_media_item(pool: &SqlitePool, folder_id: i64, title: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(
            "INSERT INTO media_item (folder_id, type, title, available) \
             VALUES (?, 'Film', ?, 1) RETURNING id",
        )
        .bind(folder_id)
        .bind(title)
        .fetch_one(pool)
        .await
        .expect("insertion media_item de test")
    }

    /// Récupère le champ `available` d'un `media_item`.
    async fn read_available(pool: &SqlitePool, media_id: i64) -> i64 {
        sqlx::query_scalar::<_, i64>("SELECT available FROM media_item WHERE id = ?")
            .bind(media_id)
            .fetch_one(pool)
            .await
            .expect("lecture media_item.available")
    }

    /// Compte les lignes de `library_folder` portant l'id donné.
    async fn count_folder_rows(pool: &SqlitePool, folder_id: i64) -> i64 {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM library_folder WHERE id = ?",
        )
        .bind(folder_id)
        .fetch_one(pool)
        .await
        .expect("count library_folder")
    }

    /// Compte total de `media_item` (toute available confondue).
    async fn count_all_media(pool: &SqlitePool) -> i64 {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM media_item")
            .fetch_one(pool)
            .await
            .expect("count media_item")
    }

    #[test]
    fn add_then_list_returns_inserted_folder() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let lib_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool);

            let id = repo
                .add_library_folder(lib_dir.path(), Category::Films)
                .await
                .expect("ajout doit réussir sur un dossier existant");

            let listed = repo.list_library_folders().await.expect("list ok");
            assert_eq!(listed.len(), 1);
            assert_eq!(listed[0].id, id);
            assert_eq!(listed[0].category, Category::Films);

            // Le path stocké doit être absolu et désigner le même dossier
            // que celui passé en entrée. On compare via canonicalize pour
            // absorber les variantes de préfixe UNC sous Windows.
            let stored = PathBuf::from(&listed[0].path);
            let expected = std::fs::canonicalize(lib_dir.path())
                .unwrap_or_else(|_| lib_dir.path().to_path_buf());
            assert_eq!(stored, expected);
        });
    }

    #[test]
    fn add_rejects_nonexistent_path_with_validation_error() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let scratch = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool);

            // Chemin garanti inexistant : sous-dossier jamais créé d'un
            // TempDir éphémère.
            let bogus = scratch.path().join("does-not-exist-xyz");
            let err = repo
                .add_library_folder(&bogus, Category::Series)
                .await
                .expect_err("un chemin inexistant doit être rejeté");

            match err {
                KyroError::Validation(msg) => {
                    assert!(
                        msg.contains("does-not-exist-xyz"),
                        "le message doit citer le chemin invalide (got: {msg:?})"
                    );
                }
                other => panic!("expected Validation, got {other:?}"),
            }

            // Aucune ligne n'a dû être insérée.
            let listed = repo.list_library_folders().await.unwrap();
            assert!(listed.is_empty());
        });
    }

    #[test]
    fn add_rejects_file_path_with_validation_error() {
        block_on(async {
            // Bord rare : un fichier existant (pas un dossier) doit être
            // refusé avec la même erreur que pour un chemin inexistant.
            let db_dir = TempDir::new().unwrap();
            let lib_dir = TempDir::new().unwrap();
            let file_path = lib_dir.path().join("not-a-dir.txt");
            std::fs::write(&file_path, b"hello").unwrap();

            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool);

            let err = repo
                .add_library_folder(&file_path, Category::Animes)
                .await
                .expect_err("un fichier ne doit pas être accepté comme dossier");
            assert!(matches!(err, KyroError::Validation(_)));
        });
    }

    #[test]
    fn remove_marks_media_unavailable_and_drops_folder_row() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let lib_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            // 1. Ajouter le dossier.
            let folder_id = repo
                .add_library_folder(lib_dir.path(), Category::Films)
                .await
                .unwrap();

            // 2. Insérer un media_item rattaché au dossier (available=1).
            let media_id = insert_media_item(&pool, folder_id.get(), "Movie A").await;
            assert_eq!(read_available(&pool, media_id).await, 1);

            // 3. Retirer le dossier.
            repo.remove_library_folder(folder_id).await.unwrap();

            // 4. La ligne library_folder a disparu.
            assert_eq!(count_folder_rows(&pool, folder_id.get()).await, 0);

            // 5. Le media_item est toujours là, mais available = 0.
            //    NB : `ON DELETE SET NULL` met `folder_id` à NULL ;
            //    l'UPDATE précédant le DELETE s'est chargé du flag
            //    `available`.
            assert_eq!(count_all_media(&pool).await, 1);
            assert_eq!(read_available(&pool, media_id).await, 0);
        });
    }

    #[test]
    fn remove_unknown_id_is_silently_ok() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool);

            // Aucune ligne `library_folder` avec id=42 — la suppression
            // doit réussir sans erreur (idempotence côté UI).
            repo.remove_library_folder(FolderId(42))
                .await
                .expect("remove sur id inconnu doit être silencieusement Ok");
        });
    }

    // -----------------------------------------------------------------------
    // Tests tâche 7.2 — upsert_media_item / upsert_episode_item.
    // -----------------------------------------------------------------------

    /// Construit un `UpsertMediaInput` plausible pour un Film.
    fn film_input(folder_id: Option<FolderId>, file_path: &str, size: i64) -> UpsertMediaInput {
        UpsertMediaInput {
            folder_id,
            media_type: MediaType::Film,
            file_path: file_path.to_string(),
            size_bytes: size,
            title: "Inception".to_string(),
            year: Some(2010),
            quality: Some("1080p".to_string()),
            duration_seconds: Some(8880),
            video_codec: Some("h264".to_string()),
            width: Some(1920),
            height: Some(1080),
            numbering_unknown: false,
        }
    }

    /// Construit un `UpsertEpisodeInput` plausible.
    fn episode_input(
        series_id: MediaItemId,
        file_path: &str,
        size: i64,
        season: i64,
        episode: i64,
    ) -> UpsertEpisodeInput {
        UpsertEpisodeInput {
            series_id,
            season_number: season,
            episode_number: episode,
            file_path: file_path.to_string(),
            size_bytes: size,
            duration_seconds: Some(2400),
            video_codec: Some("h264".to_string()),
            width: Some(1920),
            height: Some(1080),
            numbering_unknown: false,
        }
    }

    /// Récupère une ligne `media_item` complète sous forme de tuple
    /// type-sûr aligné sur les colonnes utiles aux assertions.
    async fn read_media_row(
        pool: &SqlitePool,
        id: i64,
    ) -> (
        i64,            // size_bytes
        String,         // title
        Option<i64>,    // year
        Option<String>, // quality
        Option<i64>,    // duration_seconds
        Option<String>, // video_codec
        Option<i64>,    // width
        Option<i64>,    // height
        i64,            // watch_progress_seconds
        String,         // status
        i64,            // favorite
        i64,            // to_watch
        i64,            // available
        i64,            // numbering_unknown
    ) {
        sqlx::query_as::<_, (
            Option<i64>,
            String,
            Option<i64>,
            Option<String>,
            Option<i64>,
            Option<String>,
            Option<i64>,
            Option<i64>,
            i64,
            String,
            i64,
            i64,
            i64,
            i64,
        )>(
            "SELECT size_bytes, title, year, quality, duration_seconds, video_codec, \
                    width, height, watch_progress_seconds, status, favorite, to_watch, \
                    available, numbering_unknown \
               FROM media_item WHERE id = ?",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .map(|r| {
            (
                r.0.expect("size_bytes attendu non NULL après insert"),
                r.1,
                r.2,
                r.3,
                r.4,
                r.5,
                r.6,
                r.7,
                r.8,
                r.9,
                r.10,
                r.11,
                r.12,
                r.13,
            )
        })
        .expect("lecture media_item")
    }

    #[test]
    fn upsert_media_inserts_new_film_with_default_user_fields() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let lib_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            let folder = repo
                .add_library_folder(lib_dir.path(), Category::Films)
                .await
                .unwrap();

            let input = film_input(Some(folder), "C:/movies/Inception.2010.1080p.mkv", 4_500_000_000);
            let id = repo.upsert_media_item(&input).await.expect("insert ok");

            let row = read_media_row(&pool, id.get()).await;
            assert_eq!(row.0, 4_500_000_000);
            assert_eq!(row.1, "Inception");
            assert_eq!(row.2, Some(2010));
            assert_eq!(row.3.as_deref(), Some("1080p"));
            assert_eq!(row.4, Some(8880));
            assert_eq!(row.5.as_deref(), Some("h264"));
            assert_eq!(row.6, Some(1920));
            assert_eq!(row.7, Some(1080));
            // Champs utilisateur — valeurs par défaut.
            assert_eq!(row.8, 0, "watch_progress_seconds doit être 0 par défaut");
            assert_eq!(row.9, "non_vu");
            assert_eq!(row.10, 0, "favorite par défaut");
            assert_eq!(row.11, 0, "to_watch par défaut");
            assert_eq!(row.12, 1, "available par défaut");
            assert_eq!(row.13, 0, "numbering_unknown attendu false");
        });
    }

    #[test]
    fn upsert_media_is_idempotent_when_size_unchanged() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            let path = "C:/movies/Inception.2010.1080p.mkv";
            let input = film_input(None, path, 4_500_000_000);
            let id1 = repo.upsert_media_item(&input).await.unwrap();

            // Re-upsert avec exactement le même input → no-op idempotent
            // qui doit retourner le même id sans toucher à la ligne.
            let last_scanned_before: Option<String> = sqlx::query_scalar(
                "SELECT last_scanned_at FROM media_item WHERE id = ?",
            )
            .bind(id1.get())
            .fetch_one(&pool)
            .await
            .unwrap();

            let id2 = repo.upsert_media_item(&input).await.unwrap();
            assert_eq!(id1, id2, "id doit être conservé lors d'un re-upsert idempotent");

            let last_scanned_after: Option<String> = sqlx::query_scalar(
                "SELECT last_scanned_at FROM media_item WHERE id = ?",
            )
            .bind(id2.get())
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(
                last_scanned_before, last_scanned_after,
                "last_scanned_at ne doit pas être touché par un upsert idempotent"
            );
        });
    }

    #[test]
    fn upsert_media_idempotent_preserves_user_fields() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            let path = "C:/movies/Inception.2010.1080p.mkv";
            let input = film_input(None, path, 4_500_000_000);
            let id = repo.upsert_media_item(&input).await.unwrap();

            // Édition utilisateur post-insert.
            sqlx::query(
                "UPDATE media_item \
                    SET title = ?, \
                        original_title = 'Inception', \
                        synopsis = 'Un voleur dans les rêves.', \
                        studio = 'Warner Bros', \
                        user_rating = 9, \
                        watch_progress_seconds = 4200, \
                        status = 'en_cours', \
                        favorite = 1, \
                        to_watch = 1 \
                  WHERE id = ?",
            )
            .bind("Inception (Director's Cut)")
            .bind(id.get())
            .execute(&pool)
            .await
            .unwrap();

            // Re-upsert avec mêmes (file_path, size_bytes) → no-op total.
            let id2 = repo.upsert_media_item(&input).await.unwrap();
            assert_eq!(id, id2);

            let row = read_media_row(&pool, id2.get()).await;
            assert_eq!(row.1, "Inception (Director's Cut)", "title préservé");
            assert_eq!(row.8, 4200, "watch_progress_seconds préservé");
            assert_eq!(row.9, "en_cours", "status préservé");
            assert_eq!(row.10, 1, "favorite préservé");
            assert_eq!(row.11, 1, "to_watch préservé");

            // Une seule ligne pour ce file_path.
            let count: i64 = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM media_item WHERE file_path = ?",
            )
            .bind(path)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(count, 1);
        });
    }

    #[test]
    fn upsert_media_refresh_updates_only_technical_fields_on_size_change() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            let path = "C:/movies/Inception.2010.1080p.mkv";
            let id = repo
                .upsert_media_item(&film_input(None, path, 4_500_000_000))
                .await
                .unwrap();

            // Édition manuelle complète des champs utilisateur.
            sqlx::query(
                "UPDATE media_item \
                    SET title = 'Custom Title', \
                        original_title = 'Inception', \
                        year = 2011, \
                        synopsis = 'Note utilisateur', \
                        studio = 'My Studio', \
                        user_rating = 8, \
                        watch_progress_seconds = 1234, \
                        status = 'vu', \
                        favorite = 1, \
                        to_watch = 1 \
                  WHERE id = ?",
            )
            .bind(id.get())
            .execute(&pool)
            .await
            .unwrap();

            // Re-upsert avec une nouvelle taille (re-encodage) + nouveaux
            // champs techniques différents.
            let mut updated = film_input(None, path, 3_900_000_000);
            updated.title = "AUTOPARSED TITLE".to_string(); // doit être ignoré
            updated.year = Some(1999); // doit être ignoré
            updated.duration_seconds = Some(8400);
            updated.video_codec = Some("hevc".to_string());
            updated.width = Some(3840);
            updated.height = Some(2160);
            updated.quality = Some("2160p".to_string()); // doit être ignoré (parsing)

            let id2 = repo.upsert_media_item(&updated).await.unwrap();
            assert_eq!(id, id2, "même fiche logique");

            let row = read_media_row(&pool, id2.get()).await;

            // Champs techniques rafraîchis (liste explicite tâche 7.2).
            assert_eq!(row.0, 3_900_000_000, "size_bytes mis à jour");
            assert_eq!(row.4, Some(8400), "duration_seconds mise à jour");
            assert_eq!(row.5.as_deref(), Some("hevc"), "video_codec mis à jour");
            assert_eq!(row.6, Some(3840), "width mise à jour");
            assert_eq!(row.7, Some(2160), "height mise à jour");
            assert_eq!(row.12, 1, "available remis à 1 sur refresh");

            // Champs **non rafraîchis** au refresh (préservés tels que
            // saisis ou définis à l'insert d'origine — requirement 3.3).
            assert_eq!(row.3.as_deref(), Some("1080p"), "quality d'origine préservée");

            // Champs utilisateur strictement préservés.
            assert_eq!(row.1, "Custom Title", "title utilisateur préservé");
            assert_eq!(row.2, Some(2011), "year utilisateur préservée");
            assert_eq!(row.8, 1234, "watch_progress_seconds préservé");
            assert_eq!(row.9, "vu", "status préservé");
            assert_eq!(row.10, 1, "favorite préservé");
            assert_eq!(row.11, 1, "to_watch préservé");
        });
    }

    #[test]
    fn upsert_media_refresh_restores_available_flag() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            let path = "C:/movies/Movie.mkv";
            let id = repo
                .upsert_media_item(&film_input(None, path, 1_000_000))
                .await
                .unwrap();

            // L'item a été marqué indisponible (ex. retrait du dossier).
            sqlx::query("UPDATE media_item SET available = 0 WHERE id = ?")
                .bind(id.get())
                .execute(&pool)
                .await
                .unwrap();

            // Re-scan : taille différente → refresh, available revient à 1.
            let _ = repo
                .upsert_media_item(&film_input(None, path, 2_000_000))
                .await
                .unwrap();

            let av: i64 =
                sqlx::query_scalar::<_, i64>("SELECT available FROM media_item WHERE id = ?")
                    .bind(id.get())
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(av, 1);
        });
    }

    #[test]
    fn upsert_episode_inserts_then_idempotent_then_refreshes() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            // 1. Créer une série racine via upsert_media_item.
            let series_input = UpsertMediaInput {
                folder_id: None,
                media_type: MediaType::Series,
                file_path: "C:/series/Show".to_string(),
                size_bytes: 0,
                title: "Show".to_string(),
                year: Some(2020),
                quality: None,
                duration_seconds: None,
                video_codec: None,
                width: None,
                height: None,
                numbering_unknown: false,
            };
            let series_id = repo.upsert_media_item(&series_input).await.unwrap();

            // 2. Insert nouvel épisode S01E01.
            let path = "C:/series/Show/S01E01.mkv";
            let ep_in = episode_input(series_id, path, 750_000_000, 1, 1);
            let ep_id = repo.upsert_episode_item(&ep_in).await.unwrap();

            // Vérification champs.
            let (sn, en, sb, dur, codec, w, h, wp, status, av, nu): (
                i64,
                i64,
                Option<i64>,
                Option<i64>,
                Option<String>,
                Option<i64>,
                Option<i64>,
                i64,
                String,
                i64,
                i64,
            ) = sqlx::query_as(
                "SELECT season_number, episode_number, size_bytes, duration_seconds, \
                        video_codec, width, height, watch_progress_seconds, status, \
                        available, numbering_unknown \
                   FROM episode_item WHERE id = ?",
            )
            .bind(ep_id.get())
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(sn, 1);
            assert_eq!(en, 1);
            assert_eq!(sb, Some(750_000_000));
            assert_eq!(dur, Some(2400));
            assert_eq!(codec.as_deref(), Some("h264"));
            assert_eq!(w, Some(1920));
            assert_eq!(h, Some(1080));
            assert_eq!(wp, 0);
            assert_eq!(status, "non_vu");
            assert_eq!(av, 1);
            assert_eq!(nu, 0);

            // 3. Édition utilisateur (progression, statut).
            sqlx::query(
                "UPDATE episode_item SET watch_progress_seconds = 600, status = 'en_cours' \
                  WHERE id = ?",
            )
            .bind(ep_id.get())
            .execute(&pool)
            .await
            .unwrap();

            // 4. Idempotence : même chemin + même taille → pas de modif.
            let ep_id2 = repo.upsert_episode_item(&ep_in).await.unwrap();
            assert_eq!(ep_id, ep_id2);
            let (wp2, status2): (i64, String) = sqlx::query_as(
                "SELECT watch_progress_seconds, status FROM episode_item WHERE id = ?",
            )
            .bind(ep_id2.get())
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(wp2, 600, "progression utilisateur préservée");
            assert_eq!(status2, "en_cours", "status utilisateur préservé");

            // 5. Refresh : même chemin, taille différente → maj
            //    techniques mais conservation des champs utilisateur.
            let mut refreshed = ep_in.clone();
            refreshed.size_bytes = 800_000_000;
            refreshed.duration_seconds = Some(2500);
            refreshed.video_codec = Some("hevc".to_string());
            let ep_id3 = repo.upsert_episode_item(&refreshed).await.unwrap();
            assert_eq!(ep_id, ep_id3);

            let (sb3, codec3, wp3, status3): (Option<i64>, Option<String>, i64, String) =
                sqlx::query_as(
                    "SELECT size_bytes, video_codec, watch_progress_seconds, status \
                       FROM episode_item WHERE id = ?",
                )
                .bind(ep_id3.get())
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(sb3, Some(800_000_000));
            assert_eq!(codec3.as_deref(), Some("hevc"));
            assert_eq!(wp3, 600, "progression utilisateur toujours préservée");
            assert_eq!(status3, "en_cours", "status utilisateur toujours préservé");

            // 6. Une seule ligne pour ce file_path (UNIQUE).
            let count: i64 = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM episode_item WHERE file_path = ?",
            )
            .bind(path)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(count, 1);
        });
    }

    // -----------------------------------------------------------------------
    // Tests tâche 7.3 — set_watch_progress + force_status.
    // -----------------------------------------------------------------------

    /// Crée un film de durée 100 s pour exercer `set_watch_progress`.
    async fn insert_film_with_duration(
        repo: &Repo,
        path: &str,
        duration_seconds: i64,
    ) -> MediaItemId {
        let mut input = film_input(None, path, 1_000_000);
        input.duration_seconds = Some(duration_seconds);
        repo.upsert_media_item(&input).await.unwrap()
    }

    #[test]
    fn set_watch_progress_at_50_percent_yields_en_cours() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            let id = insert_film_with_duration(&repo, "C:/movies/A.mkv", 100).await;
            let status = repo
                .set_watch_progress(ProgressTarget::Media(id), 50)
                .await
                .unwrap();

            assert_eq!(status, crate::domain::status::Status::EnCours);
            let (wp, st): (i64, String) = sqlx::query_as(
                "SELECT watch_progress_seconds, status FROM media_item WHERE id = ?",
            )
            .bind(id.get())
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(wp, 50);
            assert_eq!(st, "en_cours");
        });
    }

    #[test]
    fn set_watch_progress_at_95_percent_yields_vu() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            let id = insert_film_with_duration(&repo, "C:/movies/B.mkv", 100).await;
            let status = repo
                .set_watch_progress(ProgressTarget::Media(id), 95)
                .await
                .unwrap();

            assert_eq!(status, crate::domain::status::Status::Vu);
        });
    }

    #[test]
    fn set_watch_progress_at_zero_yields_non_vu() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            let id = insert_film_with_duration(&repo, "C:/movies/C.mkv", 100).await;
            // Forcer un statut EnCours préalable, puis remettre la
            // progression à 0 doit faire repasser à NonVu.
            repo.force_status(
                ProgressTarget::Media(id),
                crate::domain::status::Status::EnCours,
            )
            .await
            .unwrap();
            let status = repo
                .set_watch_progress(ProgressTarget::Media(id), 0)
                .await
                .unwrap();
            assert_eq!(status, crate::domain::status::Status::NonVu);
        });
    }

    #[test]
    fn set_watch_progress_with_unknown_duration_falls_back_to_non_vu() {
        block_on(async {
            // duration_seconds = NULL en DB → pas de ratio calculable
            // → NonVu indépendamment de la position.
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            let mut input = film_input(None, "C:/movies/D.mkv", 1_000_000);
            input.duration_seconds = None;
            let id = repo.upsert_media_item(&input).await.unwrap();

            let status = repo
                .set_watch_progress(ProgressTarget::Media(id), 999)
                .await
                .unwrap();
            assert_eq!(status, crate::domain::status::Status::NonVu);
        });
    }

    #[test]
    fn force_status_writes_value_directly() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            let id = insert_film_with_duration(&repo, "C:/movies/E.mkv", 100).await;
            // Initialement non_vu.
            let st_before: String =
                sqlx::query_scalar("SELECT status FROM media_item WHERE id = ?")
                    .bind(id.get())
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(st_before, "non_vu");

            repo.force_status(
                ProgressTarget::Media(id),
                crate::domain::status::Status::Vu,
            )
            .await
            .unwrap();

            let st_after: String =
                sqlx::query_scalar("SELECT status FROM media_item WHERE id = ?")
                    .bind(id.get())
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(st_after, "vu");
        });
    }

    #[test]
    fn force_status_then_set_watch_progress_overwrites_manual_value() {
        block_on(async {
            // Confirme la note de design : l'override n'est PAS
            // persisté ; un set_watch_progress ultérieur recalcule
            // depuis la progression et écrase la valeur forcée.
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            let id = insert_film_with_duration(&repo, "C:/movies/F.mkv", 100).await;

            // Force Vu manuellement.
            repo.force_status(
                ProgressTarget::Media(id),
                crate::domain::status::Status::Vu,
            )
            .await
            .unwrap();

            // Nouvelle progression à 30 % → recalcul donne EnCours.
            let status = repo
                .set_watch_progress(ProgressTarget::Media(id), 30)
                .await
                .unwrap();
            assert_eq!(status, crate::domain::status::Status::EnCours);
        });
    }

    #[test]
    fn set_watch_progress_returns_validation_error_for_missing_target() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool);

            let err = repo
                .set_watch_progress(ProgressTarget::Media(MediaItemId(99_999)), 42)
                .await
                .expect_err("cible inexistante doit être rejetée");
            assert!(matches!(err, KyroError::Validation(_)));
        });
    }

    // -----------------------------------------------------------------------
    // Tests tâche 7.4 — listings et requêtes secondaires.
    //
    // Ces tests couvrent les requirements 7.2, 7.3, 7.4, 8.5, 11.4, 11.5,
    // 12.1. Ils exercent les méthodes `list_by_category`, `search`,
    // `list_favorites`, `list_to_watch`, `list_episodes`,
    // `count_episodes_by_status` et `next_resume_episode` sur des
    // données fixtures contrôlées.
    // -----------------------------------------------------------------------

    /// Bascule manuellement le drapeau `favorite` ou `to_watch` d'un
    /// `media_item` pour les tests (anticipe la tâche 7.6 qui exposera
    /// les vraies méthodes `toggle_favorite` / `toggle_to_watch`).
    async fn set_flag(pool: &SqlitePool, id: i64, column: &str, value: i64) {
        let sql = format!("UPDATE media_item SET {column} = ? WHERE id = ?");
        sqlx::query(&sql)
            .bind(value)
            .bind(id)
            .execute(pool)
            .await
            .expect("update flag");
    }

    /// Insère une série racine via `upsert_media_item` et attache des
    /// épisodes via `upsert_episode_item`. Retourne `(series_id,
    /// vec<episode_id>)`.
    async fn make_series_with_episodes(
        repo: &Repo,
        series_path: &str,
        episodes: &[(i64, i64, &str, i64)], // (season, episode, file_path, size)
    ) -> (MediaItemId, Vec<EpisodeItemId>) {
        let series_input = UpsertMediaInput {
            folder_id: None,
            media_type: MediaType::Series,
            file_path: series_path.to_string(),
            size_bytes: 0,
            title: "Series".to_string(),
            year: None,
            quality: None,
            duration_seconds: None,
            video_codec: None,
            width: None,
            height: None,
            numbering_unknown: false,
        };
        let series_id = repo.upsert_media_item(&series_input).await.unwrap();

        let mut ids = Vec::with_capacity(episodes.len());
        for (season, episode, path, size) in episodes {
            let ep = episode_input(series_id, path, *size, *season, *episode);
            ids.push(repo.upsert_episode_item(&ep).await.unwrap());
        }
        (series_id, ids)
    }

    #[test]
    fn list_by_category_returns_only_matching_type() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            // Insérer un Film, une Série racine et un Anime racine.
            let film = repo
                .upsert_media_item(&film_input(None, "C:/movies/F.mkv", 1_000))
                .await
                .unwrap();

            let series = UpsertMediaInput {
                folder_id: None,
                media_type: MediaType::Series,
                file_path: "C:/series/S".to_string(),
                size_bytes: 0,
                title: "S".to_string(),
                year: None,
                quality: None,
                duration_seconds: None,
                video_codec: None,
                width: None,
                height: None,
                numbering_unknown: false,
            };
            let series_id = repo.upsert_media_item(&series).await.unwrap();

            let anime = UpsertMediaInput {
                folder_id: None,
                media_type: MediaType::Anime,
                file_path: "C:/animes/A".to_string(),
                size_bytes: 0,
                title: "A".to_string(),
                year: None,
                quality: None,
                duration_seconds: None,
                video_codec: None,
                width: None,
                height: None,
                numbering_unknown: false,
            };
            let anime_id = repo.upsert_media_item(&anime).await.unwrap();

            let films = repo.list_by_category(Category::Films).await.unwrap();
            assert_eq!(films.len(), 1);
            assert_eq!(films[0].id, film);
            assert_eq!(films[0].media_type, MediaType::Film);

            let series_list = repo.list_by_category(Category::Series).await.unwrap();
            assert_eq!(series_list.len(), 1);
            assert_eq!(series_list[0].id, series_id);

            let animes = repo.list_by_category(Category::Animes).await.unwrap();
            assert_eq!(animes.len(), 1);
            assert_eq!(animes[0].id, anime_id);
        });
    }

    #[test]
    fn list_by_category_empty_when_no_matching_items() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool);

            let result = repo.list_by_category(Category::Films).await.unwrap();
            assert!(result.is_empty());
        });
    }

    #[test]
    fn list_by_category_attaches_genres_in_batch() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            let id = repo
                .upsert_media_item(&film_input(None, "C:/movies/X.mkv", 1_000))
                .await
                .unwrap();

            // Insérer 2 genres et les rattacher au film.
            sqlx::query("INSERT INTO genre (name) VALUES ('Action'), ('Sci-Fi')")
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query(
                "INSERT INTO media_genre (media_item_id, genre_id) \
                 SELECT ?, id FROM genre",
            )
            .bind(id.get())
            .execute(&pool)
            .await
            .unwrap();

            let films = repo.list_by_category(Category::Films).await.unwrap();
            assert_eq!(films.len(), 1);
            // L'ordre est `name ASC` (cf. ORDER BY mg.media_item_id, g.name).
            assert_eq!(films[0].genres, vec!["Action", "Sci-Fi"]);
        });
    }

    #[test]
    fn search_filters_by_title_case_and_accent_insensitive() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            // Insérer 3 films avec des titres incluant des accents.
            let mut a = film_input(None, "C:/movies/Amelie.mkv", 1_000);
            a.title = "Amélie".to_string();
            let a_id = repo.upsert_media_item(&a).await.unwrap();

            let mut b = film_input(None, "C:/movies/Matrix.mkv", 1_000);
            b.title = "The Matrix".to_string();
            let _b_id = repo.upsert_media_item(&b).await.unwrap();

            let mut c = film_input(None, "C:/movies/Cafe.mkv", 1_000);
            c.title = "Café Society".to_string();
            let c_id = repo.upsert_media_item(&c).await.unwrap();

            // Recherche sans accent doit trouver "Amélie" et "Café".
            let result = repo.search("ame").await.unwrap();
            let ids: Vec<_> = result.iter().map(|m| m.id).collect();
            assert!(ids.contains(&a_id), "doit trouver Amélie via 'ame'");

            let result = repo.search("CAFE").await.unwrap();
            let ids: Vec<_> = result.iter().map(|m| m.id).collect();
            assert!(ids.contains(&c_id), "doit trouver Café via 'CAFE'");

            // Requête vide retourne tout.
            let all = repo.search("").await.unwrap();
            assert_eq!(all.len(), 3);
        });
    }

    /// Test FTS5 avec multi-tokens (AND implicite). « inception 2010 »
    /// doit matcher uniquement les fiches qui contiennent les deux
    /// tokens, pas l'un OU l'autre.
    #[test]
    fn search_multi_token_uses_and_semantics() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            let mut a = film_input(None, "C:/movies/i1.mkv", 1_000);
            a.title = "Inception 2010".to_string();
            let a_id = repo.upsert_media_item(&a).await.unwrap();

            let mut b = film_input(None, "C:/movies/i2.mkv", 1_000);
            b.title = "Inception 2024".to_string();
            let _b_id = repo.upsert_media_item(&b).await.unwrap();

            let mut c = film_input(None, "C:/movies/m.mkv", 1_000);
            c.title = "Other 2010".to_string();
            let _c_id = repo.upsert_media_item(&c).await.unwrap();

            let result = repo.search("inception 2010").await.unwrap();
            let ids: Vec<_> = result.iter().map(|m| m.id).collect();
            // Seul a contient les deux tokens.
            assert_eq!(ids, vec![a_id]);
        });
    }

    /// L'index FTS doit être mis à jour quand un media_item est
    /// supprimé (CASCADE via remove_library_folder par exemple) ou
    /// quand son titre change.
    #[test]
    fn search_reflects_title_updates() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            let mut a = film_input(None, "C:/movies/x.mkv", 1_000);
            a.title = "AncienTitre".to_string();
            let a_id = repo.upsert_media_item(&a).await.unwrap();

            // Recherche par ancien titre fonctionne.
            let r = repo.search("ancien").await.unwrap();
            assert_eq!(r.len(), 1);

            // Renomme via UPDATE direct.
            sqlx::query("UPDATE media_item SET title = ? WHERE id = ?")
                .bind("NouveauNom")
                .bind(a_id.get())
                .execute(repo.pool())
                .await
                .unwrap();

            // L'ancien titre ne match plus, le nouveau oui.
            let r = repo.search("ancien").await.unwrap();
            assert!(r.is_empty(), "FTS doit être à jour après UPDATE");
            let r = repo.search("nouv").await.unwrap();
            assert_eq!(r.len(), 1);
        });
    }

    /// Le helper `build_fts_match_query` doit produire des requêtes
    /// FTS5 sûres pour différents inputs utilisateurs.
    #[test]
    fn build_fts_match_query_handles_special_characters() {
        // Cas nominal : un seul token devient `"foo"*`.
        assert_eq!(build_fts_match_query("foo"), r#""foo"*"#);

        // Multi-tokens joints par espace (AND implicite côté FTS5).
        assert_eq!(
            build_fts_match_query("hello world"),
            r#""hello"* "world"*"#
        );

        // Caractères réservés FTS5 supprimés.
        assert_eq!(build_fts_match_query("foo* (bar)"), r#""foo"* "bar"*"#);

        // Guillemets doubles échappés.
        assert_eq!(
            build_fts_match_query(r#"a"b"#),
            r#""a""b"*"#,
            "les guillemets internes doivent être doublés"
        );

        // Whitespace seul → résultat vide (caller doit court-circuiter).
        assert_eq!(build_fts_match_query("   "), "");
        assert_eq!(build_fts_match_query(""), "");

        // Que des caractères réservés → résultat vide.
        assert_eq!(build_fts_match_query("***"), "");
    }

    #[test]
    fn list_favorites_returns_only_flagged_items() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            let f1 = repo
                .upsert_media_item(&film_input(None, "C:/movies/F1.mkv", 1_000))
                .await
                .unwrap();
            let _f2 = repo
                .upsert_media_item(&film_input(None, "C:/movies/F2.mkv", 2_000))
                .await
                .unwrap();
            let f3 = repo
                .upsert_media_item(&film_input(None, "C:/movies/F3.mkv", 3_000))
                .await
                .unwrap();

            // Marquer f1 et f3 comme favoris.
            set_flag(&pool, f1.get(), "favorite", 1).await;
            set_flag(&pool, f3.get(), "favorite", 1).await;

            let favs = repo.list_favorites().await.unwrap();
            let ids: Vec<_> = favs.iter().map(|m| m.id).collect();
            assert_eq!(ids, vec![f1, f3]);
        });
    }

    #[test]
    fn list_to_watch_returns_only_flagged_items() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            let _f1 = repo
                .upsert_media_item(&film_input(None, "C:/movies/F1.mkv", 1_000))
                .await
                .unwrap();
            let f2 = repo
                .upsert_media_item(&film_input(None, "C:/movies/F2.mkv", 2_000))
                .await
                .unwrap();
            set_flag(&pool, f2.get(), "to_watch", 1).await;

            let to_watch = repo.list_to_watch().await.unwrap();
            assert_eq!(to_watch.len(), 1);
            assert_eq!(to_watch[0].id, f2);
        });
    }

    #[test]
    fn list_episodes_sorted_by_season_then_episode() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            // Insertion volontairement désordonnée : S02E01, S01E02,
            // S01E10, S01E01. L'ordre attendu : S01E01, S01E02, S01E10,
            // S02E01.
            let (_series_id, _ep_ids) = make_series_with_episodes(
                &repo,
                "C:/series/Show",
                &[
                    (2, 1, "C:/series/Show/S02E01.mkv", 100),
                    (1, 2, "C:/series/Show/S01E02.mkv", 200),
                    (1, 10, "C:/series/Show/S01E10.mkv", 300),
                    (1, 1, "C:/series/Show/S01E01.mkv", 400),
                ],
            )
            .await;

            // Récupérer l'id de la série pour interroger.
            let series_id_int: i64 =
                sqlx::query_scalar("SELECT id FROM media_item WHERE type = 'Series'")
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            let episodes = repo
                .list_episodes(MediaItemId(series_id_int))
                .await
                .unwrap();

            let ordered: Vec<(i64, i64)> = episodes
                .iter()
                .map(|e| (e.season_number, e.episode_number))
                .collect();
            assert_eq!(ordered, vec![(1, 1), (1, 2), (1, 10), (2, 1)]);
        });
    }

    #[test]
    fn count_episodes_by_status_aggregates_correctly() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            let (series_id, ep_ids) = make_series_with_episodes(
                &repo,
                "C:/series/Show",
                &[
                    (1, 1, "C:/series/Show/E1.mkv", 100),
                    (1, 2, "C:/series/Show/E2.mkv", 200),
                    (1, 3, "C:/series/Show/E3.mkv", 300),
                    (1, 4, "C:/series/Show/E4.mkv", 400),
                ],
            )
            .await;

            // 2 vus, 1 en_cours, 1 non_vu.
            sqlx::query("UPDATE episode_item SET status = 'vu' WHERE id = ?")
                .bind(ep_ids[0].get())
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("UPDATE episode_item SET status = 'vu' WHERE id = ?")
                .bind(ep_ids[1].get())
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("UPDATE episode_item SET status = 'en_cours' WHERE id = ?")
                .bind(ep_ids[2].get())
                .execute(&pool)
                .await
                .unwrap();
            // ep_ids[3] reste 'non_vu' par défaut.

            let counts = repo.count_episodes_by_status(series_id).await.unwrap();
            assert_eq!(counts.get(&Status::Vu).copied(), Some(2));
            assert_eq!(counts.get(&Status::EnCours).copied(), Some(1));
            assert_eq!(counts.get(&Status::NonVu).copied(), Some(1));
            // Total = 4.
            let total: i64 = counts.values().sum();
            assert_eq!(total, 4);
        });
    }

    #[test]
    fn count_episodes_by_status_always_includes_all_three_keys() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool);

            // Série sans épisode → toutes les clés à 0.
            let (series_id, _) =
                make_series_with_episodes(&repo, "C:/series/Empty", &[]).await;
            let counts = repo.count_episodes_by_status(series_id).await.unwrap();
            assert_eq!(counts.get(&Status::NonVu).copied(), Some(0));
            assert_eq!(counts.get(&Status::EnCours).copied(), Some(0));
            assert_eq!(counts.get(&Status::Vu).copied(), Some(0));
        });
    }

    #[test]
    fn next_resume_episode_returns_first_in_progress() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            let (series_id, ep_ids) = make_series_with_episodes(
                &repo,
                "C:/series/Show",
                &[
                    (1, 1, "C:/series/Show/E1.mkv", 100),
                    (1, 2, "C:/series/Show/E2.mkv", 200),
                    (1, 3, "C:/series/Show/E3.mkv", 300),
                ],
            )
            .await;

            // E1 vu, E2 en_cours, E3 non_vu : reprise sur E2.
            sqlx::query("UPDATE episode_item SET status = 'vu' WHERE id = ?")
                .bind(ep_ids[0].get())
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("UPDATE episode_item SET status = 'en_cours' WHERE id = ?")
                .bind(ep_ids[1].get())
                .execute(&pool)
                .await
                .unwrap();

            let next = repo.next_resume_episode(series_id).await.unwrap();
            assert_eq!(next, Some(ep_ids[1]));
        });
    }

    #[test]
    fn next_resume_episode_falls_back_to_first_unseen() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            let (series_id, ep_ids) = make_series_with_episodes(
                &repo,
                "C:/series/Show",
                &[
                    (1, 1, "C:/series/Show/E1.mkv", 100),
                    (1, 2, "C:/series/Show/E2.mkv", 200),
                    (1, 3, "C:/series/Show/E3.mkv", 300),
                ],
            )
            .await;

            // E1 vu, pas d'en_cours → premier non_vu = E2.
            sqlx::query("UPDATE episode_item SET status = 'vu' WHERE id = ?")
                .bind(ep_ids[0].get())
                .execute(&pool)
                .await
                .unwrap();

            let next = repo.next_resume_episode(series_id).await.unwrap();
            assert_eq!(next, Some(ep_ids[1]));
        });
    }

    #[test]
    fn next_resume_episode_returns_first_when_all_seen() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            let (series_id, ep_ids) = make_series_with_episodes(
                &repo,
                "C:/series/Show",
                &[
                    (1, 1, "C:/series/Show/E1.mkv", 100),
                    (1, 2, "C:/series/Show/E2.mkv", 200),
                ],
            )
            .await;

            sqlx::query("UPDATE episode_item SET status = 'vu'")
                .execute(&pool)
                .await
                .unwrap();

            // Tous vus → RewatchFromStart pointe sur E1 (id du premier
            // selon l'ordre saison/épisode).
            let next = repo.next_resume_episode(series_id).await.unwrap();
            assert_eq!(next, Some(ep_ids[0]));
        });
    }

    #[test]
    fn next_resume_episode_ignores_unavailable_and_returns_none_if_empty() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            let (series_id, ep_ids) = make_series_with_episodes(
                &repo,
                "C:/series/Show",
                &[
                    (1, 1, "C:/series/Show/E1.mkv", 100),
                    (1, 2, "C:/series/Show/E2.mkv", 200),
                ],
            )
            .await;

            // Marquer tous les épisodes indisponibles.
            sqlx::query("UPDATE episode_item SET available = 0")
                .execute(&pool)
                .await
                .unwrap();

            let next = repo.next_resume_episode(series_id).await.unwrap();
            assert!(next.is_none());

            // Sanity : on touche bien aux 2 épisodes attendus.
            assert_eq!(ep_ids.len(), 2);
        });
    }

    // -----------------------------------------------------------------------
    // Tâche 7.5 — settings key/value (requirements 2.1, 7.1, 10.1, 10.2,
    // 16.1, 16.14).
    // -----------------------------------------------------------------------

    use crate::infra::settings_keys::{
        RESERVED_KEYS, SETTING_KEY_EXTERNAL_PLAYER_PATH, SETTING_KEY_FFPROBE_PATH,
        SETTING_KEY_LAST_CATEGORY, SETTING_KEY_SCAN_AT_STARTUP, SETTING_KEY_THEME,
    };

    #[test]
    fn get_setting_returns_none_when_absent() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool);

            // Clé réservée mais jamais écrite.
            let v = repo.get_setting(SETTING_KEY_THEME).await.unwrap();
            assert!(v.is_none());

            // Clé hors allowlist : `get_setting` ne valide pas, retourne
            // simplement `None`.
            let v = repo.get_setting("inconnu").await.unwrap();
            assert!(v.is_none());
        });
    }

    #[test]
    fn upsert_setting_then_get_returns_value() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool);

            repo.upsert_setting(SETTING_KEY_THEME, "adwaita-dark")
                .await
                .expect("upsert d'une clé réservée doit réussir");

            let v = repo.get_setting(SETTING_KEY_THEME).await.unwrap();
            assert_eq!(v.as_deref(), Some("adwaita-dark"));
        });
    }

    #[test]
    fn upsert_setting_replaces_existing_value() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            repo.upsert_setting(SETTING_KEY_LAST_CATEGORY, "Films")
                .await
                .unwrap();
            repo.upsert_setting(SETTING_KEY_LAST_CATEGORY, "Animes")
                .await
                .unwrap();

            // Même clé → une seule ligne en base, valeur mise à jour.
            let count: i64 = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM setting_value WHERE key = ?",
            )
            .bind(SETTING_KEY_LAST_CATEGORY)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(count, 1);

            let v = repo.get_setting(SETTING_KEY_LAST_CATEGORY).await.unwrap();
            assert_eq!(v.as_deref(), Some("Animes"));
        });
    }

    #[test]
    fn upsert_setting_accepts_each_reserved_key() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool);

            // Chaque clé réservée doit être acceptée et relue à
            // l'identique. On utilise une valeur différente par clé
            // pour vérifier l'absence de collision.
            let cases = [
                (SETTING_KEY_EXTERNAL_PLAYER_PATH, "C:\\Program Files\\mpv.net\\mpvnet.exe"),
                (SETTING_KEY_FFPROBE_PATH, "C:\\tools\\ffprobe.exe"),
                (SETTING_KEY_THEME, "adwaita-light"),
                (SETTING_KEY_SCAN_AT_STARTUP, "true"),
                (SETTING_KEY_LAST_CATEGORY, "Series"),
            ];
            for (k, v) in cases {
                repo.upsert_setting(k, v)
                    .await
                    .unwrap_or_else(|e| panic!("upsert {k:?} doit réussir, got {e:?}"));
            }

            // Sanity : on retombe sur exactement 5 lignes (= cardinal
            // de l'allowlist).
            for (k, v) in cases {
                let read = repo.get_setting(k).await.unwrap();
                assert_eq!(read.as_deref(), Some(v));
            }
            assert_eq!(RESERVED_KEYS.len(), 5);
        });
    }

    #[test]
    fn upsert_setting_rejects_unknown_key_with_validation() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            let err = repo
                .upsert_setting("scan_au_demarrage", "true")
                .await
                .expect_err("clé hors allowlist doit être rejetée");
            match err {
                KyroError::Validation(msg) => {
                    assert!(
                        msg.contains("scan_au_demarrage"),
                        "le message doit citer la clé refusée (got: {msg:?})"
                    );
                }
                other => panic!("expected Validation, got {other:?}"),
            }

            // Aucune ligne n'a été insérée par l'appel refusé.
            let count: i64 =
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM setting_value")
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(count, 0);
        });
    }

    #[test]
    fn upsert_setting_rejects_case_variant() {
        // Comparaison sensible à la casse : `Theme` n'est pas réservé.
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool);

            let err = repo
                .upsert_setting("Theme", "adwaita-dark")
                .await
                .expect_err("la casse doit être stricte");
            assert!(matches!(err, KyroError::Validation(_)));
        });
    }

    #[test]
    fn upsert_setting_preserves_value_verbatim() {
        // La valeur n'est ni trimée ni normalisée — chaque sous-système
        // gère son format. On vérifie qu'une valeur avec espaces
        // périphériques et caractère non ASCII est stockée à l'identique.
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool);

            let raw = "  C:\\Films préférés\\mpv.net.exe  ";
            repo.upsert_setting(SETTING_KEY_EXTERNAL_PLAYER_PATH, raw)
                .await
                .unwrap();

            let read = repo
                .get_setting(SETTING_KEY_EXTERNAL_PLAYER_PATH)
                .await
                .unwrap();
            assert_eq!(read.as_deref(), Some(raw));
        });
    }

    // -----------------------------------------------------------------------
    // Tests tâche 7.6 — toggle_favorite / toggle_to_watch (requirements
    // 11.1, 11.2, 11.3).
    //
    // L'invariant central exigé par la tâche est l'**involution** :
    // deux appels successifs doivent ramener exactement à l'état
    // initial. Les tests ci-dessous couvrent :
    //
    //   * Le 1er toggle change l'état attendu (0 → 1 et 1 → 0) et la
    //     valeur retournée correspond bien à la nouvelle valeur en
    //     base.
    //   * Le 2nd toggle restaure l'état initial (involution).
    //   * Une valeur initiale non triviale (déjà à 1) est correctement
    //     basculée puis restaurée.
    //   * `favorite` et `to_watch` sont strictement indépendants — un
    //     toggle de l'un ne touche pas l'autre.
    //   * Identifiant inconnu → `KyroError::Validation` sans modifier
    //     la base.
    //
    // Les tests de propriété complets (P-25) seront couverts par la
    // tâche 7.16 ; ici on vérifie l'involution sur des cas concrets.
    // -----------------------------------------------------------------------

    /// Lit le couple `(favorite, to_watch)` d'un media_item pour les
    /// assertions ci-dessous.
    async fn read_flags(pool: &SqlitePool, id: i64) -> (i64, i64) {
        sqlx::query_as::<_, (i64, i64)>(
            "SELECT favorite, to_watch FROM media_item WHERE id = ?",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("read_flags")
    }

    #[test]
    fn toggle_favorite_flips_zero_to_one_and_back() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            let id = repo
                .upsert_media_item(&film_input(None, "C:/movies/A.mkv", 1_000))
                .await
                .unwrap();

            // État initial garanti par défaut : favorite = 0.
            assert_eq!(read_flags(&pool, id.get()).await, (0, 0));

            // 1er toggle : 0 → 1.
            let after1 = repo.toggle_favorite(id).await.unwrap();
            assert!(after1, "toggle_favorite doit retourner true après 0 → 1");
            assert_eq!(read_flags(&pool, id.get()).await, (1, 0));

            // 2nd toggle : 1 → 0 (involution).
            let after2 = repo.toggle_favorite(id).await.unwrap();
            assert!(!after2, "toggle_favorite doit retourner false après 1 → 0");
            assert_eq!(
                read_flags(&pool, id.get()).await,
                (0, 0),
                "deux toggles successifs doivent ramener à l'état initial"
            );
        });
    }

    #[test]
    fn toggle_to_watch_flips_zero_to_one_and_back() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            let id = repo
                .upsert_media_item(&film_input(None, "C:/movies/B.mkv", 1_000))
                .await
                .unwrap();

            assert_eq!(read_flags(&pool, id.get()).await, (0, 0));

            let after1 = repo.toggle_to_watch(id).await.unwrap();
            assert!(after1);
            assert_eq!(read_flags(&pool, id.get()).await, (0, 1));

            let after2 = repo.toggle_to_watch(id).await.unwrap();
            assert!(!after2);
            assert_eq!(
                read_flags(&pool, id.get()).await,
                (0, 0),
                "involution sur to_watch"
            );
        });
    }

    #[test]
    fn toggle_favorite_starting_from_one_is_also_involutive() {
        // Si la fiche est déjà favori (cas d'une bibliothèque
        // pré-existante ou d'une migration), le 1er toggle doit la
        // démarquer puis le 2nd doit la remettre — l'involution ne
        // dépend pas de l'état initial.
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            let id = repo
                .upsert_media_item(&film_input(None, "C:/movies/C.mkv", 1_000))
                .await
                .unwrap();
            // Pré-positionner favorite = 1.
            sqlx::query("UPDATE media_item SET favorite = 1 WHERE id = ?")
                .bind(id.get())
                .execute(&pool)
                .await
                .unwrap();
            assert_eq!(read_flags(&pool, id.get()).await, (1, 0));

            let after1 = repo.toggle_favorite(id).await.unwrap();
            assert!(!after1);
            assert_eq!(read_flags(&pool, id.get()).await, (0, 0));

            let after2 = repo.toggle_favorite(id).await.unwrap();
            assert!(after2);
            assert_eq!(read_flags(&pool, id.get()).await, (1, 0));
        });
    }

    #[test]
    fn toggle_favorite_does_not_affect_to_watch_and_vice_versa() {
        // Indépendance stricte des deux drapeaux (cf. requirement 11.1
        // qui les déclare comme deux booléens distincts sur Media_Item).
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            let id = repo
                .upsert_media_item(&film_input(None, "C:/movies/D.mkv", 1_000))
                .await
                .unwrap();

            repo.toggle_favorite(id).await.unwrap();
            assert_eq!(
                read_flags(&pool, id.get()).await,
                (1, 0),
                "toggle_favorite ne doit pas toucher to_watch"
            );

            repo.toggle_to_watch(id).await.unwrap();
            assert_eq!(
                read_flags(&pool, id.get()).await,
                (1, 1),
                "toggle_to_watch ne doit pas toucher favorite"
            );

            // Deux toggles successifs sur to_watch ramènent à (1, 1).
            repo.toggle_to_watch(id).await.unwrap();
            repo.toggle_to_watch(id).await.unwrap();
            assert_eq!(read_flags(&pool, id.get()).await, (1, 1));
        });
    }

    #[test]
    fn toggle_favorite_unknown_id_returns_validation_error() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());

            let err = repo
                .toggle_favorite(MediaItemId(99_999))
                .await
                .expect_err("id inconnu doit être rejeté");
            match err {
                KyroError::Validation(msg) => {
                    assert!(
                        msg.contains("99999"),
                        "le message doit citer l'id refusé (got: {msg:?})"
                    );
                }
                other => panic!("expected Validation, got {other:?}"),
            }

            // Aucune ligne n'a été créée par l'appel raté.
            let count: i64 =
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM media_item")
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(count, 0);
        });
    }

    #[test]
    fn toggle_to_watch_unknown_id_returns_validation_error() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool);

            let err = repo
                .toggle_to_watch(MediaItemId(42))
                .await
                .expect_err("id inconnu doit être rejeté");
            assert!(matches!(err, KyroError::Validation(_)));
        });
    }

    // -----------------------------------------------------------------------
    // Tâche 7.16 — Test de propriété P-25 : involutions des toggles.
    //
    // **Property 25 : Toggle favori et toggle à_regarder sont des
    // involutions.**
    //
    // Pour tout Media_Item d'état initial `(favorite, to_watch)`
    // arbitraire, deux appels successifs de `toggle_favorite` doivent
    // ramener exactement à l'état initial (idem pour `toggle_to_watch`).
    // Autrement dit : `f ∘ f = id` sur `{0, 1}`.
    //
    // Stratégie :
    //   * Générateur sur le couple initial `(fav, tw) ∈ {0,1}²` ainsi
    //     que sur quelques champs « techniques » (taille, durée) afin
    //     d'éviter qu'une éventuelle dépendance involontaire à ces
    //     champs ne masque la propriété.
    //   * Pour chaque cas : insertion d'un Media_Item via
    //     `upsert_media_item`, puis SQL UPDATE pour pré-positionner
    //     `(favorite, to_watch)` à la valeur générée (les valeurs par
    //     défaut sont `(0, 0)` ; pour exercer le départ depuis `1`
    //     on applique un UPDATE direct).
    //   * Lecture `(fav0, tw0)`, deux toggles successifs sur la même
    //     colonne, lecture finale `(fav2, tw2)`, on vérifie l'égalité.
    //   * Vérification croisée : la colonne **non** togglée n'est pas
    //     modifiée (la propriété P-25 porte uniquement sur l'involution,
    //     mais l'absence d'effet de bord rend la propriété parfaitement
    //     indépendante par axe — utile pour le shrinking).
    //
    // Tag : `Feature: kyro, Property 25`. ≥ 100 cas (cf. `ProptestConfig`).
    //
    // **Validates: Requirements 11.2, 11.3**
    // -----------------------------------------------------------------------

    /// Construit un `Media_Item` minimal et pré-positionne `(favorite,
    /// to_watch)` à la valeur fournie. Retourne l'`id` du `media_item`
    /// inséré.
    async fn insert_media_with_flags(
        pool: &SqlitePool,
        repo: &Repo,
        file_path: &str,
        size: i64,
        favorite: i64,
        to_watch: i64,
    ) -> MediaItemId {
        let id = repo
            .upsert_media_item(&film_input(None, file_path, size))
            .await
            .expect("insertion media_item");

        // Pré-positionnement explicite — par défaut le DDL force
        // `(0, 0)`, donc cet UPDATE est un no-op pour le cas trivial,
        // mais il garantit une couverture homogène quand le générateur
        // tire des valeurs initiales différentes de zéro.
        sqlx::query("UPDATE media_item SET favorite = ?, to_watch = ? WHERE id = ?")
            .bind(favorite)
            .bind(to_watch)
            .bind(id.get())
            .execute(pool)
            .await
            .expect("pré-positionnement (favorite, to_watch)");

        id
    }

    proptest! {
        // 100 cas par défaut suffisent : l'espace d'état pertinent
        // (`favorite × to_watch`) ne contient que 4 combinaisons, mais
        // l'enrichissement par `size_bytes` et le path explore des
        // entrées DB distinctes sans collision sur `file_path`.
        #![proptest_config(ProptestConfig {
            cases: 100,
            ..ProptestConfig::default()
        })]

        /// **Property 25 (P-25) — `toggle_favorite` est une involution.**
        ///
        /// Pour tout Media_Item d'état initial `fav0 ∈ {0, 1}`, deux
        /// `toggle_favorite` successifs ramènent à `fav0`. La colonne
        /// `to_watch` est laissée intacte (vérification d'absence
        /// d'effet de bord, garante de l'orthogonalité des deux
        /// drapeaux exigée par le requirement 11.1).
        ///
        /// Tag: Feature: kyro, Property 25.
        ///
        /// **Validates: Requirements 11.2, 11.3**
        #[test]
        fn p25_toggle_favorite_is_involution(
            fav0 in 0i64..=1,
            tw0 in 0i64..=1,
            size in 1i64..=1_000_000_000,
            seed in 0u32..1_000_000,
        ) {
            block_on(async move {
                let db_dir = TempDir::new().unwrap();
                let pool = fresh_pool(db_dir.path()).await;
                let repo = Repo::new(pool.clone());

                // `seed` rend chaque `file_path` unique entre cas pour
                // éviter toute interaction inter-cas (chaque cas ouvre
                // pourtant son propre `TempDir`, mais la précaution est
                // gratuite et clarifie l'intention).
                let path = format!("C:/movies/p25_fav_{seed}.mkv");
                let id = insert_media_with_flags(&pool, &repo, &path, size, fav0, tw0).await;

                // Sanity check du pré-positionnement.
                let before = read_flags(&pool, id.get()).await;
                prop_assert_eq!(
                    before,
                    (fav0, tw0),
                    "pré-positionnement initial doit refléter le générateur"
                );

                // Deux toggles successifs sur `favorite`.
                repo.toggle_favorite(id).await.unwrap();
                repo.toggle_favorite(id).await.unwrap();

                let after = read_flags(&pool, id.get()).await;
                prop_assert_eq!(
                    after,
                    before,
                    "P-25 (favorite) : deux toggles successifs doivent être l'identité"
                );
                Ok(())
            })?;
        }

        /// **Property 25 (P-25, suite) — `toggle_to_watch` est une
        /// involution.**
        ///
        /// Symétrique de la propriété ci-dessus : deux toggles
        /// successifs sur `to_watch` ramènent à `tw0`. La colonne
        /// `favorite` est laissée intacte.
        ///
        /// Tag: Feature: kyro, Property 25.
        ///
        /// **Validates: Requirements 11.2, 11.3**
        #[test]
        fn p25_toggle_to_watch_is_involution(
            fav0 in 0i64..=1,
            tw0 in 0i64..=1,
            size in 1i64..=1_000_000_000,
            seed in 0u32..1_000_000,
        ) {
            block_on(async move {
                let db_dir = TempDir::new().unwrap();
                let pool = fresh_pool(db_dir.path()).await;
                let repo = Repo::new(pool.clone());

                let path = format!("C:/movies/p25_tw_{seed}.mkv");
                let id = insert_media_with_flags(&pool, &repo, &path, size, fav0, tw0).await;

                let before = read_flags(&pool, id.get()).await;
                prop_assert_eq!(
                    before,
                    (fav0, tw0),
                    "pré-positionnement initial doit refléter le générateur"
                );

                repo.toggle_to_watch(id).await.unwrap();
                repo.toggle_to_watch(id).await.unwrap();

                let after = read_flags(&pool, id.get()).await;
                prop_assert_eq!(
                    after,
                    before,
                    "P-25 (to_watch) : deux toggles successifs doivent être l'identité"
                );
                Ok(())
            })?;
        }
    }

    // -----------------------------------------------------------------------
    // Tâche 7.17 — Test de propriété P-26 : vues `list_favorites()` et
    // `list_to_watch()`.
    //
    // **Property 26 : Vue favoris et vue à_regarder.**
    //
    // Feature: kyro, Property 26.
    //
    // **Validates: Requirements 11.4, 11.5**
    //
    // Pour toute bibliothèque arbitraire (0..=50 `Media_Item`) avec
    // drapeaux `(favorite, to_watch) ∈ {0,1}²` tirés indépendamment
    // par item :
    //
    //   * `list_favorites()` retourne **exactement** le sous-ensemble
    //     des items dont `favorite = 1` ;
    //   * `list_to_watch()` retourne **exactement** le sous-ensemble
    //     des items dont `to_watch = 1`.
    //
    // Stratégie :
    //   * DB SQLite fraîche par cas (`TempDir` + `fresh_pool`),
    //     migrations appliquées par `fresh_pool`.
    //   * Insertion via `upsert_media_item` (le DDL initialise
    //     `favorite = 0, to_watch = 0`), puis `UPDATE` explicite pour
    //     positionner les drapeaux selon le générateur — même schéma
    //     que P-25 (`insert_media_with_flags`).
    //   * `file_path` dérivé de `(seed, index)` pour éviter toute
    //     collision sur la clé d'unicité applicative
    //     `media_item.file_path` (un upsert sur un même chemin
    //     produirait un no-op idempotent qui fausserait le décompte).
    //   * Projection des ids observés en `BTreeSet<i64>` (cf. P-14)
    //     pour neutraliser l'ordre et obtenir une égalité ensembliste
    //     garantissant la double inclusion `⇔`.
    //
    // ≥ 100 cas (cf. `ProptestConfig`).
    // -----------------------------------------------------------------------
    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 100,
            ..ProptestConfig::default()
        })]

        /// **Property 26 (P-26) — Vues `list_favorites()` et
        /// `list_to_watch()`.**
        ///
        /// Pour toute bibliothèque arbitraire avec drapeaux
        /// `(favorite, to_watch)` tirés indépendamment dans
        /// `{0, 1}` :
        ///   * `list_favorites().ids ⇔ { m | m.favorite = 1 }`
        ///   * `list_to_watch().ids  ⇔ { m | m.to_watch  = 1 }`
        ///
        /// Tag: Feature: kyro, Property 26.
        ///
        /// **Validates: Requirements 11.4, 11.5**
        #[test]
        fn p26_list_favorites_and_to_watch_match_flags(
            flags in proptest::collection::vec((0i64..=1, 0i64..=1), 0..=50),
            seed in 0u32..1_000_000,
        ) {
            // Le travail asynchrone (création DB, insertions,
            // listings) est exécuté dans `block_on` qui retourne les
            // ensembles d'ids attendus / observés. Les `prop_assert_*`
            // sont effectués en dehors du futur pour rester
            // compatibles avec `Result<(), TestCaseError>` synthétisé
            // par `proptest!`.
            let (expected_fav, actual_fav, expected_tw, actual_tw) = block_on(async {
                let db_dir = TempDir::new().expect("tempdir DB");
                let pool = fresh_pool(db_dir.path()).await;
                let repo = Repo::new(pool.clone());

                let mut expected_fav: BTreeSet<i64> = BTreeSet::new();
                let mut expected_tw: BTreeSet<i64> = BTreeSet::new();

                for (i, (fav, tw)) in flags.iter().enumerate() {
                    // `seed` + `i` rendent chaque `file_path` unique
                    // entre cas et entre items d'un même cas (chaque
                    // cas ouvre pourtant son propre `TempDir`, mais la
                    // précaution clarifie l'intention).
                    let path = format!("C:/lib/p26_{seed}_{i}.mkv");
                    let id = repo
                        .upsert_media_item(&film_input(None, &path, 1_000 + i as i64))
                        .await
                        .expect("upsert_media_item ne doit pas échouer");

                    // Pré-positionnement explicite des drapeaux
                    // (le DDL force `(0, 0)` à l'insertion ; cet
                    // UPDATE est un no-op pour `(0, 0)` mais
                    // garantit une couverture homogène pour les
                    // autres tirages).
                    sqlx::query(
                        "UPDATE media_item SET favorite = ?, to_watch = ? WHERE id = ?",
                    )
                    .bind(*fav)
                    .bind(*tw)
                    .bind(id.get())
                    .execute(&pool)
                    .await
                    .expect("UPDATE (favorite, to_watch)");

                    if *fav == 1 {
                        expected_fav.insert(id.get());
                    }
                    if *tw == 1 {
                        expected_tw.insert(id.get());
                    }
                }

                let favs = repo
                    .list_favorites()
                    .await
                    .expect("list_favorites ok");
                let tws = repo
                    .list_to_watch()
                    .await
                    .expect("list_to_watch ok");

                let actual_fav: BTreeSet<i64> =
                    favs.iter().map(|m| m.id.get()).collect();
                let actual_tw: BTreeSet<i64> =
                    tws.iter().map(|m| m.id.get()).collect();

                (expected_fav, actual_fav, expected_tw, actual_tw)
            });

            // Cœur de la propriété — équivalence ensembliste sur les
            // deux vues.
            prop_assert_eq!(
                &actual_fav,
                &expected_fav,
                "P-26 (favorites) : ensemble retourné != filtre favorite=1"
            );
            prop_assert_eq!(
                &actual_tw,
                &expected_tw,
                "P-26 (to_watch) : ensemble retourné != filtre to_watch=1"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Tâche 7.8 — Property 1 : Round-trip d'ajout / lecture d'un dossier de
    // bibliothèque.
    //
    // Feature: kyro, Property 1.
    //
    // **Validates: Requirements 2.2, 2.5**
    //
    // Pour tout chemin valide (dossier réellement existant, créé via
    // `tempfile`) et toute catégorie ∈ {Films, Series, Animes}, après
    // `add_library_folder(path, category)` la liste retournée par
    // `list_library_folders()` doit contenir **exactement une entrée**
    // dont :
    //   * la catégorie est égale à `category` (requirement 2.5 — la
    //     catégorie choisie est persistée et restituée à l'identique) ;
    //   * le chemin pointe sur le même dossier que celui passé en
    //     entrée (requirement 2.2 — le chemin absolu est conservé).
    //
    // La comparaison de chemin passe par `std::fs::canonicalize` afin
    // de neutraliser les variantes Windows (préfixe UNC `\\?\`,
    // séparateurs, casse) — c'est précisément la transformation
    // appliquée par `add_library_folder` via `canonicalize_or_keep`.
    // -----------------------------------------------------------------------

    /// Catégorie arbitraire couvrant les trois variantes acceptées par
    /// le `CHECK (category IN ('Films','Series','Animes'))`.
    fn arb_category() -> impl Strategy<Value = Category> {
        prop_oneof![
            Just(Category::Films),
            Just(Category::Series),
            Just(Category::Animes),
        ]
    }

    proptest! {
        // ≥ 100 cas comme exigé par la tâche 7.8. L'espace effectif
        // (3 catégories × chemin de TempDir aléatoire) est largement
        // couvert ; on évite de monter plus haut car chaque cas crée
        // une DB SQLite + un répertoire temporaire (coût I/O non
        // négligeable).
        #![proptest_config(ProptestConfig {
            cases: 100,
            ..ProptestConfig::default()
        })]

        /// **Property 1 (P-1) — Round-trip d'ajout / lecture d'un dossier
        /// de bibliothèque.**
        ///
        /// Tag: Feature: kyro, Property 1.
        ///
        /// **Validates: Requirements 2.2, 2.5**
        #[test]
        fn p1_add_library_folder_round_trip(
            category in arb_category(),
            // Composant de chemin arbitraire (lettres + chiffres + tiret
            // + underscore) que l'on greffe sur un sous-dossier du
            // TempDir. Cela garantit que chaque cas exerce un chemin
            // distinct sans heurter l'unicité `library_folder.path`.
            sub in "[A-Za-z0-9_\\-]{1,16}",
        ) {
            block_on(async move {
                let db_dir = TempDir::new().unwrap();
                let lib_root = TempDir::new().unwrap();
                let lib_dir = lib_root.path().join(&sub);
                std::fs::create_dir_all(&lib_dir)
                    .expect("création du sous-dossier de bibliothèque");

                let pool = fresh_pool(db_dir.path()).await;
                let repo = Repo::new(pool);

                let id = repo
                    .add_library_folder(&lib_dir, category)
                    .await
                    .expect("P-1 : un dossier réellement présent doit être accepté");

                let listed = repo
                    .list_library_folders()
                    .await
                    .expect("P-1 : list_library_folders ne doit pas échouer");

                // (a) Cardinalité — exactement une entrée.
                prop_assert_eq!(
                    listed.len(),
                    1,
                    "P-1 : list_library_folders doit contenir exactement une entrée \
                     après un seul ajout"
                );

                let entry = &listed[0];

                // (b) Identifiant — celui retourné par add_library_folder.
                prop_assert_eq!(entry.id, id);

                // (c) Catégorie — restituée à l'identique (requirement 2.5).
                prop_assert_eq!(
                    entry.category,
                    category,
                    "P-1 : la catégorie persistée doit correspondre à celle ajoutée"
                );

                // (d) Chemin — désigne le même dossier que l'entrée
                // (requirement 2.2). On compare via canonicalize côté
                // attendu pour absorber la même normalisation que
                // `add_library_folder` applique en interne.
                let stored = PathBuf::from(&entry.path);
                let expected = std::fs::canonicalize(&lib_dir)
                    .unwrap_or_else(|_| lib_dir.clone());
                prop_assert_eq!(
                    stored,
                    expected,
                    "P-1 : le chemin persisté doit pointer sur le dossier ajouté"
                );

                Ok(())
            })?;
        }
    }

    // -----------------------------------------------------------------------
    // Tâche 7.9 — Property 2 : Retrait d'un dossier marque tous ses items
    // indisponibles.
    //
    // Feature: kyro, Property 2.
    //
    // **Validates: Requirements 2.3**
    //
    // Pour tout dossier de bibliothèque contenant `n ≥ 0` `media_item`
    // (Films / Series / Animes) et leurs `episode_item` associés (pour
    // les Series/Animes), après `remove_library_folder(id)` on doit
    // observer simultanément :
    //
    //   1. la ligne `library_folder` retirée (`COUNT(*) = 0`) ;
    //   2. **aucun** `media_item` ni `episode_item` supprimé — les
    //      cardinaux de chaque table sont conservés à l'identique ;
    //   3. **tous** les `media_item` rattachés au dossier
    //      (`folder_id = id` au moment du retrait) avec
    //      `available = 0` ;
    //   4. **tous** les `episode_item` dont la série mère est
    //      rattachée au dossier avec `available = 0`.
    //
    // Stratégie de génération :
    //
    //   * Liste de spécifications d'items de cardinal `0..=20` (le cas
    //     `n = 0` est couvert par la borne basse — il valide que le
    //     retrait d'un dossier vide ne casse pas l'invariant).
    //   * Chaque item porte un `MediaType` arbitraire et, pour les
    //     Series/Animes, un nombre d'épisodes `0..=4`. Cela donne
    //     jusqu'à `20 * 4 = 80` `episode_item` par cas, suffisant pour
    //     stresser l'UPDATE croisé `episode_item ← media_item.folder_id`
    //     sans pénaliser le run-time.
    //   * Les `file_path` sont synthétiques et **n'ont pas besoin
    //     d'exister sur disque** — `upsert_media_item` /
    //     `upsert_episode_item` ne valident pas l'existence du fichier
    //     (la validation FS est restreinte à `add_library_folder`,
    //     requirement 2.4 ; l'existence réelle d'un fichier vidéo est
    //     de la responsabilité du scanner — tâche 8).
    //
    // ≥ 100 cas (cf. `ProptestConfig`).
    // -----------------------------------------------------------------------

    /// Spécification compacte d'un item généré par P-2 : type de média
    /// + nombre d'épisodes (0 pour les Films).
    #[derive(Debug, Clone)]
    struct Prop2ItemSpec {
        media_type: MediaType,
        episode_count: u8,
    }

    /// Stratégie : `MediaType` uniforme parmi les trois variantes,
    /// `episode_count ∈ [0, 4]`.
    fn arb_prop2_item_spec() -> impl Strategy<Value = Prop2ItemSpec> {
        (
            prop_oneof![
                Just(MediaType::Film),
                Just(MediaType::Series),
                Just(MediaType::Anime),
            ],
            0u8..=4u8,
        )
            .prop_map(|(media_type, episode_count)| Prop2ItemSpec {
                media_type,
                episode_count,
            })
    }

    /// Lit le flag `available` d'un `episode_item` — pendant de
    /// `read_available` côté `media_item`.
    async fn read_episode_available(pool: &SqlitePool, episode_id: i64) -> i64 {
        sqlx::query_scalar::<_, i64>(
            "SELECT available FROM episode_item WHERE id = ?",
        )
        .bind(episode_id)
        .fetch_one(pool)
        .await
        .expect("lecture episode_item.available")
    }

    /// Compte total de `episode_item` (toutes available confondues) —
    /// pendant de `count_all_media`.
    async fn count_all_episodes(pool: &SqlitePool) -> i64 {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM episode_item")
            .fetch_one(pool)
            .await
            .expect("count episode_item")
    }

    proptest! {
        // ≥ 100 cas comme exigé par la tâche 7.9 ; chaque cas crée un
        // pool SQLite fichier dans un `TempDir` éphémère, applique les
        // migrations, exerce l'invariant complet, puis laisse le `Drop`
        // de `TempDir` nettoyer.
        #![proptest_config(ProptestConfig {
            cases: 100,
            .. ProptestConfig::default()
        })]

        /// **Property 2 (P-2) — Retrait d'un dossier marque tous ses
        /// items indisponibles.**
        ///
        /// Tag: Feature: kyro, Property 2.
        ///
        /// **Validates: Requirements 2.3**
        #[test]
        fn p2_remove_folder_marks_all_items_unavailable(
            specs in proptest::collection::vec(arb_prop2_item_spec(), 0..=20),
        ) {
            // Toute l'I/O DB est exécutée dans `block_on`, qui retourne
            // les ensembles d'ids et compteurs nécessaires aux
            // `prop_assert_*` (effectués hors du futur pour respecter
            // le `Result<(), TestCaseError>` synthétisé par `proptest!`).
            let (
                media_ids,
                episode_ids,
                total_media_before,
                total_episodes_before,
                total_media_after,
                total_episodes_after,
                folder_id_value,
                folder_rows_after,
                media_available_after,
                episode_available_after,
            ) = block_on(async {
                let db_dir = TempDir::new().expect("tempdir DB");
                let lib_dir = TempDir::new().expect("tempdir bibliothèque");
                let pool = fresh_pool(db_dir.path()).await;
                let repo = Repo::new(pool.clone());

                // Le dossier doit réellement exister (requirement 2.4
                // — sinon `add_library_folder` retournerait Validation
                // et la propriété serait vide). On utilise `Series`
                // par défaut : la catégorie n'a pas d'effet sur P-2,
                // l'UPDATE de `remove_library_folder` cible
                // `media_item.folder_id` indépendamment du `type`.
                let folder_id = repo
                    .add_library_folder(lib_dir.path(), Category::Series)
                    .await
                    .expect("ajout dossier doit réussir sur TempDir");

                let mut media_ids: Vec<i64> = Vec::new();
                let mut episode_ids: Vec<i64> = Vec::new();

                for (i, spec) in specs.iter().enumerate() {
                    // `file_path` unique par index pour respecter la
                    // clé d'unicité applicative (un upsert sur le même
                    // chemin produirait un no-op idempotent qui
                    // fausserait le décompte attendu).
                    let media_input = UpsertMediaInput {
                        folder_id: Some(folder_id),
                        media_type: spec.media_type,
                        file_path: format!("C:/kyro-prop2/m_{i}.mkv"),
                        size_bytes: 1_024 + i as i64,
                        title: format!("Item {i}"),
                        year: None,
                        quality: None,
                        duration_seconds: None,
                        video_codec: None,
                        width: None,
                        height: None,
                        numbering_unknown: false,
                    };
                    let media_id = repo
                        .upsert_media_item(&media_input)
                        .await
                        .expect("upsert media_item PBT");
                    media_ids.push(media_id.get());

                    // Épisodes uniquement pour Series / Animes — le
                    // scanner de production ne crée jamais d'épisode
                    // sous un Film, on s'aligne sur cette sémantique.
                    if matches!(
                        spec.media_type,
                        MediaType::Series | MediaType::Anime
                    ) {
                        for j in 0..spec.episode_count {
                            let ep_input = UpsertEpisodeInput {
                                series_id: media_id,
                                season_number: 1,
                                episode_number: (j as i64) + 1,
                                file_path: format!(
                                    "C:/kyro-prop2/m_{i}_s1e{j}.mkv"
                                ),
                                size_bytes: 500 + j as i64,
                                duration_seconds: None,
                                video_codec: None,
                                width: None,
                                height: None,
                                numbering_unknown: false,
                            };
                            let ep_id = repo
                                .upsert_episode_item(&ep_input)
                                .await
                                .expect("upsert episode_item PBT");
                            episode_ids.push(ep_id.get());
                        }
                    }
                }

                let total_media_before = count_all_media(&pool).await;
                let total_episodes_before = count_all_episodes(&pool).await;

                // Acte central : retrait du dossier.
                repo.remove_library_folder(folder_id)
                    .await
                    .expect("remove_library_folder PBT");

                let total_media_after = count_all_media(&pool).await;
                let total_episodes_after = count_all_episodes(&pool).await;
                let folder_rows_after =
                    count_folder_rows(&pool, folder_id.get()).await;

                // Lecture de tous les flags `available` post-retrait.
                let mut media_available_after: Vec<(i64, i64)> = Vec::new();
                for mid in &media_ids {
                    media_available_after
                        .push((*mid, read_available(&pool, *mid).await));
                }
                let mut episode_available_after: Vec<(i64, i64)> = Vec::new();
                for eid in &episode_ids {
                    episode_available_after
                        .push((*eid, read_episode_available(&pool, *eid).await));
                }

                (
                    media_ids,
                    episode_ids,
                    total_media_before,
                    total_episodes_before,
                    total_media_after,
                    total_episodes_after,
                    folder_id.get(),
                    folder_rows_after,
                    media_available_after,
                    episode_available_after,
                )
            });

            // (1) La ligne `library_folder` a été supprimée.
            prop_assert_eq!(
                folder_rows_after,
                0,
                "P-2 : la ligne library_folder id={} doit être supprimée",
                folder_id_value
            );

            // (2) Aucune suppression collatérale — cardinaux conservés.
            prop_assert_eq!(
                total_media_before,
                media_ids.len() as i64,
                "sanity : total media_item avant retrait == nombre inséré"
            );
            prop_assert_eq!(
                total_episodes_before,
                episode_ids.len() as i64,
                "sanity : total episode_item avant retrait == nombre inséré"
            );
            prop_assert_eq!(
                total_media_after,
                total_media_before,
                "P-2 : aucun media_item ne doit être supprimé par remove_library_folder"
            );
            prop_assert_eq!(
                total_episodes_after,
                total_episodes_before,
                "P-2 : aucun episode_item ne doit être supprimé par remove_library_folder"
            );

            // (3) Tous les media_item du dossier ont available = 0.
            for (mid, available) in &media_available_after {
                prop_assert_eq!(
                    *available,
                    0,
                    "P-2 : media_item id={} doit être marqué indisponible",
                    mid
                );
            }

            // (4) Tous les episode_item liés ont available = 0.
            for (eid, available) in &episode_available_after {
                prop_assert_eq!(
                    *available,
                    0,
                    "P-2 : episode_item id={} doit être marqué indisponible",
                    eid
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Tâche 7.13 — Property 14 : Filtrage par catégorie.
    //
    // Feature: kyro, Property 14.
    //
    // **Validates: Requirements 7.2**
    //
    // Pour toute bibliothèque mixte arbitraire de Media_Item de types
    // {Film, Series, Anime} (cardinalité `0..=50`), on vérifie pour
    // chaque catégorie `c` ∈ {Films, Series, Animes} que :
    //
    //     set(list_by_category(c)) == set({ m ∈ items | m.type = type(c) })
    //
    // où `type(c)` est le mapping `Films → Film`, `Series → Series`,
    // `Animes → Anime` matérialisé par `category_to_media_type`. Cette
    // équivalence est **ensembliste** : la propriété ne contraint pas
    // l'ordre (la garantie d'ordre `id ASC` est couverte par les tests
    // exemple-par-exemple `list_by_category_returns_only_matching_type`
    // et `list_by_category_attaches_genres_in_batch`).
    //
    // Stratégie :
    //   * Générateur unique = liste de `MediaType` de longueur 0..=50.
    //     Le `file_path` de chaque item est dérivé de l'index pour
    //     éviter toute collision sur la clé d'unicité applicative
    //     `media_item.file_path` (un upsert sur un même chemin produit
    //     un no-op idempotent qui fausserait le décompte attendu).
    //   * Pour chaque cas : DB SQLite fraîche dans un `TempDir`,
    //     migrations appliquées, insertion via `upsert_media_item`,
    //     puis trois listings (un par catégorie). Les ids attendus
    //     sont collectés en parallèle de l'insertion ; les ids
    //     observés sont projetés en `BTreeSet<i64>` pour neutraliser
    //     l'ordre. L'égalité ensembliste garantit la double inclusion
    //     `⇔` exigée par la propriété.
    //   * Vérification croisée : tous les items retournés par chaque
    //     listing portent bien le `MediaType` attendu (cohérence du
    //     mapping `category_to_media_type`).
    //
    // ≥ 100 cas (cf. `ProptestConfig`).
    // -----------------------------------------------------------------------

    /// Type de média uniformément choisi parmi les trois variantes du
    /// CHECK SQL `media_item.type IN ('Film','Series','Anime')`.
    /// Inline ici (plutôt que partagé via `domain::filters::proptests`)
    /// pour garder les tests d'infrastructure indépendants des modules
    /// de domaine.
    fn arb_media_type_for_p14() -> impl Strategy<Value = MediaType> {
        prop_oneof![
            Just(MediaType::Film),
            Just(MediaType::Series),
            Just(MediaType::Anime),
        ]
    }

    proptest! {
        // ≥ 100 cas comme exigé par la tâche 7.13. La taille de la
        // liste (0..=50) couvre les cas dégénérés (bibliothèque vide,
        // une seule catégorie représentée) et les distributions
        // mixtes où chaque type apparaît à plusieurs reprises.
        #![proptest_config(ProptestConfig {
            cases: 100,
            ..ProptestConfig::default()
        })]

        /// **Property 14 (P-14) — Filtrage par catégorie.**
        ///
        /// Pour toute bibliothèque mixte arbitraire (0..=50 items),
        /// `list_by_category(c)` retourne exactement le sous-ensemble
        /// des items dont `media_type` correspond à `c` selon le
        /// mapping `Films → Film`, `Series → Series`, `Animes → Anime`.
        ///
        /// Tag: Feature: kyro, Property 14.
        ///
        /// **Validates: Requirements 7.2**
        #[test]
        fn p14_list_by_category_equals_type_filter(
            types in proptest::collection::vec(arb_media_type_for_p14(), 0..=50),
        ) {
            // Tout le travail asynchrone (création DB, insertions,
            // listings) est exécuté dans `block_on` qui retourne les
            // ensembles d'ids attendus / observés ainsi que les types
            // observés. Les `prop_assert_*` sont ensuite faits en
            // dehors du futur pour rester compatibles avec
            // `Result<(), TestCaseError>` synthétisé par `proptest!`.
            let (
                expected_films,
                actual_films,
                expected_series,
                actual_series,
                expected_animes,
                actual_animes,
                types_films,
                types_series,
                types_animes,
            ) = block_on(async {
                let db_dir = TempDir::new().expect("tempdir DB");
                let pool = fresh_pool(db_dir.path()).await;
                let repo = Repo::new(pool);

                let mut expected_films: BTreeSet<i64> = BTreeSet::new();
                let mut expected_series: BTreeSet<i64> = BTreeSet::new();
                let mut expected_animes: BTreeSet<i64> = BTreeSet::new();

                // Insertion. Le `file_path` est unique par index pour
                // éviter toute collision sur la clé d'unicité
                // applicative `media_item.file_path` (un upsert sur
                // un même chemin produirait un no-op idempotent qui
                // fausserait le décompte attendu).
                for (i, mt) in types.iter().enumerate() {
                    let input = UpsertMediaInput {
                        folder_id: None,
                        media_type: *mt,
                        file_path: format!("C:/lib/p14_item_{i}.mkv"),
                        size_bytes: 1_000 + i as i64,
                        title: format!("Item {i}"),
                        year: None,
                        quality: None,
                        duration_seconds: None,
                        video_codec: None,
                        width: None,
                        height: None,
                        numbering_unknown: false,
                    };
                    let id = repo
                        .upsert_media_item(&input)
                        .await
                        .expect("upsert_media_item ne doit pas échouer");
                    match mt {
                        MediaType::Film => {
                            expected_films.insert(id.get());
                        }
                        MediaType::Series => {
                            expected_series.insert(id.get());
                        }
                        MediaType::Anime => {
                            expected_animes.insert(id.get());
                        }
                    }
                }

                let films = repo
                    .list_by_category(Category::Films)
                    .await
                    .expect("list_by_category(Films) ok");
                let series = repo
                    .list_by_category(Category::Series)
                    .await
                    .expect("list_by_category(Series) ok");
                let animes = repo
                    .list_by_category(Category::Animes)
                    .await
                    .expect("list_by_category(Animes) ok");

                let actual_films: BTreeSet<i64> =
                    films.iter().map(|m| m.id.get()).collect();
                let actual_series: BTreeSet<i64> =
                    series.iter().map(|m| m.id.get()).collect();
                let actual_animes: BTreeSet<i64> =
                    animes.iter().map(|m| m.id.get()).collect();

                let types_films: Vec<MediaType> =
                    films.iter().map(|m| m.media_type).collect();
                let types_series: Vec<MediaType> =
                    series.iter().map(|m| m.media_type).collect();
                let types_animes: Vec<MediaType> =
                    animes.iter().map(|m| m.media_type).collect();

                (
                    expected_films,
                    actual_films,
                    expected_series,
                    actual_series,
                    expected_animes,
                    actual_animes,
                    types_films,
                    types_series,
                    types_animes,
                )
            });

            // 1. Équivalence ensembliste — cœur de la propriété :
            //    `list_by_category(c) ⇔ { m | m.type = type(c) }`.
            prop_assert_eq!(
                &actual_films,
                &expected_films,
                "P-14 (Films) : ensemble retourné != filtre type=Film"
            );
            prop_assert_eq!(
                &actual_series,
                &expected_series,
                "P-14 (Series) : ensemble retourné != filtre type=Series"
            );
            prop_assert_eq!(
                &actual_animes,
                &expected_animes,
                "P-14 (Animes) : ensemble retourné != filtre type=Anime"
            );

            // 2. Cohérence du mapping `category_to_media_type` —
            //    chaque item retourné porte exactement le type
            //    attendu de sa catégorie. Cette assertion détecte
            //    une éventuelle régression du mapping
            //    Categorie ↔ MediaType (régressions silencieuses
            //    sur la projection serait masquée par l'égalité
            //    ensembliste précédente si jamais le filtre WHERE
            //    laissait passer des types inattendus).
            for t in &types_films {
                prop_assert_eq!(
                    *t,
                    MediaType::Film,
                    "list_by_category(Films) doit ne contenir que des MediaType::Film"
                );
            }
            for t in &types_series {
                prop_assert_eq!(
                    *t,
                    MediaType::Series,
                    "list_by_category(Series) doit ne contenir que des MediaType::Series"
                );
            }
            for t in &types_animes {
                prop_assert_eq!(
                    *t,
                    MediaType::Anime,
                    "list_by_category(Animes) doit ne contenir que des MediaType::Anime"
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Tâche 7.10 — Property 8 (Feature: kyro, Property 8).
    //
    // Persistance fidèle de la taille de fichier.
    //
    // Pour tout fichier temporaire de taille arbitraire dans
    // [0, 1 MiB], après `upsert_media_item` avec
    // `size_bytes = fs::metadata(path).len() as i64` sur une base
    // fraîche, la valeur relue depuis `media_item.size_bytes` doit être
    // strictement égale à `fs::metadata(path).len()`.
    //
    // **Validates: Requirements 5.2**
    //
    // Bornes : on limite à 1 MiB pour garder le test rapide
    // (≥ 100 cas × écriture disque + cycle DB), tout en laissant
    // proptest explorer les bords intéressants (0 octet, valeurs
    // proches de 1 MiB) via le shrinking. La conversion `usize → i64`
    // est sûre tant que `size ≤ 1 MiB ≪ i64::MAX`.
    // -----------------------------------------------------------------------
    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 100,
            ..ProptestConfig::default()
        })]

        /// **Property 8 (P-8) — Persistance fidèle de la taille de fichier.**
        ///
        /// Génère un fichier temporaire de taille arbitraire ∈ [0, 1 MiB],
        /// appelle `upsert_media_item` sur une base fraîche avec
        /// `size_bytes = fs::metadata(path).len() as i64`, puis vérifie
        /// que `SELECT size_bytes FROM media_item WHERE id = ?` rend
        /// exactement la même valeur que `fs::metadata(path).len()`.
        ///
        /// Tag: Feature: kyro, Property 8.
        ///
        /// **Validates: Requirements 5.2**
        #[test]
        fn property_8_size_round_trips_exactly_through_upsert(
            size in 0usize..=1_048_576usize,
            seed in 0u32..1_000_000,
        ) {
            block_on(async move {
                let db_dir = TempDir::new().expect("tempdir db");
                let lib_dir = TempDir::new().expect("tempdir lib");
                let pool = fresh_pool(db_dir.path()).await;
                let repo = Repo::new(pool.clone());

                // 1. Matérialiser un fichier de taille `size` exactement.
                //    Le `seed` rend le nom unique entre cas par
                //    précaution (chaque cas a déjà son propre TempDir).
                let file_path = lib_dir.path().join(format!("p8_{seed}.mkv"));
                let buffer = vec![0u8; size];
                std::fs::write(&file_path, &buffer).expect("écriture fichier temporaire");

                // 2. Lire la taille via fs::metadata — l'oracle.
                let metadata_len = std::fs::metadata(&file_path)
                    .expect("lecture metadata")
                    .len();

                // 3. Upsert avec size_bytes issu de fs::metadata.
                let path_str = file_path.to_string_lossy().into_owned();
                let input = UpsertMediaInput {
                    folder_id: None,
                    media_type: MediaType::Film,
                    file_path: path_str,
                    size_bytes: metadata_len as i64,
                    title: "P8".to_string(),
                    year: None,
                    quality: None,
                    duration_seconds: None,
                    video_codec: None,
                    width: None,
                    height: None,
                    numbering_unknown: false,
                };
                let id = repo
                    .upsert_media_item(&input)
                    .await
                    .expect("upsert_media_item doit réussir");

                // 4. Relire la valeur persistée et la confronter à
                //    fs::metadata(path).len() — l'invariant à vérifier.
                let stored_size: Option<i64> = sqlx::query_scalar(
                    "SELECT size_bytes FROM media_item WHERE id = ?",
                )
                .bind(id.get())
                .fetch_one(&pool)
                .await
                .expect("relecture size_bytes");

                prop_assert_eq!(
                    stored_size,
                    Some(metadata_len as i64),
                    "repo.size_bytes doit être strictement égal à fs::metadata(path).len()"
                );
                Ok(())
            })?;
        }
    }

    // -----------------------------------------------------------------------
    // Tâche 7.15 — Property 18 : Compteur épisodes vus.
    //
    // Feature: kyro, Property 18.
    //
    // **Validates: Requirements 8.5**
    //
    // *For any* série munie d'une liste arbitraire d'épisodes (taille
    // ∈ [0, 30]) dont chaque `status` est tiré uniformément dans
    // {`non_vu`, `en_cours`, `vu`}, le résultat de
    // `Repo::count_episodes_by_status(series_id)` satisfait :
    //
    //   * `counts[Status::Vu]   = card({ e | e.status = Vu })`
    //   * `counts[Status::EnCours] = card({ e | e.status = EnCours })`
    //   * `counts[Status::NonVu]   = card({ e | e.status = NonVu })`
    //   * `counts.values().sum()   = card(episodes)` (le « total » N).
    //
    // C'est l'invariant de comptage attendu côté UI pour l'affichage
    // « X / N épisodes vus » du requirement 8.5.
    //
    // Stratégie de test :
    //   1. Générer un vecteur d'indices de statuts (0=NonVu, 1=EnCours,
    //      2=Vu) de taille `0..=30` — borne haute volontairement
    //      modeste car chaque cas ouvre un pool SQLite + applique les
    //      migrations.
    //   2. Créer une série racine via `make_series_with_episodes` et
    //      insérer les N épisodes (file_paths uniques générés à partir
    //      de l'index local du cas).
    //   3. Faire un `UPDATE episode_item SET status = ?` pour chaque
    //      épisode dont le statut tiré ≠ `NonVu` (les inserts initiaux
    //      étant `non_vu` par défaut côté DDL).
    //   4. Appeler `count_episodes_by_status` et confronter chaque
    //      compteur à l'oracle calculé en Rust pur depuis le vecteur
    //      de statuts.
    // -----------------------------------------------------------------------

    /// Statut arbitraire couvrant les trois variantes acceptées par le
    /// `CHECK (status IN ('non_vu','en_cours','vu'))`. Tirage uniforme
    /// — `prop_oneof!` répartit également les chances entre branches.
    fn arb_status_for_p18() -> impl Strategy<Value = Status> {
        prop_oneof![
            Just(Status::NonVu),
            Just(Status::EnCours),
            Just(Status::Vu),
        ]
    }

    proptest! {
        // ≥ 100 cas comme exigé par la tâche 7.15. Chaque cas crée un
        // pool SQLite + applique les migrations + insère jusqu'à 30
        // épisodes ; coût modéré mais largement tractable.
        #![proptest_config(ProptestConfig {
            cases: 100,
            ..ProptestConfig::default()
        })]

        /// **Property 18 (P-18) — Compteur épisodes vus.**
        ///
        /// Pour toute série munie d'épisodes de statuts arbitraires,
        /// `count_episodes_by_status` retourne pour chaque statut le
        /// cardinal du sous-ensemble correspondant, et la somme des
        /// trois compteurs est égale au nombre total d'épisodes
        /// (`card(episodes)`).
        ///
        /// Tag: Feature: kyro, Property 18.
        ///
        /// **Validates: Requirements 8.5**
        #[test]
        fn p18_count_episodes_by_status_matches_cardinalities(
            statuses in proptest::collection::vec(arb_status_for_p18(), 0..=30),
            seed in 0u32..1_000_000,
        ) {
            // Oracle : compter en Rust pur depuis le vecteur généré.
            let expected_vu: i64 = statuses
                .iter()
                .filter(|s| **s == Status::Vu)
                .count() as i64;
            let expected_en_cours: i64 = statuses
                .iter()
                .filter(|s| **s == Status::EnCours)
                .count() as i64;
            let expected_non_vu: i64 = statuses
                .iter()
                .filter(|s| **s == Status::NonVu)
                .count() as i64;
            let expected_total: i64 = statuses.len() as i64;

            // Toute l'I/O DB s'exécute dans `block_on` ; on extrait la
            // map de comptes pour la confronter à l'oracle hors du
            // futur (les `prop_assert_*` doivent renvoyer
            // `Result<(), TestCaseError>` synthétisé par `proptest!`).
            let counts = block_on(async {
                let db_dir = TempDir::new().expect("tempdir db");
                let pool = fresh_pool(db_dir.path()).await;
                let repo = Repo::new(pool.clone());

                // 1. Construire la liste d'épisodes (chemins uniques).
                //    `seed` rend les chemins disjoints entre cas par
                //    précaution — chaque cas a déjà son propre TempDir
                //    et un pool isolé, mais l'unicité de `file_path`
                //    reste utile pour clarifier les diagnostics si un
                //    `proptest` shrink réutilise des seeds.
                let series_path = format!("C:/series/p18_{seed}");
                let episodes: Vec<(i64, i64, String, i64)> = statuses
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        let i_i64 = i as i64;
                        (
                            1,
                            i_i64 + 1,
                            format!("{series_path}/E{i}.mkv"),
                            100 + i_i64,
                        )
                    })
                    .collect();
                let episodes_ref: Vec<(i64, i64, &str, i64)> = episodes
                    .iter()
                    .map(|(s, e, p, sz)| (*s, *e, p.as_str(), *sz))
                    .collect();

                let (series_id, ep_ids) =
                    make_series_with_episodes(&repo, &series_path, &episodes_ref).await;

                // 2. Forcer le statut de chaque épisode au statut tiré.
                //    Les inserts initiaux sont `non_vu` par défaut, on
                //    n'émet donc d'UPDATE que pour les autres branches
                //    (micro-optim — ne change rien à la sémantique).
                for (status, ep_id) in statuses.iter().zip(ep_ids.iter()) {
                    if *status != Status::NonVu {
                        sqlx::query("UPDATE episode_item SET status = ? WHERE id = ?")
                            .bind(status.as_str())
                            .bind(ep_id.get())
                            .execute(&pool)
                            .await
                            .expect("UPDATE episode_item.status doit réussir");
                    }
                }

                // 3. Appeler la méthode sous test.
                repo.count_episodes_by_status(series_id)
                    .await
                    .expect("count_episodes_by_status doit réussir")
            });

            // 4. Confronter à l'oracle. Les trois clés sont garanties
            //    présentes par contrat de `count_episodes_by_status`.
            let got_vu = counts.get(&Status::Vu).copied().unwrap_or(-1);
            let got_en_cours = counts.get(&Status::EnCours).copied().unwrap_or(-1);
            let got_non_vu = counts.get(&Status::NonVu).copied().unwrap_or(-1);
            let got_total: i64 = counts.values().sum();

            prop_assert_eq!(
                got_vu,
                expected_vu,
                "P-18 : counts[Vu] doit égaler card({{e | e.status = Vu}})"
            );
            prop_assert_eq!(
                got_en_cours,
                expected_en_cours,
                "P-18 : counts[EnCours] doit égaler card({{e | e.status = EnCours}})"
            );
            prop_assert_eq!(
                got_non_vu,
                expected_non_vu,
                "P-18 : counts[NonVu] doit égaler card({{e | e.status = NonVu}})"
            );
            prop_assert_eq!(
                got_total,
                expected_total,
                "P-18 : sum(counts) doit égaler card(episodes) (le total N)"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Tâche 7.18 — Property 36 : Persistance avant signal de succès.
    //
    // Feature: kyro, Property 36.
    //
    // **Validates: Requirements 17.2**
    //
    // Pour toute mutation arbitraire ∈ {UpsertMedia, SetWatchProgress,
    // ForceStatus, ToggleFavorite, ToggleToWatch, UpsertSetting},
    // après que la commande renvoie `Ok(_)`, une lecture immédiate sur
    // le **même** `SqlitePool` doit observer la nouvelle valeur
    // (write-then-read cohérent). C'est la traduction directe du
    // requirement 17.2 : aucune mutation ne doit signaler succès tant
    // que le `COMMIT` n'a pas eu lieu — donc une lecture
    // post-`Ok(_)` ne peut pas voir un état antérieur.
    //
    // Stratégie de génération :
    //
    //   * Énumération `P36Mutation` à six variantes, une par méthode
    //     mutante du `Repo` couverte à ce stade. Le générateur
    //     `arb_p36_mutation` distribue uniformément via `prop_oneof!`.
    //   * Chaque cas ouvre un pool SQLite frais (fichier dans un
    //     `TempDir`), applique les migrations, **pré-populetout**
    //     parent requis (un `media_item` de base avec
    //     `duration_seconds = 10 000` afin que `SetWatchProgress` ait
    //     une durée non-nulle qui exerce le calcul de statut), exécute
    //     la mutation, puis émet une lecture `SELECT` ciblée sur la
    //     ligne affectée pour comparer l'état persisté à la valeur
    //     attendue.
    //   * Pour `ToggleFavorite` / `ToggleToWatch`, l'oracle est la
    //     valeur du flag avant toggle inversée (involution → l'état
    //     post-toggle est le complément). On vérifie en plus que la
    //     valeur retournée par le repo correspond bien à l'état
    //     persisté (cohérence interne du contrat).
    //   * Pour `UpsertSetting`, on tire une clé dans
    //     `RESERVED_KEYS` (l'allowlist) — toute autre clé serait
    //     rejetée par `KyroError::Validation` et la propriété serait
    //     vide.
    //
    // ≥ 100 cas (cf. `ProptestConfig`).
    // -----------------------------------------------------------------------

    /// Mutation arbitraire couverte par P-36. Une variante par méthode
    /// mutante du `Repo` à ce stade du repo.
    #[derive(Debug, Clone)]
    enum P36Mutation {
        UpsertMedia { size: i64, title: String },
        SetWatchProgress { position: u32 },
        ForceStatus { status: Status },
        ToggleFavorite,
        ToggleToWatch,
        UpsertSetting { key_idx: usize, value: String },
    }

    /// Stratégie : six variantes équiprobables. Bornes calibrées :
    ///
    /// * `size ∈ [1, 1 000 000 000]` — couvre la plage usuelle d'un
    ///   fichier vidéo (jusqu'à ~1 Go) tout en excluant 0 (qui
    ///   conduirait au cas idempotent côté `upsert_media_item`).
    /// * `position ∈ [0, 10 000]` — borné par la `duration_seconds`
    ///   du media de base pré-populé pour que `compute_status` ait du
    ///   matériel à ratio.
    /// * `value` UTF-8 imprimable de longueur ≤ 32 — assez varié pour
    ///   exercer la persistance verbatim (cf. test
    ///   `upsert_setting_preserves_value_verbatim`) sans inonder
    ///   proptest de cas pathologiques.
    fn arb_p36_mutation() -> impl Strategy<Value = P36Mutation> {
        prop_oneof![
            (1i64..=1_000_000_000i64, "[A-Za-z0-9 ]{1,32}")
                .prop_map(|(size, title)| P36Mutation::UpsertMedia { size, title }),
            (0u32..=10_000u32)
                .prop_map(|position| P36Mutation::SetWatchProgress { position }),
            prop_oneof![
                Just(Status::NonVu),
                Just(Status::EnCours),
                Just(Status::Vu),
            ]
            .prop_map(|status| P36Mutation::ForceStatus { status }),
            Just(P36Mutation::ToggleFavorite),
            Just(P36Mutation::ToggleToWatch),
            (0usize..RESERVED_KEYS.len(), "[A-Za-z0-9 ./_\\-]{0,32}")
                .prop_map(|(key_idx, value)| P36Mutation::UpsertSetting { key_idx, value }),
        ]
    }

    proptest! {
        // ≥ 100 cas comme exigé par la tâche 7.18. Chaque cas crée
        // un pool SQLite + applique les migrations + exécute une
        // mutation puis une lecture ciblée ; coût modéré.
        #![proptest_config(ProptestConfig {
            cases: 100,
            ..ProptestConfig::default()
        })]

        /// **Property 36 (P-36) — Persistance avant signal de succès.**
        ///
        /// Pour toute mutation arbitraire (`UpsertMedia`,
        /// `SetWatchProgress`, `ForceStatus`, `ToggleFavorite`,
        /// `ToggleToWatch`, `UpsertSetting`), une lecture immédiate
        /// post-`Ok(_)` sur le même `SqlitePool` doit refléter la
        /// nouvelle valeur (write-then-read cohérent).
        ///
        /// Tag: Feature: kyro, Property 36.
        ///
        /// **Validates: Requirements 17.2**
        #[test]
        fn p36_write_then_read_is_consistent(
            mutation in arb_p36_mutation(),
            seed in 0u32..1_000_000,
        ) {
            block_on(async move {
                let db_dir = TempDir::new().expect("tempdir DB");
                let pool = fresh_pool(db_dir.path()).await;
                let repo = Repo::new(pool.clone());

                // Pré-population : un `media_item` de base, requis par
                // toutes les mutations ciblant un id existant
                // (`SetWatchProgress`, `ForceStatus`, `ToggleFavorite`,
                // `ToggleToWatch`). `duration_seconds = 10 000` permet
                // au calcul de statut de fonctionner sur l'intégralité
                // du domaine de `position` (0..=10 000).
                let base_path = format!("C:/p36/base_{seed}.mkv");
                let mut base_input = film_input(None, &base_path, 1_000_000);
                base_input.duration_seconds = Some(10_000);
                let base_id = repo
                    .upsert_media_item(&base_input)
                    .await
                    .expect("P-36 : upsert base media_item");

                match mutation {
                    P36Mutation::UpsertMedia { size, ref title } => {
                        // INSERT d'une nouvelle fiche : un `file_path`
                        // distinct du `base_path` garantit un insert
                        // (pas un refresh idempotent).
                        let path = format!("C:/p36/new_{seed}.mkv");
                        let mut input = film_input(None, &path, size);
                        input.title = title.clone();
                        let id = repo
                            .upsert_media_item(&input)
                            .await
                            .expect("P-36 : upsert nouvelle fiche");

                        let (sb, t): (Option<i64>, String) = sqlx::query_as(
                            "SELECT size_bytes, title FROM media_item WHERE id = ?",
                        )
                        .bind(id.get())
                        .fetch_one(&pool)
                        .await
                        .expect("P-36 : relecture media_item");

                        prop_assert_eq!(
                            sb,
                            Some(size),
                            "P-36 (UpsertMedia) : size_bytes persisté != input"
                        );
                        prop_assert_eq!(
                            &t,
                            title,
                            "P-36 (UpsertMedia) : title persisté != input"
                        );
                    }
                    P36Mutation::SetWatchProgress { position } => {
                        repo.set_watch_progress(
                            ProgressTarget::Media(base_id),
                            position,
                        )
                        .await
                        .expect("P-36 : set_watch_progress doit réussir");

                        let wp: i64 = sqlx::query_scalar(
                            "SELECT watch_progress_seconds FROM media_item WHERE id = ?",
                        )
                        .bind(base_id.get())
                        .fetch_one(&pool)
                        .await
                        .expect("P-36 : relecture watch_progress_seconds");

                        prop_assert_eq!(
                            wp,
                            position as i64,
                            "P-36 (SetWatchProgress) : watch_progress_seconds persisté != position"
                        );
                    }
                    P36Mutation::ForceStatus { status } => {
                        repo.force_status(ProgressTarget::Media(base_id), status)
                            .await
                            .expect("P-36 : force_status doit réussir");

                        let st: String = sqlx::query_scalar(
                            "SELECT status FROM media_item WHERE id = ?",
                        )
                        .bind(base_id.get())
                        .fetch_one(&pool)
                        .await
                        .expect("P-36 : relecture status");

                        prop_assert_eq!(
                            st.as_str(),
                            status.as_str(),
                            "P-36 (ForceStatus) : status persisté != status forcé"
                        );
                    }
                    P36Mutation::ToggleFavorite => {
                        let before: i64 = sqlx::query_scalar(
                            "SELECT favorite FROM media_item WHERE id = ?",
                        )
                        .bind(base_id.get())
                        .fetch_one(&pool)
                        .await
                        .expect("P-36 : lecture favorite avant toggle");

                        let new_state = repo
                            .toggle_favorite(base_id)
                            .await
                            .expect("P-36 : toggle_favorite doit réussir");

                        let after: i64 = sqlx::query_scalar(
                            "SELECT favorite FROM media_item WHERE id = ?",
                        )
                        .bind(base_id.get())
                        .fetch_one(&pool)
                        .await
                        .expect("P-36 : relecture favorite après toggle");

                        let expected = if before == 0 { 1 } else { 0 };
                        prop_assert_eq!(
                            after,
                            expected,
                            "P-36 (ToggleFavorite) : flag DB non inversé après Ok"
                        );
                        prop_assert_eq!(
                            new_state,
                            expected == 1,
                            "P-36 (ToggleFavorite) : valeur retournée != flag DB"
                        );
                    }
                    P36Mutation::ToggleToWatch => {
                        let before: i64 = sqlx::query_scalar(
                            "SELECT to_watch FROM media_item WHERE id = ?",
                        )
                        .bind(base_id.get())
                        .fetch_one(&pool)
                        .await
                        .expect("P-36 : lecture to_watch avant toggle");

                        let new_state = repo
                            .toggle_to_watch(base_id)
                            .await
                            .expect("P-36 : toggle_to_watch doit réussir");

                        let after: i64 = sqlx::query_scalar(
                            "SELECT to_watch FROM media_item WHERE id = ?",
                        )
                        .bind(base_id.get())
                        .fetch_one(&pool)
                        .await
                        .expect("P-36 : relecture to_watch après toggle");

                        let expected = if before == 0 { 1 } else { 0 };
                        prop_assert_eq!(
                            after,
                            expected,
                            "P-36 (ToggleToWatch) : flag DB non inversé après Ok"
                        );
                        prop_assert_eq!(
                            new_state,
                            expected == 1,
                            "P-36 (ToggleToWatch) : valeur retournée != flag DB"
                        );
                    }
                    P36Mutation::UpsertSetting { key_idx, ref value } => {
                        let key = RESERVED_KEYS[key_idx % RESERVED_KEYS.len()];
                        repo.upsert_setting(key, value)
                            .await
                            .expect("P-36 : upsert_setting doit réussir");

                        let v: Option<String> = sqlx::query_scalar(
                            "SELECT value FROM setting_value WHERE key = ?",
                        )
                        .bind(key)
                        .fetch_optional(&pool)
                        .await
                        .expect("P-36 : relecture setting_value");

                        prop_assert_eq!(
                            v.as_deref(),
                            Some(value.as_str()),
                            "P-36 (UpsertSetting) : value persistée != value upserted"
                        );
                    }
                }

                Ok(())
            })?;
        }
    }

    // -----------------------------------------------------------------------
    // Tâche 7.12 — Property 12 : Validation de la note utilisateur.
    //
    // Feature: kyro, Property 12.
    //
    // **Validates: Requirements 6.6**
    //
    // Pour toute fiche persistée et toute note `r` hors de
    // l'intervalle `[0, 10]`, l'appel `Repo::update_user_rating(id, r)`
    // doit :
    //
    //   1. retourner `Err(KyroError::Validation(_))` ;
    //   2. laisser la fiche en base strictement inchangée
    //      (le `user_rating` initial — ici `Some(7)` — n'est pas
    //      réécrit) ;
    //   3. produire un message d'erreur **non vide** rappelant la
    //      plage autorisée.
    //
    // Le générateur produit une note arbitraire dans
    // `[i64::MIN, -1] ∪ [11, i64::MAX]` (deux branches `prop_oneof`)
    // pour exercer aussi bien les valeurs négatives que les valeurs
    // largement supérieures au plafond. La fiche initiale reçoit une
    // note valide via une UPDATE directe SQL — l'oracle de la
    // propriété (« inchangée ») est ainsi indépendant de la méthode
    // sous test.
    // -----------------------------------------------------------------------

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 100,
            .. ProptestConfig::default()
        })]

        /// **Property 12 (P-12) — Validation de la note utilisateur.**
        ///
        /// Tag: Feature: kyro, Property 12.
        ///
        /// Pour toute note `r` hors de l'intervalle `[0, 10]` :
        ///
        ///   * `update_user_rating(id, Some(r))` retourne
        ///     `KyroError::Validation` ;
        ///   * la valeur persistée reste celle pré-existante ;
        ///   * le message d'erreur est non vide et mentionne la
        ///     plage `[0, 10]`.
        ///
        /// **Validates: Requirements 6.6**
        #[test]
        fn p12_update_user_rating_rejects_out_of_range(
            invalid_rating in prop_oneof![
                i64::MIN..=-1i64,
                11i64..=i64::MAX,
            ],
        ) {
            // L'I/O DB s'exécute dans `block_on` qui renvoie le couple
            // (résultat de l'appel sous test, valeur persistée relue).
            // Les `prop_assert!` doivent rester en dehors de l'`async`
            // car ils s'expansent en `return Err(TestCaseError)` —
            // incompatible avec un bloc async retournant `()`.
            let (result, persisted) = block_on(async {
                let db_dir = TempDir::new().expect("tempdir DB");
                let pool = fresh_pool(db_dir.path()).await;
                let repo = Repo::new(pool.clone());

                // 1. Fiche initiale via upsert_media_item (titre,
                //    file_path, taille techniquement valides).
                let path = "C:/movies/PropFiche.mkv";
                let input = film_input(None, path, 1_500_000_000);
                let id = repo
                    .upsert_media_item(&input)
                    .await
                    .expect("P-12 : upsert initial");

                // 2. Initialiser la note utilisateur à une valeur
                //    valide directement en SQL — point de
                //    comparaison stable pour l'invariant
                //    « persistée non modifiée ».
                let initial_rating: i64 = 7;
                sqlx::query("UPDATE media_item SET user_rating = ? WHERE id = ?")
                    .bind(initial_rating)
                    .bind(id.get())
                    .execute(&pool)
                    .await
                    .expect("P-12 : init user_rating valide");

                // 3. Appel sous test avec une note hors plage.
                let result = repo
                    .update_user_rating(id, Some(invalid_rating))
                    .await;

                // 4. Relire la valeur persistée (oracle).
                let persisted: Option<i64> = sqlx::query_scalar::<_, Option<i64>>(
                    "SELECT user_rating FROM media_item WHERE id = ?",
                )
                .bind(id.get())
                .fetch_one(&pool)
                .await
                .expect("P-12 : relecture user_rating");

                (result, persisted)
            });

            // 5. (a) — l'appel est rejeté avec
            //     `KyroError::Validation` et un message non vide
            //     mentionnant la plage `[0, 10]` (requirement 6.6).
            let err = result.expect_err(
                "P-12 : update_user_rating(out_of_range) doit échouer",
            );
            match &err {
                KyroError::Validation(message) => {
                    prop_assert!(
                        !message.is_empty(),
                        "P-12 : message d'erreur vide"
                    );
                    prop_assert!(
                        message.contains("[0, 10]"),
                        "P-12 : message d'erreur sans plage explicite : {:?}",
                        message
                    );
                }
                other => {
                    prop_assert!(
                        false,
                        "P-12 : variante d'erreur attendue KyroError::Validation, \
                         obtenu {:?}",
                        other
                    );
                }
            }

            // 5. (b) — la fiche persistée reste inchangée (note
            //     initiale = 7 toujours présente).
            prop_assert_eq!(
                persisted,
                Some(7i64),
                "P-12 : user_rating modifié alors que l'update est rejeté \
                 (note hors plage)"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Tâche 7.11 — Property 11 : Round-trip d'édition d'une fiche.
    //
    // Feature: kyro, Property 11.
    //
    // **Validates: Requirements 6.1, 6.3, 17.2**
    //
    // Pour toute fiche valide générée par `arb_edit_form` (titre non
    // vide ∈ 1..=80 caractères, année ∈ [1900, 2099] ou nulle, note
    // utilisateur ∈ [0, 10] ou nulle, original_title / synopsis /
    // studio facultatifs, status ∈ {non_vu, en_cours, vu}) :
    //
    //   1. on insère une fiche `media_item` de type Film via
    //      `upsert_media_item` (point d'entrée production) ;
    //   2. on applique les valeurs générées sur les colonnes éditées
    //      manuellement par l'utilisateur via une UPDATE SQL ciblée —
    //      cette UPDATE matérialise l'opération « enregistrer la fiche
    //      éditée » du requirement 6.3 (en attendant la commande
    //      `update_media_item` de la tâche 12.x) ;
    //   3. on relit ces colonnes via un SELECT ;
    //   4. on confronte chaque colonne lue à la valeur générée — toute
    //      différence violerait le requirement 6.3 (« persister les
    //      valeurs ») ou le requirement 17.2 (« la transition d'état
    //      n'est considérée comme réussie qu'après commit »).
    //
    // Le requirement 6.1 fournit la liste exacte des champs éditables
    // côté formulaire : titre, titre original, année, synopsis, genres
    // (hors-périmètre repo, gérés par `media_genre`), note utilisateur
    // ∈ [0, 10], studio ou réalisateur, statut. La propriété couvre
    // ces champs scalaires côté `media_item` ; les genres sont validés
    // par leur propre propriété (P-25 / tâche 7.16).
    //
    // L'oracle est l'identité : la valeur lue après UPDATE doit être
    // strictement égale à la valeur écrite. Aucune normalisation n'est
    // tolérée sur les chaînes (pas de trim, pas de lowercase) : si
    // l'UI permet d'enregistrer `"  Foo  "`, c'est cette chaîne
    // exacte que la fiche relue doit contenir.
    // -----------------------------------------------------------------------

    /// Forme d'édition générée — projette les **champs éditables
    /// manuellement** d'un `media_item` (requirement 6.1) sur des
    /// types Rust de domaine. Le `MediaType` n'est volontairement pas
    /// dans la forme : il est fixé à `Film` pour ce test (les Series /
    /// Animes éditent les mêmes colonnes scalaires, l'invariant
    /// repo-niveau est strictement identique).
    #[derive(Debug, Clone)]
    struct EditForm {
        title: String,
        original_title: Option<String>,
        year: Option<i64>,
        synopsis: Option<String>,
        studio: Option<String>,
        user_rating: Option<i64>,
        status: Status,
    }

    /// Stratégie proptest pour `EditForm` — combine les contraintes
    /// du requirement 6.1 (note ∈ [0, 10], titre non vide) et du DDL
    /// (`year` nullable, `original_title` / `synopsis` / `studio`
    /// nullables). Les bornes de longueur sur les chaînes sont
    /// volontairement modestes pour rester dans des entrées
    /// utilisateur plausibles tout en explorant le bord « 1
    /// caractère » qui valide la garde « non vide ».
    fn arb_edit_form() -> impl Strategy<Value = EditForm> {
        (
            // title : 1..=80 caractères ASCII alphanumériques (+ espace).
            // Le `{1,80}` garantit non-vide (requirement 6.1 — un
            // formulaire d'édition exige un titre).
            "[A-Za-z0-9 ]{1,80}",
            // original_title : Option<String>, 1..=80 caractères.
            proptest::option::of("[A-Za-z0-9 ]{1,80}"),
            // year : Option<i64> ∈ [1900, 2099] — borne calquée sur
            // les autres générateurs d'année (cf. tâche 7.8) et le
            // requirement 4.2.
            proptest::option::of(1900i64..=2099i64),
            // synopsis : Option<String>, 0..=200 caractères. On
            // autorise la chaîne vide explicite — un utilisateur peut
            // saisir un synopsis vide via le formulaire.
            proptest::option::of("[A-Za-z0-9 .,!?]{0,200}"),
            // studio : Option<String>, 1..=40 caractères.
            proptest::option::of("[A-Za-z0-9 ]{1,40}"),
            // user_rating : Option<i64> ∈ [0, 10] — requirement 6.1
            // (« entier de 0 à 10 ») et CHECK constraint du DDL.
            proptest::option::of(0i64..=10i64),
            // status : couvre les trois variantes du CHECK
            // `status IN ('non_vu','en_cours','vu')`.
            prop_oneof![
                Just(Status::NonVu),
                Just(Status::EnCours),
                Just(Status::Vu),
            ],
        )
            .prop_map(
                |(title, original_title, year, synopsis, studio, user_rating, status)| {
                    EditForm {
                        title,
                        original_title,
                        year,
                        synopsis,
                        studio,
                        user_rating,
                        status,
                    }
                },
            )
    }

    proptest! {
        // ≥ 100 cas comme exigé par la tâche 7.11. Chaque cas crée un
        // pool SQLite + applique les migrations + insère une fiche +
        // l'édite via UPDATE + relit les colonnes ; coût modéré mais
        // dominé par l'I/O — on reste à 100 pour garder le temps
        // total tractable (~quelques secondes en local) tout en
        // respectant le minimum imposé par la tâche.
        #![proptest_config(ProptestConfig {
            cases: 100,
            ..ProptestConfig::default()
        })]

        /// **Property 11 (P-11) — Round-trip d'édition d'une fiche.**
        ///
        /// Tag: Feature: kyro, Property 11.
        ///
        /// Pour toute fiche éditée valide, après `UPDATE` SQL des
        /// colonnes utilisateur puis `SELECT`, chaque colonne relue
        /// est strictement égale à la valeur générée.
        ///
        /// **Validates: Requirements 6.1, 6.3, 17.2**
        #[test]
        fn p11_edit_round_trip_preserves_all_user_fields(
            form in arb_edit_form(),
        ) {
            // L'I/O DB est encapsulée dans `block_on` qui retourne
            // l'`EditForm` relu en base. Les `prop_assert_*` sont
            // effectués en dehors du futur pour respecter le
            // `Result<(), TestCaseError>` synthétisé par `proptest!`.
            let observed = block_on(async {
                let db_dir = TempDir::new().expect("tempdir DB");
                let pool = fresh_pool(db_dir.path()).await;
                let repo = Repo::new(pool.clone());

                // 1. Insérer une fiche de base via le point d'entrée
                //    production. Le `file_path` est constant : chaque
                //    cas proptest ouvre un pool frais donc aucune
                //    collision n'est possible avec d'autres cas.
                let base_input = film_input(
                    None,
                    "C:/movies/PropEditRoundTrip.mkv",
                    1_500_000_000,
                );
                let id = repo
                    .upsert_media_item(&base_input)
                    .await
                    .expect("P-11 : upsert initial");

                // 2. Appliquer la fiche éditée sur les colonnes
                //    utilisateur via une UPDATE paramétrée — c'est
                //    l'opération que la commande `update_media_item`
                //    (tâche 12.x) exécutera côté backend après
                //    soumission du formulaire (requirement 6.3).
                //    L'UPDATE unique en autocommit SQLite est
                //    atomique (requirement 17.2 — succès uniquement
                //    après écriture effective).
                sqlx::query(
                    "UPDATE media_item \
                        SET title = ?, \
                            original_title = ?, \
                            year = ?, \
                            synopsis = ?, \
                            studio = ?, \
                            user_rating = ?, \
                            status = ? \
                      WHERE id = ?",
                )
                .bind(&form.title)
                .bind(form.original_title.as_deref())
                .bind(form.year)
                .bind(form.synopsis.as_deref())
                .bind(form.studio.as_deref())
                .bind(form.user_rating)
                .bind(form.status.as_str())
                .bind(id.get())
                .execute(&pool)
                .await
                .expect("P-11 : UPDATE des champs édités");

                // 3. Relire chaque colonne éditée via un SELECT
                //    direct — on évite `list_by_category` /
                //    `search` qui projettent une vue domaine
                //    incomplète (pas de synopsis, pas de studio,
                //    pas de user_rating). Le mapping est manuel
                //    pour rester aligné sur le DDL.
                let row = sqlx::query(
                    "SELECT title, original_title, year, synopsis, \
                            studio, user_rating, status \
                       FROM media_item \
                      WHERE id = ?",
                )
                .bind(id.get())
                .fetch_one(&pool)
                .await
                .expect("P-11 : SELECT après UPDATE");

                let title: String = row.try_get("title").expect("P-11 : title");
                let original_title: Option<String> = row
                    .try_get("original_title")
                    .expect("P-11 : original_title");
                let year: Option<i64> = row.try_get("year").expect("P-11 : year");
                let synopsis: Option<String> =
                    row.try_get("synopsis").expect("P-11 : synopsis");
                let studio: Option<String> =
                    row.try_get("studio").expect("P-11 : studio");
                let user_rating: Option<i64> =
                    row.try_get("user_rating").expect("P-11 : user_rating");
                let status_raw: String =
                    row.try_get("status").expect("P-11 : status");
                let status: Status = status_raw
                    .parse()
                    .expect("P-11 : status persisté hors valeurs autorisées");

                EditForm {
                    title,
                    original_title,
                    year,
                    synopsis,
                    studio,
                    user_rating,
                    status,
                }
            });

            // 4. Round-trip strict, champ par champ. On compare
            //    individuellement plutôt qu'avec un `prop_assert_eq!`
            //    sur la struct entière pour que le diagnostic
            //    proptest pointe directement la colonne fautive en
            //    cas d'échec (un seul champ divergent suffit à
            //    casser l'invariant).
            prop_assert_eq!(
                observed.title.clone(),
                form.title.clone(),
                "P-11 : title non préservé"
            );
            prop_assert_eq!(
                observed.original_title.clone(),
                form.original_title.clone(),
                "P-11 : original_title non préservé"
            );
            prop_assert_eq!(
                observed.year,
                form.year,
                "P-11 : year non préservée"
            );
            prop_assert_eq!(
                observed.synopsis.clone(),
                form.synopsis.clone(),
                "P-11 : synopsis non préservé"
            );
            prop_assert_eq!(
                observed.studio.clone(),
                form.studio.clone(),
                "P-11 : studio non préservé"
            );
            prop_assert_eq!(
                observed.user_rating,
                form.user_rating,
                "P-11 : user_rating non préservée"
            );
            prop_assert_eq!(
                observed.status,
                form.status,
                "P-11 : status non préservé"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Tests de propriété (tâche 7.14).
//
// `proptests` couvre :
//
//   * **Property 15 — Tri saisons puis épisodes** (Feature: kyro,
//     Property 15). Pour toute série et toute liste arbitraire
//     d'`EpisodeItem` insérés dans un ordre quelconque, la sortie de
//     `Repo::list_episodes(series_id)` est triée lexicographiquement
//     par `(season_number ASC, episode_number ASC)`.
//
// Le test ouvre un pool SQLite frais par cas (fichier dans un
// `TempDir`), applique les migrations, insère la série racine puis les
// épisodes générés et compare la projection `(season, episode)` de la
// sortie avec sa version triée par l'oracle.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod proptests {
    //! Tâche 7.14 — Property 15 (tri `(season, episode)` ASC).
    //!
    //! Validates: Requirements 7.3, 7.4

    use super::*;
    use crate::domain::types::MediaType;
    use proptest::collection::vec;
    use proptest::prelude::*;
    use sqlx::sqlite::{
        SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
    };
    use std::path::Path;
    use tempfile::TempDir;

    /// Bornes du générateur — calibrées sur des séries réelles tout en
    /// laissant suffisamment d'espace de combinaisons pour exercer le
    /// tri sur de nombreuses configurations distinctes :
    ///
    /// * `MAX_SEASON = 50` couvre les séries longues (ex. soaps) et
    ///   pousse `season_number` au-delà de 9 pour que le tri
    ///   numérique (et non lexicographique sur des chaînes) soit
    ///   réellement testé.
    /// * `MAX_EPISODE = 200` permet d'avoir des épisodes au-delà de 99
    ///   et donc de distinguer un tri numérique correct (`E10` vient
    ///   après `E9`) d'un tri textuel erroné (`E10` viendrait avant
    ///   `E9`).
    const MAX_SEASON: i64 = 50;
    const MAX_EPISODE: i64 = 200;

    /// Construit un runtime `tokio` `current_thread` dédié à un cas
    /// proptest. On n'utilise pas `#[tokio::test]` car la feature
    /// `macros` de `tokio` n'est pas activée dans `Cargo.toml`.
    fn block_on<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("création d'un runtime tokio current_thread")
            .block_on(fut)
    }

    /// Reproduit `tests::fresh_pool` (privé au module `tests`) : ouvre
    /// un pool SQLite sur fichier temporaire avec les pragmas de
    /// production, applique les migrations.
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

    /// Génère une paire `(season, episode)` arbitraire dans les bornes
    /// `[1, MAX_SEASON] × [1, MAX_EPISODE]`. Les numéros démarrent à
    /// `1` pour respecter l'usage métier (saisons et épisodes
    /// numérotés à partir de 1) tout en laissant proptest explorer
    /// largement l'espace produit.
    fn arb_season_episode() -> impl Strategy<Value = (i64, i64)> {
        (1i64..=MAX_SEASON, 1i64..=MAX_EPISODE)
    }

    /// Génère une liste d'épisodes — uniquement les paires `(season,
    /// episode)`. Les `file_path` sont fabriqués à l'insertion à partir
    /// de l'index dans la liste pour garantir l'unicité requise par la
    /// contrainte `episode_item.file_path UNIQUE` du DDL, indépendamment
    /// des éventuelles collisions de paires `(season, episode)` dans le
    /// générateur (le tri est défini même en cas de doublons : il
    /// préserve simplement un ordre cohérent côté SQL).
    ///
    /// Taille de liste **1..30** comme spécifié par la tâche.
    fn arb_episode_list() -> impl Strategy<Value = Vec<(i64, i64)>> {
        vec(arb_season_episode(), 1..30)
    }

    proptest! {
        // Le test ouvre un pool SQLite par cas (création + migrations).
        // C'est volontairement coûteux mais reste tractable : on
        // calibre `cases = 100` pour respecter le minimum de la tâche
        // tout en gardant un temps total raisonnable (~quelques
        // secondes en local).
        #![proptest_config(ProptestConfig {
            cases: 100,
            .. ProptestConfig::default()
        })]

        // -------------------------------------------------------------
        // Feature: kyro, Property 15 — Tri saisons puis épisodes.
        //
        // Pour toute série et toute liste arbitraire de paires
        // `(season_number, episode_number)` insérées dans un ordre
        // quelconque, `Repo::list_episodes(series_id)` retourne la
        // liste triée lexicographiquement par
        // `(season_number ASC, episode_number ASC)`.
        //
        // L'oracle est simplement la projection `(season, episode)` de
        // la liste générée triée par `slice::sort` (ordre lex naturel
        // sur les tuples `(i64, i64)`). Cette équivalence couvre
        // simultanément :
        //
        //   * Requirement 7.3 — affichage des épisodes par saison
        //     puis par numéro pour la fiche détail d'une Série/Anime.
        //   * Requirement 7.4 — la projection `EpisodeRow` consommée
        //     par `next_resume_episode` (tâche 4.2) doit être déjà
        //     ordonnée pour que l'algorithme de reprise pioche
        //     correctement le prochain épisode.
        //
        // Validates: Requirements 7.3, 7.4
        // -------------------------------------------------------------
        #[test]
        fn p15_list_episodes_sorted_by_season_then_episode(
            pairs in arb_episode_list(),
        ) {
            // L'I/O DB est exécutée dans `block_on` qui retourne la
            // projection `(season, episode)` de la sortie de
            // `list_episodes`. Les `prop_assert!` doivent rester en
            // dehors de l'`async` car ils s'expansent en
            // `return Err(TestCaseError)` — incompatible avec un bloc
            // async retournant `()`.
            let actual: Vec<(i64, i64)> = block_on(async {
                let db_dir = TempDir::new().expect("tempdir DB");
                let pool = fresh_pool(db_dir.path()).await;
                let repo = Repo::new(pool);

                // 1. Insérer la série racine via upsert_media_item.
                let series_input = UpsertMediaInput {
                    folder_id: None,
                    media_type: MediaType::Series,
                    file_path: "C:/series/PropShow".to_string(),
                    size_bytes: 0,
                    title: "PropShow".to_string(),
                    year: None,
                    quality: None,
                    duration_seconds: None,
                    video_codec: None,
                    width: None,
                    height: None,
                    numbering_unknown: false,
                };
                let series_id = repo
                    .upsert_media_item(&series_input)
                    .await
                    .expect("upsert série");

                // 2. Insérer chaque épisode avec un file_path unique
                //    dérivé de l'index (la contrainte UNIQUE sur
                //    file_path interdit les collisions, mais les
                //    paires (season, episode) peuvent en revanche
                //    être dupliquées et le tri doit rester défini).
                for (idx, (season, episode)) in pairs.iter().enumerate() {
                    let path = format!(
                        "C:/series/PropShow/S{:02}E{:03}_idx{}.mkv",
                        season, episode, idx
                    );
                    let ep = UpsertEpisodeInput {
                        series_id,
                        season_number: *season,
                        episode_number: *episode,
                        file_path: path,
                        size_bytes: 1_000 + idx as i64,
                        duration_seconds: None,
                        video_codec: None,
                        width: None,
                        height: None,
                        numbering_unknown: false,
                    };
                    repo.upsert_episode_item(&ep)
                        .await
                        .expect("upsert épisode");
                }

                // 3. Lister via la méthode sous test, projeter en
                //    `(season, episode)`. La projection est suffisante
                //    pour vérifier l'invariant de tri ; les autres
                //    champs d'`EpisodeRow` (id, status, available)
                //    sont couverts par les tests exemple-par-exemple.
                let listed = repo
                    .list_episodes(series_id)
                    .await
                    .expect("list_episodes");

                listed
                    .iter()
                    .map(|e| (e.season_number, e.episode_number))
                    .collect()
            });

            // 4. Cardinalité préservée : chaque paire générée
            //    correspond exactement à une ligne en base
            //    (file_path uniques fabriqués à l'insertion).
            prop_assert_eq!(actual.len(), pairs.len());

            // 5. Oracle : projection `(season, episode)` triée par
            //    l'ordre lex naturel sur les tuples `(i64, i64)`. La
            //    `BTreeMap` n'est pas applicable ici car la spec
            //    n'interdit pas les paires dupliquées ; on utilise
            //    donc un tri stable directement sur la liste.
            let mut expected: Vec<(i64, i64)> = pairs.clone();
            expected.sort();
            prop_assert_eq!(actual.clone(), expected);

            // 6. Garde-fou redondant : la sortie est non-décroissante
            //    sur `(season, episode)`. Un échec ici localise
            //    l'invariant rompu (la paire courante < la
            //    précédente) plutôt que de comparer deux vecteurs
            //    entiers.
            for w in actual.windows(2) {
                prop_assert!(
                    w[0] <= w[1],
                    "list_episodes non triée : {:?} > {:?}",
                    w[0],
                    w[1]
                );
            }
        }
    }
}
