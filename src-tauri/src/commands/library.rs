//! Tâche 12.1 — surface Tauri Commands « library ».
//!
//! Ce module regroupe les commandes `#[tauri::command]` exposées au
//! frontend SvelteKit pour gérer les fiches `Media_Item`, les
//! `Episode_Item`, leurs métadonnées éditables manuellement, leur
//! statut de visionnage, leurs drapeaux `favori` / `à_regarder`, leur
//! affiche locale et la reprise au sein d'une série.
//!
//! _Requirements: 6.1, 6.2, 6.3, 6.5, 6.6, 7.2, 7.4, 8.4, 8.5, 11.2,
//! 11.3, 11.4, 11.5, 12.2, 13.2_
//!
//! ## Architecture
//!
//! Pour chaque commande, on définit deux niveaux :
//!
//! 1. Un helper `*_impl(repo: &Repo, ...) -> Result<T, KyroError>` —
//!    public au crate, **testable** sans dépendance à
//!    `tauri::AppHandle` ni au runtime IPC. C'est ce qui exécute la
//!    validation des entrées, la délégation au [`Repo`] et, pour les
//!    quelques cas où le repo ne dispose pas encore d'une méthode
//!    typée (ex. `get_media_item`, `update_media_item`,
//!    `update_episode_item`), la requête SQL directe via
//!    `repo.pool()`.
//! 2. Une fonction `#[tauri::command] pub async fn xxx(state, ...)` —
//!    thin wrapper qui résout l'état partagé Tauri (`tauri::State<'_,
//!    Repo>`) et délègue au helper. La registration sur le
//!    `tauri::Builder::invoke_handler` est explicitement déléguée à
//!    la tâche 12.6 — ce module se contente d'exposer les fonctions.
//!
//! ## Validation des entrées (requirements 6.6, 4.2)
//!
//! Toutes les commandes mutantes valident en amont avant tout SQL :
//!
//! * `year` ∈ [1900, 2099] ou `None` (requirement 4.2 — bornes du
//!   pattern de parsing) ;
//! * `user_rating` ∈ [0, 10] ou `None` (requirement 6.6 — message
//!   indiquant la plage autorisée). Pour les fiches `Media_Item`,
//!   l'écriture finale passe également par
//!   [`Repo::update_user_rating`] qui matérialise la même garantie au
//!   niveau du repo (tâche 7.12).
//!
//! Les autres champs textuels (`title`, `synopsis`, `studio`, etc.)
//! sont conservés à l'identique tels que reçus du frontend (pas de
//! `trim`, pas de normalisation) afin que la valeur lue après
//! enregistrement soit strictement identique à la valeur saisie
//! (cf. propriété 11 / tâche 7.11).

use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::domain::status::Status;
use crate::domain::types::{Category, EpisodeItemId, MediaItem, MediaItemId, MediaType};
use crate::error::KyroError;
use crate::infra::posters::PosterTarget;
use crate::infra::repo::{EpisodeRow, ProgressTarget, Repo};

// ---------------------------------------------------------------------------
// DTOs — projections sérialisables pour le pont IPC Tauri.
//
// Les types « domaine » (`MediaItem`, `EpisodeRow`, `Status`, etc.)
// portent des `enum` Rust dont la sérialisation textuelle vers le
// frontend doit rester stable. On définit ici des DTOs avec
// `#[derive(Serialize, Deserialize)]` qui projettent ces types vers
// des chaînes (`type`, `status`, `category`) et des `i64`
// (identifiants), pour deux raisons :
//
//   * Les types domaine n'implémentent pas tous `Serialize` (cf.
//     `MediaType`, `MediaItemId`, `Category`) — les ajouter
//     toucherait des couches transverses. On préfère un DTO local au
//     module commands pour matérialiser le contrat IPC.
//   * Les chaînes (`"Film"`, `"non_vu"`, etc.) sont stables côté DB
//     (CHECK SQL) ET côté frontend, donc utiliser le `as_str()` du
//     domaine comme valeur de la chaîne IPC évite toute divergence.
// ---------------------------------------------------------------------------

/// Vue minimale d'un Media_Item pour les listings et résultats de
/// recherche. Construit à partir de [`MediaItem`] du domaine ; les
/// champs `media_type` et `status` sont sérialisés sous leur forme
/// stable (`"Film"`, `"non_vu"`, etc.).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaItemDto {
    pub id: i64,
    pub title: String,
    pub original_title: Option<String>,
    pub year: Option<u16>,
    /// Valeurs : `"Film"`, `"Series"`, `"Anime"` (alignées sur le
    /// CHECK SQL `media_item.type`).
    #[serde(rename = "type")]
    pub media_type: String,
    /// Valeurs : `"non_vu"`, `"en_cours"`, `"vu"` (alignées sur le
    /// CHECK SQL `media_item.status`).
    pub status: String,
    pub genres: Vec<String>,
    /// Chemin absolu local de l'affiche, ou `None`. Le frontend le
    /// passe à `convertFileSrc` (Tauri v2) pour obtenir une URL
    /// `asset://localhost/...` consommable par `<img>`.
    pub poster_path: Option<String>,
}

impl From<MediaItem> for MediaItemDto {
    fn from(m: MediaItem) -> Self {
        MediaItemDto {
            id: m.id.get(),
            title: m.title,
            original_title: m.original_title,
            year: m.year,
            media_type: m.media_type.as_str().to_string(),
            status: m.status.as_str().to_string(),
            genres: m.genres,
            poster_path: m.poster_path,
        }
    }
}

/// Fiche complète d'un Media_Item retournée par
/// [`get_media_item_impl`]. Inclut toutes les colonnes éditables
/// manuellement (requirement 6.1) ainsi que les drapeaux et la
/// progression de lecture.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaItemDetailDto {
    pub id: i64,
    #[serde(rename = "type")]
    pub media_type: String,
    pub title: String,
    pub original_title: Option<String>,
    pub year: Option<i64>,
    pub synopsis: Option<String>,
    pub studio: Option<String>,
    pub user_rating: Option<i64>,
    pub poster_path: Option<String>,
    pub quality: Option<String>,
    pub duration_seconds: Option<i64>,
    pub video_codec: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub watch_progress_seconds: i64,
    pub status: String,
    pub favorite: bool,
    pub to_watch: bool,
    pub available: bool,
    pub numbering_unknown: bool,
    pub genres: Vec<String>,
}

/// Vue d'un Episode_Item retournée par [`list_episodes_impl`] et
/// [`next_resume_episode_impl`]. Strictement la projection consommée
/// par les pages `/detail/:id` côté frontend.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EpisodeItemDto {
    pub id: i64,
    pub season_number: i64,
    pub episode_number: i64,
    pub status: String,
    pub available: bool,
    /// Position de lecture courante en secondes (alimentée par le
    /// pont mpv.net). Permet à l'UI d'afficher « X min restantes ».
    pub watch_progress_seconds: i64,
    /// Durée totale en secondes (peut être `null` si ffprobe n'a pas
    /// encore pu la déterminer).
    pub duration_seconds: Option<i64>,
}

impl From<EpisodeRow> for EpisodeItemDto {
    fn from(e: EpisodeRow) -> Self {
        EpisodeItemDto {
            id: e.id.get(),
            season_number: e.season_number,
            episode_number: e.episode_number,
            status: e.status.as_str().to_string(),
            available: e.available,
            watch_progress_seconds: e.watch_progress_seconds,
            duration_seconds: e.duration_seconds,
        }
    }
}

/// Payload d'`update_media_item` — porte les champs éditables d'un
/// Media_Item (requirement 6.1). `genres` n'est pas inclus dans cette
/// première itération : la table `media_genre` est gérée séparément
/// (les commandes dédiées arriveront avec la tâche 14.x).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateMediaItemInput {
    pub title: String,
    pub original_title: Option<String>,
    pub year: Option<i64>,
    pub synopsis: Option<String>,
    pub studio: Option<String>,
    pub user_rating: Option<i64>,
    /// Valeurs admises : `"non_vu"`, `"en_cours"`, `"vu"`.
    pub status: String,
}

/// Payload d'`update_episode_item` — porte les champs éditables d'un
/// Episode_Item (requirement 6.5). L'affiche locale passe par la
/// commande dédiée [`import_poster_for_episode`] (cf. tâche 7.7).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateEpisodeItemInput {
    pub title: Option<String>,
    pub synopsis: Option<String>,
    /// Valeurs admises : `"non_vu"`, `"en_cours"`, `"vu"`.
    pub status: String,
}

/// Distingue la table cible pour les commandes polymorphes
/// (`force_status`, `set_watch_progress`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    Media,
    Episode,
}

// ---------------------------------------------------------------------------
// Validation helpers — requirements 6.6 (rating ∈ [0, 10]) et 4.2
// (year ∈ [1900, 2099]).
// ---------------------------------------------------------------------------

/// Borne basse de l'année admise (requirement 4.2).
pub const YEAR_MIN: i64 = 1900;
/// Borne haute de l'année admise (requirement 4.2).
pub const YEAR_MAX: i64 = 2099;

/// Refuse une année hors `[1900, 2099]` avec un message indiquant la
/// plage autorisée. `None` est admis (colonne nullable, requirement
/// 6.1 — l'année peut rester vide).
fn validate_year(year: Option<i64>) -> Result<(), KyroError> {
    if let Some(y) = year {
        if !(YEAR_MIN..=YEAR_MAX).contains(&y) {
            return Err(KyroError::Validation(format!(
                "l'année {y} est hors de la plage autorisée [{YEAR_MIN}, {YEAR_MAX}]"
            )));
        }
    }
    Ok(())
}

/// Refuse une note hors `[0, 10]` avec un message indiquant la plage
/// autorisée (requirement 6.6). `None` est admis (la note peut rester
/// vide). Cette validation côté commande double celle effectuée par
/// [`Repo::update_user_rating`] (tâche 7.12) — on rejette ici
/// **avant** toute écriture pour produire le message FR localisé sans
/// avoir à ouvrir une transaction.
fn validate_rating(rating: Option<i64>) -> Result<(), KyroError> {
    if let Some(r) = rating {
        if !(0..=10).contains(&r) {
            return Err(KyroError::Validation(format!(
                "la note utilisateur {r} est hors de la plage autorisée [0, 10]"
            )));
        }
    }
    Ok(())
}

/// Parse une chaîne `"Films"` / `"Series"` / `"Animes"` vers
/// [`Category`]. Retourne `KyroError::Validation` si la valeur est
/// hors allowlist.
fn parse_category(raw: &str) -> Result<Category, KyroError> {
    match raw {
        "Films" => Ok(Category::Films),
        "Series" => Ok(Category::Series),
        "Animes" => Ok(Category::Animes),
        other => Err(KyroError::Validation(format!(
            "catégorie inconnue: {other:?} (attendu: Films, Series, Animes)"
        ))),
    }
}

/// Parse une chaîne `"non_vu"` / `"en_cours"` / `"vu"` vers
/// [`Status`]. Retourne `KyroError::Validation` si la valeur est hors
/// allowlist.
fn parse_status(raw: &str) -> Result<Status, KyroError> {
    raw.parse().map_err(|_| {
        KyroError::Validation(format!(
            "statut inconnu: {raw:?} (attendu: non_vu, en_cours, vu)"
        ))
    })
}

/// Construit une [`ProgressTarget`] à partir du couple `(kind, id)`
/// reçu via IPC.
fn build_progress_target(kind: TargetKind, id: i64) -> ProgressTarget {
    match kind {
        TargetKind::Media => ProgressTarget::Media(MediaItemId(id)),
        TargetKind::Episode => ProgressTarget::Episode(EpisodeItemId(id)),
    }
}

// ---------------------------------------------------------------------------
// Helpers `*_impl` — testables sans `tauri::AppHandle`.
// ---------------------------------------------------------------------------

/// Helper testable pour `list_by_category` (requirement 7.2).
pub async fn list_by_category_impl(
    repo: &Repo,
    category: &str,
) -> Result<Vec<MediaItemDto>, KyroError> {
    let cat = parse_category(category)?;
    let items = repo.list_by_category(cat).await?;
    Ok(items.into_iter().map(MediaItemDto::from).collect())
}

/// Helper testable pour `search` (requirement 13.2).
pub async fn search_impl(repo: &Repo, query: &str) -> Result<Vec<MediaItemDto>, KyroError> {
    let items = repo.search(query).await?;
    Ok(items.into_iter().map(MediaItemDto::from).collect())
}

/// Helper testable pour `list_favorites` (requirement 11.4).
pub async fn list_favorites_impl(repo: &Repo) -> Result<Vec<MediaItemDto>, KyroError> {
    let items = repo.list_favorites().await?;
    Ok(items.into_iter().map(MediaItemDto::from).collect())
}

/// Helper testable pour `list_to_watch` (requirement 11.5).
pub async fn list_to_watch_impl(repo: &Repo) -> Result<Vec<MediaItemDto>, KyroError> {
    let items = repo.list_to_watch().await?;
    Ok(items.into_iter().map(MediaItemDto::from).collect())
}

/// Helper testable pour `list_episodes` (requirements 7.3, 7.4).
pub async fn list_episodes_impl(
    repo: &Repo,
    series_id: i64,
) -> Result<Vec<EpisodeItemDto>, KyroError> {
    let rows = repo.list_episodes(MediaItemId(series_id)).await?;
    Ok(rows.into_iter().map(EpisodeItemDto::from).collect())
}

/// Helper testable pour `next_resume_episode` (requirement 12.2).
pub async fn next_resume_episode_impl(
    repo: &Repo,
    series_id: i64,
) -> Result<Option<i64>, KyroError> {
    let id = repo.next_resume_episode(MediaItemId(series_id)).await?;
    Ok(id.map(|e| e.get()))
}

/// Helper testable pour `toggle_favorite` (requirement 11.2).
pub async fn toggle_favorite_impl(repo: &Repo, id: i64) -> Result<bool, KyroError> {
    repo.toggle_favorite(MediaItemId(id)).await
}

/// Helper testable pour `toggle_to_watch` (requirement 11.3).
pub async fn toggle_to_watch_impl(repo: &Repo, id: i64) -> Result<bool, KyroError> {
    repo.toggle_to_watch(MediaItemId(id)).await
}

/// Helper testable pour `force_status` (requirement 8.4).
pub async fn force_status_impl(
    repo: &Repo,
    target_kind: TargetKind,
    target_id: i64,
    status: &str,
) -> Result<(), KyroError> {
    let st = parse_status(status)?;
    let target = build_progress_target(target_kind, target_id);
    repo.force_status(target, st).await
}

/// Helper testable pour `set_watch_progress` (requirements 8.1, 8.2,
/// 8.3, 9.4). Retourne le nouveau statut sérialisé en chaîne stable.
pub async fn set_watch_progress_impl(
    repo: &Repo,
    target_kind: TargetKind,
    target_id: i64,
    position: u32,
) -> Result<String, KyroError> {
    let target = build_progress_target(target_kind, target_id);
    let new_status = repo.set_watch_progress(target, position).await?;
    Ok(new_status.as_str().to_string())
}

/// Helper testable pour `import_poster_for_media` (requirements 6.2,
/// 6.3). Le `posters_dir` est passé explicitement par l'appelant
/// (tâche 12.x : la commande Tauri résoudra `%APPDATA%\Kyro\posters\`
/// via `tauri::Manager::path()`).
pub async fn import_poster_for_media_impl(
    repo: &Repo,
    media_id: i64,
    source_path: &std::path::Path,
    posters_dir: &std::path::Path,
) -> Result<String, KyroError> {
    let dest = repo
        .import_poster(
            PosterTarget::Media(MediaItemId(media_id)),
            source_path,
            posters_dir,
        )
        .await?;
    Ok(dest.to_string_lossy().into_owned())
}

/// Helper testable pour `import_poster_for_episode` (requirements
/// 6.2, 6.3, 6.5).
pub async fn import_poster_for_episode_impl(
    repo: &Repo,
    episode_id: i64,
    source_path: &std::path::Path,
    posters_dir: &std::path::Path,
) -> Result<String, KyroError> {
    let dest = repo
        .import_poster(
            PosterTarget::Episode(EpisodeItemId(episode_id)),
            source_path,
            posters_dir,
        )
        .await?;
    Ok(dest.to_string_lossy().into_owned())
}

/// Helper testable pour `get_media_item` — retourne la fiche complète
/// éditable, attache les genres en batch (requirement 6.1).
pub async fn get_media_item_impl(
    repo: &Repo,
    id: i64,
) -> Result<MediaItemDetailDto, KyroError> {
    let row = sqlx::query(
        "SELECT id, type, title, original_title, year, synopsis, studio, \
                user_rating, poster_path, quality, duration_seconds, video_codec, \
                width, height, watch_progress_seconds, status, favorite, to_watch, \
                available, numbering_unknown \
           FROM media_item \
          WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(repo.pool())
    .await?
    .ok_or_else(|| {
        KyroError::Validation(format!("media_item id={id} introuvable"))
    })?;

    let media_type_str: String = row.try_get("type")?;
    let status_str: String = row.try_get("status")?;
    let favorite_int: i64 = row.try_get("favorite")?;
    let to_watch_int: i64 = row.try_get("to_watch")?;
    let available_int: i64 = row.try_get("available")?;
    let numbering_unknown_int: i64 = row.try_get("numbering_unknown")?;

    // Charger les genres associés (table de jointure `media_genre`).
    let genre_rows = sqlx::query(
        "SELECT g.name \
           FROM media_genre mg \
           JOIN genre g ON g.id = mg.genre_id \
          WHERE mg.media_item_id = ? \
          ORDER BY g.name ASC",
    )
    .bind(id)
    .fetch_all(repo.pool())
    .await?;
    let genres: Vec<String> = genre_rows
        .iter()
        .map(|r| r.try_get::<String, _>("name"))
        .collect::<Result<_, _>>()?;

    Ok(MediaItemDetailDto {
        id: row.try_get("id")?,
        media_type: media_type_str,
        title: row.try_get("title")?,
        original_title: row.try_get("original_title")?,
        year: row.try_get("year")?,
        synopsis: row.try_get("synopsis")?,
        studio: row.try_get("studio")?,
        user_rating: row.try_get("user_rating")?,
        poster_path: row.try_get("poster_path")?,
        quality: row.try_get("quality")?,
        duration_seconds: row.try_get("duration_seconds")?,
        video_codec: row.try_get("video_codec")?,
        width: row.try_get("width")?,
        height: row.try_get("height")?,
        watch_progress_seconds: row.try_get("watch_progress_seconds")?,
        status: status_str,
        favorite: favorite_int != 0,
        to_watch: to_watch_int != 0,
        available: available_int != 0,
        numbering_unknown: numbering_unknown_int != 0,
        genres,
    })
}

/// Helper testable pour `update_media_item` (requirements 6.1, 6.3,
/// 6.6, 4.2, 17.2).
///
/// Validation préalable :
/// 1. `title` non vide après trim (requirement 6.1 — formulaire exige
///    un titre) ;
/// 2. `year` ∈ [1900, 2099] ou `None` ;
/// 3. `user_rating` ∈ [0, 10] ou `None` ;
/// 4. `status` ∈ `{non_vu, en_cours, vu}`.
///
/// Mutation encapsulée dans une transaction unique : succès uniquement
/// après `COMMIT` (requirement 17.2). L'`update_user_rating` est
/// déclenchée séparément côté repo pour matérialiser la même garantie
/// de plage que la propriété 12 — mais comme on doit aussi muter les
/// autres champs dans la même transaction, on inline le `UPDATE`
/// global ici après avoir validé la note.
pub async fn update_media_item_impl(
    repo: &Repo,
    id: i64,
    input: &UpdateMediaItemInput,
) -> Result<(), KyroError> {
    if input.title.trim().is_empty() {
        return Err(KyroError::Validation(
            "le titre ne peut pas être vide".to_string(),
        ));
    }
    validate_year(input.year)?;
    validate_rating(input.user_rating)?;
    let status = parse_status(&input.status)?;

    let mut tx = repo.pool().begin().await?;
    let affected = sqlx::query(
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
    .bind(&input.title)
    .bind(input.original_title.as_deref())
    .bind(input.year)
    .bind(input.synopsis.as_deref())
    .bind(input.studio.as_deref())
    .bind(input.user_rating)
    .bind(status.as_str())
    .bind(id)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    if affected == 0 {
        tx.rollback().await?;
        return Err(KyroError::Validation(format!(
            "media_item id={id} introuvable pour update_media_item"
        )));
    }

    tx.commit().await?;
    tracing::debug!(media_item_id = id, "Kyro: update_media_item committé");
    Ok(())
}

/// Helper testable pour `update_episode_item` (requirements 6.5,
/// 6.3, 17.2).
///
/// Validation préalable :
/// 1. `status` ∈ `{non_vu, en_cours, vu}`.
///
/// Mutation encapsulée dans une transaction unique.
pub async fn update_episode_item_impl(
    repo: &Repo,
    id: i64,
    input: &UpdateEpisodeItemInput,
) -> Result<(), KyroError> {
    let status = parse_status(&input.status)?;

    let mut tx = repo.pool().begin().await?;
    let affected = sqlx::query(
        "UPDATE episode_item \
            SET title = ?, \
                synopsis = ?, \
                status = ? \
          WHERE id = ?",
    )
    .bind(input.title.as_deref())
    .bind(input.synopsis.as_deref())
    .bind(status.as_str())
    .bind(id)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    if affected == 0 {
        tx.rollback().await?;
        return Err(KyroError::Validation(format!(
            "episode_item id={id} introuvable pour update_episode_item"
        )));
    }

    tx.commit().await?;
    tracing::debug!(episode_item_id = id, "Kyro: update_episode_item committé");
    Ok(())
}

// ---------------------------------------------------------------------------
// Façades `#[tauri::command]` — thin wrappers exposés au frontend.
//
// La registration sur `tauri::Builder::invoke_handler` est explicitement
// déléguée à la tâche 12.6.
// ---------------------------------------------------------------------------

/// Liste les Media_Item d'une catégorie (requirement 7.2).
#[tauri::command]
pub async fn list_by_category(
    state: tauri::State<'_, Repo>,
    category: String,
) -> Result<Vec<MediaItemDto>, KyroError> {
    list_by_category_impl(state.inner(), &category).await
}

/// Recherche par titre / titre original, insensible à la casse et aux
/// accents (requirement 13.2).
#[tauri::command]
pub async fn search(
    state: tauri::State<'_, Repo>,
    query: String,
) -> Result<Vec<MediaItemDto>, KyroError> {
    search_impl(state.inner(), &query).await
}

/// Liste les Media_Item marqués `favorite = 1` (requirement 11.4).
#[tauri::command]
pub async fn list_favorites(
    state: tauri::State<'_, Repo>,
) -> Result<Vec<MediaItemDto>, KyroError> {
    list_favorites_impl(state.inner()).await
}

/// Liste les Media_Item marqués `to_watch = 1` (requirement 11.5).
#[tauri::command]
pub async fn list_to_watch(
    state: tauri::State<'_, Repo>,
) -> Result<Vec<MediaItemDto>, KyroError> {
    list_to_watch_impl(state.inner()).await
}

/// Récupère la fiche complète d'un Media_Item (requirement 6.1).
#[tauri::command]
pub async fn get_media_item(
    state: tauri::State<'_, Repo>,
    id: i64,
) -> Result<MediaItemDetailDto, KyroError> {
    get_media_item_impl(state.inner(), id).await
}

/// Liste les Episode_Item d'une série triés (requirements 7.3, 7.4).
#[tauri::command]
pub async fn list_episodes(
    state: tauri::State<'_, Repo>,
    series_id: i64,
) -> Result<Vec<EpisodeItemDto>, KyroError> {
    list_episodes_impl(state.inner(), series_id).await
}

/// Met à jour les champs éditables d'un Media_Item (requirement 6.1,
/// 6.3, 6.6).
#[tauri::command]
pub async fn update_media_item(
    state: tauri::State<'_, Repo>,
    id: i64,
    input: UpdateMediaItemInput,
) -> Result<(), KyroError> {
    update_media_item_impl(state.inner(), id, &input).await
}

/// Met à jour les champs éditables d'un Episode_Item (requirement
/// 6.5, 6.3).
#[tauri::command]
pub async fn update_episode_item(
    state: tauri::State<'_, Repo>,
    id: i64,
    input: UpdateEpisodeItemInput,
) -> Result<(), KyroError> {
    update_episode_item_impl(state.inner(), id, &input).await
}

/// Bascule le drapeau `favorite` (requirement 11.2).
#[tauri::command]
pub async fn toggle_favorite(
    state: tauri::State<'_, Repo>,
    id: i64,
) -> Result<bool, KyroError> {
    toggle_favorite_impl(state.inner(), id).await
}

/// Bascule le drapeau `to_watch` (requirement 11.3).
#[tauri::command]
pub async fn toggle_to_watch(
    state: tauri::State<'_, Repo>,
    id: i64,
) -> Result<bool, KyroError> {
    toggle_to_watch_impl(state.inner(), id).await
}

/// Force manuellement le statut d'un Media_Item ou Episode_Item
/// (requirement 8.4).
#[tauri::command]
pub async fn force_status(
    state: tauri::State<'_, Repo>,
    target_kind: TargetKind,
    target_id: i64,
    status: String,
) -> Result<(), KyroError> {
    force_status_impl(state.inner(), target_kind, target_id, &status).await
}

/// Met à jour la progression de lecture (requirements 8.1–8.3, 9.4).
#[tauri::command]
pub async fn set_watch_progress(
    state: tauri::State<'_, Repo>,
    target_kind: TargetKind,
    target_id: i64,
    position: u32,
) -> Result<String, KyroError> {
    set_watch_progress_impl(state.inner(), target_kind, target_id, position).await
}

/// Importe une affiche locale pour un Media_Item (requirements 6.2,
/// 6.3).
#[tauri::command]
pub async fn import_poster_for_media(
    state: tauri::State<'_, Repo>,
    media_id: i64,
    source_path: String,
    posters_dir: String,
) -> Result<String, KyroError> {
    import_poster_for_media_impl(
        state.inner(),
        media_id,
        std::path::Path::new(&source_path),
        std::path::Path::new(&posters_dir),
    )
    .await
}

/// Importe une affiche locale pour un Episode_Item (requirements
/// 6.2, 6.3, 6.5).
#[tauri::command]
pub async fn import_poster_for_episode(
    state: tauri::State<'_, Repo>,
    episode_id: i64,
    source_path: String,
    posters_dir: String,
) -> Result<String, KyroError> {
    import_poster_for_episode_impl(
        state.inner(),
        episode_id,
        std::path::Path::new(&source_path),
        std::path::Path::new(&posters_dir),
    )
    .await
}

/// Calcule l'épisode à reprendre dans une série (requirement 12.2).
#[tauri::command]
pub async fn next_resume_episode(
    state: tauri::State<'_, Repo>,
    series_id: i64,
) -> Result<Option<i64>, KyroError> {
    next_resume_episode_impl(state.inner(), series_id).await
}

// Suppress unused warning for `MediaType` — the type is referenced
// indirectly via `MediaItem.media_type.as_str()` in `From<MediaItem>
// for MediaItemDto`. Re-export ensures the path stays available for
// future helpers without re-importing.
#[allow(dead_code)]
const _ASSERT_MEDIA_TYPE_USED: fn() -> &'static str = || MediaType::Film.as_str();

// ---------------------------------------------------------------------------
// Tests unitaires.
//
// Ils exercent les `*_impl` directement avec un `Repo` réel construit
// sur un pool SQLite temporaire — on évite ainsi `tauri::AppHandle`
// tout en validant la chaîne complète : sérialisation domain → DTO,
// validation des entrées, et propagation au repo.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{
        SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions,
        SqliteSynchronous,
    };
    use std::path::Path;
    use tempfile::TempDir;

    fn block_on<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("création d'un runtime tokio current_thread")
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
            .expect("ouverture du pool SQLite de test");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("exécution des migrations sur la DB de test");
        pool
    }

    /// Insère un media_item Film minimal et retourne son id.
    async fn insert_film(pool: &SqlitePool, title: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(
            "INSERT INTO media_item (type, title, available) \
             VALUES ('Film', ?, 1) RETURNING id",
        )
        .bind(title)
        .fetch_one(pool)
        .await
        .expect("insertion film")
    }

    /// Insère un media_item Series + un épisode rattaché.
    async fn insert_series_with_episode(
        pool: &SqlitePool,
        title: &str,
        season: i64,
        episode: i64,
        file_path: &str,
    ) -> (i64, i64) {
        let series_id = sqlx::query_scalar::<_, i64>(
            "INSERT INTO media_item (type, title, available) \
             VALUES ('Series', ?, 1) RETURNING id",
        )
        .bind(title)
        .fetch_one(pool)
        .await
        .expect("insertion série");

        let ep_id = sqlx::query_scalar::<_, i64>(
            "INSERT INTO episode_item ( \
                 series_id, season_number, episode_number, file_path, available \
             ) VALUES (?, ?, ?, ?, 1) RETURNING id",
        )
        .bind(series_id)
        .bind(season)
        .bind(episode)
        .bind(file_path)
        .fetch_one(pool)
        .await
        .expect("insertion épisode");

        (series_id, ep_id)
    }

    // -----------------------------------------------------------------
    // Validation des entrées (requirements 6.6, 4.2).
    // -----------------------------------------------------------------

    #[test]
    fn validate_year_accepts_none_and_in_range() {
        assert!(validate_year(None).is_ok());
        assert!(validate_year(Some(1900)).is_ok());
        assert!(validate_year(Some(2099)).is_ok());
        assert!(validate_year(Some(1995)).is_ok());
    }

    #[test]
    fn validate_year_rejects_below_1900() {
        let err = validate_year(Some(1899)).expect_err("doit refuser 1899");
        match err {
            KyroError::Validation(msg) => {
                assert!(msg.contains("1899") && msg.contains("1900") && msg.contains("2099"));
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn validate_year_rejects_above_2099() {
        let err = validate_year(Some(2100)).expect_err("doit refuser 2100");
        assert!(matches!(err, KyroError::Validation(_)));
    }

    #[test]
    fn validate_rating_accepts_none_and_in_range() {
        assert!(validate_rating(None).is_ok());
        for r in 0..=10 {
            assert!(validate_rating(Some(r)).is_ok(), "{r} doit être accepté");
        }
    }

    #[test]
    fn validate_rating_rejects_negative() {
        let err = validate_rating(Some(-1)).expect_err("doit refuser -1");
        match err {
            KyroError::Validation(msg) => {
                assert!(msg.contains("-1") && msg.contains("0") && msg.contains("10"));
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[test]
    fn validate_rating_rejects_above_ten() {
        let err = validate_rating(Some(11)).expect_err("doit refuser 11");
        assert!(matches!(err, KyroError::Validation(_)));
    }

    #[test]
    fn parse_category_accepts_canonical_values() {
        assert_eq!(parse_category("Films").unwrap(), Category::Films);
        assert_eq!(parse_category("Series").unwrap(), Category::Series);
        assert_eq!(parse_category("Animes").unwrap(), Category::Animes);
    }

    #[test]
    fn parse_category_rejects_unknown_value() {
        let err = parse_category("Movies").expect_err("Movies doit être refusé");
        assert!(matches!(err, KyroError::Validation(_)));
    }

    #[test]
    fn parse_status_accepts_canonical_values() {
        assert_eq!(parse_status("non_vu").unwrap(), Status::NonVu);
        assert_eq!(parse_status("en_cours").unwrap(), Status::EnCours);
        assert_eq!(parse_status("vu").unwrap(), Status::Vu);
    }

    #[test]
    fn parse_status_rejects_unknown_value() {
        let err = parse_status("watched").expect_err("watched doit être refusé");
        assert!(matches!(err, KyroError::Validation(_)));
    }

    // -----------------------------------------------------------------
    // list_by_category / search / list_favorites / list_to_watch.
    // -----------------------------------------------------------------

    #[test]
    fn list_by_category_returns_only_matching_type() {
        block_on(async {
            let dir = TempDir::new().unwrap();
            let pool = fresh_pool(dir.path()).await;
            let repo = Repo::new(pool.clone());

            insert_film(&pool, "Inception").await;
            // `list_by_category` masque désormais les séries sans
            // épisode disponible (cf. doc), donc on attache un épisode
            // dispo à « Breaking Bad » pour qu'elle reste visible.
            insert_series_with_episode(&pool, "Breaking Bad", 1, 1, "C:/series/bb-s01e01.mkv")
                .await;

            let films = list_by_category_impl(&repo, "Films")
                .await
                .expect("list films");
            assert_eq!(films.len(), 1);
            assert_eq!(films[0].title, "Inception");
            assert_eq!(films[0].media_type, "Film");

            let series = list_by_category_impl(&repo, "Series")
                .await
                .expect("list séries");
            assert_eq!(series.len(), 1);
            assert_eq!(series[0].title, "Breaking Bad");
        });
    }

    #[test]
    fn list_by_category_rejects_unknown_category() {
        block_on(async {
            let dir = TempDir::new().unwrap();
            let pool = fresh_pool(dir.path()).await;
            let repo = Repo::new(pool);

            let err = list_by_category_impl(&repo, "Movies")
                .await
                .expect_err("catégorie inconnue doit être refusée");
            assert!(matches!(err, KyroError::Validation(_)));
        });
    }

    #[test]
    fn search_filters_case_and_accent_insensitive() {
        block_on(async {
            let dir = TempDir::new().unwrap();
            let pool = fresh_pool(dir.path()).await;
            let repo = Repo::new(pool.clone());

            insert_film(&pool, "Amélie").await;
            insert_film(&pool, "Inception").await;

            let hits = search_impl(&repo, "amelie").await.expect("search");
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].title, "Amélie");

            let all = search_impl(&repo, "").await.expect("search vide");
            assert_eq!(all.len(), 2);
        });
    }

    #[test]
    fn list_favorites_and_to_watch_only_return_flagged() {
        block_on(async {
            let dir = TempDir::new().unwrap();
            let pool = fresh_pool(dir.path()).await;
            let repo = Repo::new(pool.clone());

            let id_a = insert_film(&pool, "A").await;
            let _id_b = insert_film(&pool, "B").await;
            let id_c = insert_film(&pool, "C").await;

            sqlx::query("UPDATE media_item SET favorite = 1 WHERE id = ?")
                .bind(id_a)
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("UPDATE media_item SET to_watch = 1 WHERE id = ?")
                .bind(id_c)
                .execute(&pool)
                .await
                .unwrap();

            let favs = list_favorites_impl(&repo).await.expect("favoris");
            assert_eq!(favs.len(), 1);
            assert_eq!(favs[0].id, id_a);

            let tw = list_to_watch_impl(&repo).await.expect("à regarder");
            assert_eq!(tw.len(), 1);
            assert_eq!(tw[0].id, id_c);
        });
    }

    // -----------------------------------------------------------------
    // get_media_item / update_media_item / update_episode_item.
    // -----------------------------------------------------------------

    #[test]
    fn get_media_item_returns_full_detail() {
        block_on(async {
            let dir = TempDir::new().unwrap();
            let pool = fresh_pool(dir.path()).await;
            let repo = Repo::new(pool.clone());

            let id = insert_film(&pool, "Inception").await;
            sqlx::query(
                "UPDATE media_item \
                    SET year = 2010, user_rating = 9, status = 'vu', favorite = 1 \
                  WHERE id = ?",
            )
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();

            let detail = get_media_item_impl(&repo, id).await.expect("get detail");
            assert_eq!(detail.id, id);
            assert_eq!(detail.title, "Inception");
            assert_eq!(detail.year, Some(2010));
            assert_eq!(detail.user_rating, Some(9));
            assert_eq!(detail.status, "vu");
            assert!(detail.favorite);
            assert!(!detail.to_watch);
            assert_eq!(detail.media_type, "Film");
        });
    }

    #[test]
    fn get_media_item_returns_validation_error_for_unknown_id() {
        block_on(async {
            let dir = TempDir::new().unwrap();
            let pool = fresh_pool(dir.path()).await;
            let repo = Repo::new(pool);

            let err = get_media_item_impl(&repo, 99_999)
                .await
                .expect_err("id inconnu doit être refusé");
            assert!(matches!(err, KyroError::Validation(_)));
        });
    }

    #[test]
    fn update_media_item_persists_all_editable_fields() {
        block_on(async {
            let dir = TempDir::new().unwrap();
            let pool = fresh_pool(dir.path()).await;
            let repo = Repo::new(pool.clone());

            let id = insert_film(&pool, "Original Title").await;

            let input = UpdateMediaItemInput {
                title: "New Title".to_string(),
                original_title: Some("Original".to_string()),
                year: Some(2010),
                synopsis: Some("Un synopsis bien fourni.".to_string()),
                studio: Some("Warner".to_string()),
                user_rating: Some(8),
                status: "en_cours".to_string(),
            };
            update_media_item_impl(&repo, id, &input)
                .await
                .expect("update doit réussir");

            let detail = get_media_item_impl(&repo, id).await.expect("re-read");
            assert_eq!(detail.title, "New Title");
            assert_eq!(detail.original_title.as_deref(), Some("Original"));
            assert_eq!(detail.year, Some(2010));
            assert_eq!(detail.synopsis.as_deref(), Some("Un synopsis bien fourni."));
            assert_eq!(detail.studio.as_deref(), Some("Warner"));
            assert_eq!(detail.user_rating, Some(8));
            assert_eq!(detail.status, "en_cours");
        });
    }

    #[test]
    fn update_media_item_rejects_year_out_of_range() {
        block_on(async {
            let dir = TempDir::new().unwrap();
            let pool = fresh_pool(dir.path()).await;
            let repo = Repo::new(pool.clone());

            let id = insert_film(&pool, "T").await;

            let input = UpdateMediaItemInput {
                title: "T".to_string(),
                original_title: None,
                year: Some(1500),
                synopsis: None,
                studio: None,
                user_rating: None,
                status: "non_vu".to_string(),
            };
            let err = update_media_item_impl(&repo, id, &input)
                .await
                .expect_err("année hors plage doit être refusée");
            assert!(matches!(err, KyroError::Validation(_)));

            // La fiche ne doit pas avoir été modifiée.
            let detail = get_media_item_impl(&repo, id).await.unwrap();
            assert_eq!(detail.title, "T");
            assert_eq!(detail.year, None);
        });
    }

    #[test]
    fn update_media_item_rejects_rating_out_of_range() {
        block_on(async {
            let dir = TempDir::new().unwrap();
            let pool = fresh_pool(dir.path()).await;
            let repo = Repo::new(pool.clone());

            let id = insert_film(&pool, "T").await;

            let input = UpdateMediaItemInput {
                title: "T".to_string(),
                original_title: None,
                year: None,
                synopsis: None,
                studio: None,
                user_rating: Some(11),
                status: "non_vu".to_string(),
            };
            let err = update_media_item_impl(&repo, id, &input)
                .await
                .expect_err("note hors plage doit être refusée");
            assert!(matches!(err, KyroError::Validation(_)));
        });
    }

    #[test]
    fn update_media_item_rejects_empty_title() {
        block_on(async {
            let dir = TempDir::new().unwrap();
            let pool = fresh_pool(dir.path()).await;
            let repo = Repo::new(pool.clone());

            let id = insert_film(&pool, "T").await;

            let input = UpdateMediaItemInput {
                title: "   ".to_string(),
                original_title: None,
                year: None,
                synopsis: None,
                studio: None,
                user_rating: None,
                status: "non_vu".to_string(),
            };
            let err = update_media_item_impl(&repo, id, &input)
                .await
                .expect_err("titre vide doit être refusé");
            assert!(matches!(err, KyroError::Validation(_)));
        });
    }

    #[test]
    fn update_media_item_rejects_unknown_status() {
        block_on(async {
            let dir = TempDir::new().unwrap();
            let pool = fresh_pool(dir.path()).await;
            let repo = Repo::new(pool.clone());

            let id = insert_film(&pool, "T").await;

            let input = UpdateMediaItemInput {
                title: "T".to_string(),
                original_title: None,
                year: None,
                synopsis: None,
                studio: None,
                user_rating: None,
                status: "watched".to_string(),
            };
            let err = update_media_item_impl(&repo, id, &input)
                .await
                .expect_err("statut inconnu doit être refusé");
            assert!(matches!(err, KyroError::Validation(_)));
        });
    }

    #[test]
    fn update_media_item_rejects_unknown_id() {
        block_on(async {
            let dir = TempDir::new().unwrap();
            let pool = fresh_pool(dir.path()).await;
            let repo = Repo::new(pool);

            let input = UpdateMediaItemInput {
                title: "T".to_string(),
                original_title: None,
                year: None,
                synopsis: None,
                studio: None,
                user_rating: None,
                status: "non_vu".to_string(),
            };
            let err = update_media_item_impl(&repo, 99_999, &input)
                .await
                .expect_err("id inconnu doit être refusé");
            assert!(matches!(err, KyroError::Validation(_)));
        });
    }

    #[test]
    fn update_episode_item_persists_fields() {
        block_on(async {
            let dir = TempDir::new().unwrap();
            let pool = fresh_pool(dir.path()).await;
            let repo = Repo::new(pool.clone());

            let (_, ep_id) =
                insert_series_with_episode(&pool, "S", 1, 1, "C:/s/S01E01.mkv").await;

            let input = UpdateEpisodeItemInput {
                title: Some("Pilot".to_string()),
                synopsis: Some("Le pilote.".to_string()),
                status: "vu".to_string(),
            };
            update_episode_item_impl(&repo, ep_id, &input)
                .await
                .expect("update épisode");

            let row = sqlx::query("SELECT title, synopsis, status FROM episode_item WHERE id = ?")
                .bind(ep_id)
                .fetch_one(&pool)
                .await
                .unwrap();
            let title: Option<String> = row.try_get("title").unwrap();
            let synopsis: Option<String> = row.try_get("synopsis").unwrap();
            let status: String = row.try_get("status").unwrap();
            assert_eq!(title.as_deref(), Some("Pilot"));
            assert_eq!(synopsis.as_deref(), Some("Le pilote."));
            assert_eq!(status, "vu");
        });
    }

    #[test]
    fn update_episode_item_rejects_unknown_status() {
        block_on(async {
            let dir = TempDir::new().unwrap();
            let pool = fresh_pool(dir.path()).await;
            let repo = Repo::new(pool.clone());

            let (_, ep_id) =
                insert_series_with_episode(&pool, "S", 1, 1, "C:/s/S01E01.mkv").await;

            let input = UpdateEpisodeItemInput {
                title: None,
                synopsis: None,
                status: "watched".to_string(),
            };
            let err = update_episode_item_impl(&repo, ep_id, &input)
                .await
                .expect_err("statut inconnu doit être refusé");
            assert!(matches!(err, KyroError::Validation(_)));
        });
    }

    #[test]
    fn update_episode_item_rejects_unknown_id() {
        block_on(async {
            let dir = TempDir::new().unwrap();
            let pool = fresh_pool(dir.path()).await;
            let repo = Repo::new(pool);

            let input = UpdateEpisodeItemInput {
                title: None,
                synopsis: None,
                status: "non_vu".to_string(),
            };
            let err = update_episode_item_impl(&repo, 99_999, &input)
                .await
                .expect_err("id inconnu doit être refusé");
            assert!(matches!(err, KyroError::Validation(_)));
        });
    }

    // -----------------------------------------------------------------
    // toggle_favorite / toggle_to_watch.
    // -----------------------------------------------------------------

    #[test]
    fn toggle_favorite_flips_state_and_returns_new_value() {
        block_on(async {
            let dir = TempDir::new().unwrap();
            let pool = fresh_pool(dir.path()).await;
            let repo = Repo::new(pool.clone());

            let id = insert_film(&pool, "T").await;

            let after_first = toggle_favorite_impl(&repo, id).await.expect("toggle 1");
            assert!(after_first, "premier toggle doit activer");
            let after_second = toggle_favorite_impl(&repo, id).await.expect("toggle 2");
            assert!(!after_second, "deuxième toggle doit revenir à l'état initial");
        });
    }

    #[test]
    fn toggle_to_watch_flips_state_and_returns_new_value() {
        block_on(async {
            let dir = TempDir::new().unwrap();
            let pool = fresh_pool(dir.path()).await;
            let repo = Repo::new(pool.clone());

            let id = insert_film(&pool, "T").await;

            let after = toggle_to_watch_impl(&repo, id).await.expect("toggle");
            assert!(after);
        });
    }

    // -----------------------------------------------------------------
    // force_status / set_watch_progress.
    // -----------------------------------------------------------------

    #[test]
    fn force_status_writes_value_and_validates_status_string() {
        block_on(async {
            let dir = TempDir::new().unwrap();
            let pool = fresh_pool(dir.path()).await;
            let repo = Repo::new(pool.clone());

            let id = insert_film(&pool, "T").await;
            force_status_impl(&repo, TargetKind::Media, id, "vu")
                .await
                .expect("force_status doit réussir");

            let detail = get_media_item_impl(&repo, id).await.unwrap();
            assert_eq!(detail.status, "vu");

            let err = force_status_impl(&repo, TargetKind::Media, id, "WATCHED")
                .await
                .expect_err("statut inconnu doit être refusé");
            assert!(matches!(err, KyroError::Validation(_)));
        });
    }

    #[test]
    fn set_watch_progress_returns_recomputed_status() {
        block_on(async {
            let dir = TempDir::new().unwrap();
            let pool = fresh_pool(dir.path()).await;
            let repo = Repo::new(pool.clone());

            let id = sqlx::query_scalar::<_, i64>(
                "INSERT INTO media_item (type, title, duration_seconds, available) \
                 VALUES ('Film', 'T', 100, 1) RETURNING id",
            )
            .fetch_one(&pool)
            .await
            .unwrap();

            // Progression à 95 % : statut Vu.
            let new_status = set_watch_progress_impl(&repo, TargetKind::Media, id, 95)
                .await
                .expect("set progress");
            assert_eq!(new_status, "vu");

            // Progression à 50 % : statut EnCours.
            let new_status = set_watch_progress_impl(&repo, TargetKind::Media, id, 50)
                .await
                .expect("set progress");
            assert_eq!(new_status, "en_cours");
        });
    }

    // -----------------------------------------------------------------
    // list_episodes / next_resume_episode.
    // -----------------------------------------------------------------

    #[test]
    fn list_episodes_returns_sorted_dtos() {
        block_on(async {
            let dir = TempDir::new().unwrap();
            let pool = fresh_pool(dir.path()).await;
            let repo = Repo::new(pool.clone());

            let (series_id, _) =
                insert_series_with_episode(&pool, "S", 2, 5, "C:/s/S02E05.mkv").await;
            // Ajouter d'autres épisodes dans un ordre arbitraire.
            sqlx::query(
                "INSERT INTO episode_item ( \
                     series_id, season_number, episode_number, file_path, available \
                 ) VALUES (?, 1, 1, 'C:/s/S01E01.mkv', 1), \
                          (?, 2, 1, 'C:/s/S02E01.mkv', 1)",
            )
            .bind(series_id)
            .bind(series_id)
            .execute(&pool)
            .await
            .unwrap();

            let eps = list_episodes_impl(&repo, series_id).await.expect("list");
            assert_eq!(eps.len(), 3);
            // Tri (saison, épisode) ASC.
            assert_eq!((eps[0].season_number, eps[0].episode_number), (1, 1));
            assert_eq!((eps[1].season_number, eps[1].episode_number), (2, 1));
            assert_eq!((eps[2].season_number, eps[2].episode_number), (2, 5));
        });
    }

    #[test]
    fn next_resume_episode_returns_first_unseen_id() {
        block_on(async {
            let dir = TempDir::new().unwrap();
            let pool = fresh_pool(dir.path()).await;
            let repo = Repo::new(pool.clone());

            let (series_id, ep_id) =
                insert_series_with_episode(&pool, "S", 1, 1, "C:/s/S01E01.mkv").await;

            let resume = next_resume_episode_impl(&repo, series_id)
                .await
                .expect("next resume");
            assert_eq!(resume, Some(ep_id));
        });
    }

    // -----------------------------------------------------------------
    // import_poster_for_media / import_poster_for_episode.
    // -----------------------------------------------------------------

    #[test]
    fn import_poster_for_media_validates_extension_and_persists() {
        block_on(async {
            let dir = TempDir::new().unwrap();
            let scratch = TempDir::new().unwrap();
            let posters = TempDir::new().unwrap();
            let pool = fresh_pool(dir.path()).await;
            let repo = Repo::new(pool.clone());

            let id = insert_film(&pool, "T").await;
            let src = scratch.path().join("p.jpg");
            std::fs::write(&src, b"img").unwrap();

            let dest = import_poster_for_media_impl(&repo, id, &src, posters.path())
                .await
                .expect("import doit réussir");
            assert!(dest.contains("media-"));
            assert!(dest.ends_with(".jpg"));

            // Extension non supportée → refus.
            let bad = scratch.path().join("p.bmp");
            std::fs::write(&bad, b"img").unwrap();
            let err = import_poster_for_media_impl(&repo, id, &bad, posters.path())
                .await
                .expect_err("bmp doit être refusé");
            assert!(matches!(err, KyroError::Validation(_)));
        });
    }

    #[test]
    fn import_poster_for_episode_works_for_episode_target() {
        block_on(async {
            let dir = TempDir::new().unwrap();
            let scratch = TempDir::new().unwrap();
            let posters = TempDir::new().unwrap();
            let pool = fresh_pool(dir.path()).await;
            let repo = Repo::new(pool.clone());

            let (_, ep_id) =
                insert_series_with_episode(&pool, "S", 1, 1, "C:/s/S01E01.mkv").await;
            let src = scratch.path().join("ep.png");
            std::fs::write(&src, b"img").unwrap();

            let dest = import_poster_for_episode_impl(&repo, ep_id, &src, posters.path())
                .await
                .expect("import doit réussir");
            assert!(dest.contains("episode-"));
            assert!(dest.ends_with(".png"));
        });
    }

    // -----------------------------------------------------------------
    // DTO sérialisation (contrat IPC).
    // -----------------------------------------------------------------

    #[test]
    fn target_kind_serializes_to_snake_case() {
        assert_eq!(
            serde_json::to_string(&TargetKind::Media).unwrap(),
            "\"media\""
        );
        assert_eq!(
            serde_json::to_string(&TargetKind::Episode).unwrap(),
            "\"episode\""
        );
        assert_eq!(
            serde_json::from_str::<TargetKind>("\"media\"").unwrap(),
            TargetKind::Media
        );
    }

    #[test]
    fn media_item_dto_uses_type_field_for_media_type() {
        let dto = MediaItemDto {
            id: 1,
            title: "T".to_string(),
            original_title: None,
            year: Some(2020),
            media_type: "Film".to_string(),
            status: "non_vu".to_string(),
            genres: vec!["Drame".to_string()],
            poster_path: None,
        };
        let json = serde_json::to_string(&dto).unwrap();
        assert!(json.contains("\"type\":\"Film\""));
        assert!(!json.contains("\"media_type\""));
    }
}
