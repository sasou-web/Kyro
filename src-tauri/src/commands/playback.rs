//! Commandes `#[tauri::command]` du domaine **lecture** (Kyro_Player_Bridge).
//!
//! Ce module rassemble les commandes Tauri du parcours « démarrage de
//! la lecture » (tâches 11.2, 12.3) :
//!
//! * [`install_lua_script`] (tâche 11.2, requirement 9.7) — installe
//!   `kyro.lua` dans `%APPDATA%\mpv.net\scripts\`.
//! * [`play_media`] (tâche 12.3, requirements 9.1, 12.4) — déclenche la
//!   lecture d'un Media_Item ou d'un Episode_Item via le lecteur
//!   externe configuré, reprend à la position enregistrée et raccorde,
//!   pour mpv.net, le canal IPC `Kyro_IPC_Pipe`.
//! * [`play_resume`] (tâche 12.3, requirements 12.2, 12.3) — délègue
//!   à `Repo::next_resume_episode(series_id)` puis à [`play_media`].
//! * [`play_rewatch_from_start`] (tâche 12.3, requirement 12.3) — remet
//!   à 0 la progression de tous les épisodes d'une série puis joue le
//!   premier épisode dans l'ordre `(season_number, episode_number)`.
//!
//! La registration des commandes sur le `tauri::Builder`
//! (`invoke_handler`) est explicitement déléguée à la tâche 12.x — ce
//! module se contente d'exposer les fonctions.
//!
//! # Architecture
//!
//! ## `install_lua_script`
//!
//! Scindée en deux niveaux pour rester testable sans
//! `tauri::AppHandle` :
//!
//! * [`install_lua_script_to`] — helper *pure-IO* qui prend un
//!   `dest_dir: &Path` arbitraire et exécute la séquence
//!   `create_dir_all` → `write` → renvoi du `PathBuf` final. C'est ce
//!   helper que ciblent les tests unitaires (via `tempfile::TempDir`).
//! * [`install_lua_script`] — `#[tauri::command]` qui résout le
//!   répertoire `%APPDATA%\mpv.net\scripts` puis délègue au helper.
//!
//! Le contenu du script Lua est embarqué à la compilation par
//! `include_bytes!` afin d'être bundlé dans le binaire et survivre à
//! n'importe quelle disposition de fichiers à l'exécution (requirement
//! 9.7 + design « binaires & ressources embarquées »).
//!
//! ## `play_media` / `play_resume` / `play_rewatch_from_start`
//!
//! La logique est factorisée en helpers `pub(crate)` (sans handle
//! `tauri::Window`) afin que les tests unitaires puissent exercer la
//! lecture de la cible (`load_target_info`), la résolution de la
//! cible de reprise (déjà testée côté `Repo::next_resume_episode`),
//! et la remise à zéro des épisodes (`reset_all_episodes_progress`)
//! sans engager le spawn d'un véritable processus mpv.net. Le
//! lancement effectif (`launch_player`, `ipc_pipe::listen`,
//! `run_ipc_session_emit`) est testé bout-en-bout par la tâche 17.2.

use std::path::{Path, PathBuf};

use tauri::Emitter;

use crate::domain::types::{EpisodeItemId, MediaItemId};
use crate::error::KyroError;
use crate::infra::ipc_pipe;
use crate::infra::player_bridge::{
    self, EVENT_EXTERNAL_PLAYER_MISSING, EVENT_EXTERNAL_PLAYER_NON_MPV,
};
use crate::infra::repo::{ProgressTarget, Repo};
use crate::infra::settings_keys::SETTING_KEY_EXTERNAL_PLAYER_PATH;

// =====================================================================
// Tâche 11.2 — installation du script Lua mpv.net.
// =====================================================================

/// Contenu binaire du script Lua, embarqué à la compilation pour qu'il
/// soit toujours présent dans le binaire publié — peu importe la
/// disposition des ressources sur la machine de l'utilisateur final.
const KYRO_LUA_BYTES: &[u8] = include_bytes!("../../resources/kyro.lua");

/// Nom canonique du fichier installé dans le dossier `scripts/` de
/// mpv.net. mpv charge automatiquement les `.lua` présents dans ce
/// répertoire au démarrage (cf. design § Kyro_Lua_Script).
const KYRO_LUA_FILENAME: &str = "kyro.lua";

/// Sous-chemin relatif au répertoire `%APPDATA%` (Roaming) où mpv.net
/// recherche ses scripts utilisateur (requirement 9.7).
const MPV_NET_SCRIPTS_SUBDIR: &[&str] = &["mpv.net", "scripts"];

/// Helper pure-IO : installe `kyro.lua` dans `dest_dir`.
///
/// Étapes :
/// 1. Crée `dest_dir` récursivement si manquant
///    (`std::fs::create_dir_all` — idempotent).
/// 2. Écrit `KYRO_LUA_BYTES` dans `dest_dir/kyro.lua` — `std::fs::write`
///    tronque/écrase la cible si elle existe déjà (requirement 9.7
///    « écraser si déjà présent »).
/// 3. Renvoie le `PathBuf` du fichier écrit.
///
/// Toutes les erreurs d'IO sont remontées comme `KyroError::Io` via la
/// conversion `From<std::io::Error>` déjà en place.
///
/// Exposé `pub(crate)` pour permettre aux tests unitaires de cibler une
/// destination temporaire (`tempfile::TempDir`) sans avoir besoin d'un
/// `tauri::AppHandle` ni d'écrire dans le vrai `%APPDATA%`.
pub(crate) fn install_lua_script_to(dest_dir: &Path) -> Result<PathBuf, KyroError> {
    std::fs::create_dir_all(dest_dir)?;
    let dest_file = dest_dir.join(KYRO_LUA_FILENAME);
    std::fs::write(&dest_file, KYRO_LUA_BYTES)?;
    Ok(dest_file)
}

/// Résout le répertoire `%APPDATA%\mpv.net\scripts` (Roaming AppData
/// Windows). Préfère la variable d'environnement `APPDATA` — c'est la
/// source canonique côté Windows pour le profil utilisateur courant et
/// elle est honorée par mpv.net lui-même — puis retombe sur
/// `dirs::config_dir()` si la variable n'est pas définie (cas tests /
/// environnements minimaux).
fn resolve_mpv_net_scripts_dir() -> Result<PathBuf, KyroError> {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(dirs::config_dir)
        .ok_or_else(|| {
            KyroError::Config(
                "impossible de résoudre %APPDATA% (mpv.net\\scripts)".to_string(),
            )
        })?;

    let mut dir = base;
    for segment in MPV_NET_SCRIPTS_SUBDIR {
        dir.push(segment);
    }
    Ok(dir)
}

/// Commande Tauri exposée au frontend (paramètres) : installe
/// `kyro.lua` dans `%APPDATA%\mpv.net\scripts\` et retourne le chemin
/// effectivement écrit (sous forme de `String` pour passer le pont IPC
/// Tauri sans surcoût de sérialisation).
///
/// _Requirements: 9.7_
#[tauri::command]
pub async fn install_lua_script() -> Result<String, KyroError> {
    let dest_dir = resolve_mpv_net_scripts_dir()?;
    let written = install_lua_script_to(&dest_dir)?;
    Ok(written.to_string_lossy().into_owned())
}

// =====================================================================
// Tâche 12.3 — commandes Tauri de lecture.
// =====================================================================

/// Discriminant `kind` de [`play_media`] : la commande accepte deux
/// formes de cibles, alignées sur l'enum [`ProgressTarget`] côté infra.
///
/// Les littéraux acceptés côté IPC sont `"media"` (pour un Film ou la
/// racine d'une Série/Anime — usage en pratique : Film) et `"episode"`
/// (pour un Episode_Item d'une Série/Anime). Toute autre valeur produit
/// une [`KyroError::Validation`] explicite avant tout accès DB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlaybackKind {
    Media,
    Episode,
}

impl PlaybackKind {
    /// Parse le littéral textuel reçu via IPC. Comparaison
    /// case-sensitive : le frontend a un contrat fixe (cf. design §
    /// « Modules backend Rust »). Toute valeur inconnue est refusée.
    pub(crate) fn parse(kind: &str) -> Result<Self, KyroError> {
        match kind {
            "media" => Ok(Self::Media),
            "episode" => Ok(Self::Episode),
            other => Err(KyroError::Validation(format!(
                "kind de lecture inconnu: {other:?} (attendu \"media\" ou \"episode\")"
            ))),
        }
    }
}

/// Informations chargées depuis la DB pour préparer un `play_media`.
///
/// * `target` — discriminant typé `(Media|Episode, id)` consommé par
///   `Repo::set_watch_progress` à travers [`run_ipc_session_emit`].
/// * `file_path` — chemin absolu du fichier média à passer au lecteur.
/// * `start_seconds` — position de reprise (`watch_progress_seconds`)
///   convertie en `u32` pour `--start=` (requirement 9.1, 12.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetInfo {
    pub target: ProgressTarget,
    pub file_path: String,
    pub start_seconds: u32,
}

/// Charge `(file_path, watch_progress_seconds)` pour une cible
/// `(target_id, kind)` et projette le résultat en [`TargetInfo`].
///
/// Erreurs :
/// * la fiche n'existe pas → [`KyroError::Validation`] citant le couple
///   `(kind, id)` ;
/// * la fiche existe mais `file_path` est NULL (cas typique : série
///   placeholder « À classer » côté `media_item` — tâche 7.x) →
///   [`KyroError::Validation`] explicite, car on ne peut pas la jouer.
///
/// La conversion `i64 → u32` clamp à 0 sur valeur négative
/// (théoriquement impossible : la colonne est non-signée applicativement,
/// mais on préfère un comportement défensif à un panic).
pub(crate) async fn load_target_info(
    repo: &Repo,
    target_id: i64,
    kind: PlaybackKind,
) -> Result<TargetInfo, KyroError> {
    match kind {
        PlaybackKind::Media => {
            let row: Option<(Option<String>, i64)> = sqlx::query_as(
                "SELECT file_path, watch_progress_seconds \
                   FROM media_item WHERE id = ?",
            )
            .bind(target_id)
            .fetch_optional(repo.pool())
            .await?;
            let (file_path_opt, wp) = row.ok_or_else(|| {
                KyroError::Validation(format!(
                    "media_item id={target_id} introuvable pour play_media"
                ))
            })?;
            let file_path = file_path_opt.ok_or_else(|| {
                KyroError::Validation(format!(
                    "media_item id={target_id} sans file_path (placeholder ?), \
                     lecture impossible"
                ))
            })?;
            Ok(TargetInfo {
                target: ProgressTarget::Media(MediaItemId(target_id)),
                file_path,
                start_seconds: u32::try_from(wp).unwrap_or(0),
            })
        }
        PlaybackKind::Episode => {
            // `episode_item.file_path` est `UNIQUE NOT NULL` côté DDL,
            // pas besoin d'absorber un Option<String>.
            let row: Option<(String, i64)> = sqlx::query_as(
                "SELECT file_path, watch_progress_seconds \
                   FROM episode_item WHERE id = ?",
            )
            .bind(target_id)
            .fetch_optional(repo.pool())
            .await?;
            let (file_path, wp) = row.ok_or_else(|| {
                KyroError::Validation(format!(
                    "episode_item id={target_id} introuvable pour play_media"
                ))
            })?;
            Ok(TargetInfo {
                target: ProgressTarget::Episode(EpisodeItemId(target_id)),
                file_path,
                start_seconds: u32::try_from(wp).unwrap_or(0),
            })
        }
    }
}

/// Remet à 0 la progression de **tous** les épisodes d'une série, dans
/// l'ordre `(season_number, episode_number)` ASC, et retourne la liste
/// triée. Helper consommé par [`play_rewatch_from_start`].
///
/// Chaque épisode est traité indépendamment via
/// [`Repo::set_watch_progress`] (qui recalcule le statut → `NonVu`
/// puisque la position est 0 — cf. tests `force_status` /
/// `set_watch_progress` côté repo). Une erreur sur un épisode propage
/// immédiatement et abandonne les épisodes restants : conserver un
/// état partiellement remis à zéro serait pire pour l'utilisateur que
/// de remonter l'erreur.
///
/// Retourne la liste ordonnée des `EpisodeItemId` traités, ce qui
/// permet à l'appelant de piocher directement le **premier** id pour
/// le `play_media` qui suit (sans relancer un `list_episodes`).
///
/// Le scan est limité à la série donnée et utilise l'index composite
/// `(series_id, season_number, episode_number)` pour le tri (cf. doc
/// de `Repo::list_episodes`).
pub(crate) async fn reset_all_episodes_progress(
    repo: &Repo,
    series_id: MediaItemId,
) -> Result<Vec<EpisodeItemId>, KyroError> {
    let episodes = repo.list_episodes(series_id).await?;
    let ids: Vec<EpisodeItemId> = episodes.iter().map(|e| e.id).collect();
    for id in &ids {
        repo.set_watch_progress(ProgressTarget::Episode(*id), 0).await?;
    }
    Ok(ids)
}

/// Lance la lecture d'un Media_Item ou d'un Episode_Item via le lecteur
/// externe configuré.
///
/// Étapes :
/// 1. **Setting `external_player_path`** — lu via `Repo::get_setting` ;
///    absent → [`KyroError::Validation`] (`requirement 10.1`).
/// 2. **Validation existence + `.exe`** —
///    [`player_bridge::validate_external_player_path`] ; si l'erreur est
///    une `Validation`, on émet [`EVENT_EXTERNAL_PLAYER_MISSING`] vers
///    le frontend pour qu'il affiche un avis explicite, puis on
///    retourne l'erreur (requirements 10.3, 10.4).
/// 3. **Chargement de la cible** — `(file_path, watch_progress_seconds)`
///    via [`load_target_info`].
/// 4. **Lancement** — [`player_bridge::launch_player`] route entre la
///    branche mpv.net (IPC + `--start=` + `--script-opts=`) et la
///    branche lecteur tiers (chemin du fichier seul) selon le nom de
///    l'exécutable.
/// 5. **Branche `MpvNet`** — on attache [`ipc_pipe::listen`] sur le
///    `pipe_name` de la session puis on spawn une task
///    `run_ipc_session_emit` qui consomme le receiver, persiste la
///    progression au fil des messages NDJSON et émet
///    `playback-ended` à la fin de la session (requirements 9.3, 9.4).
/// 6. **Branche `External`** — pas d'IPC, pas de pipe ; on émet
///    [`EVENT_EXTERNAL_PLAYER_NON_MPV`] pour que le frontend affiche le
///    badge « reprise exacte non garantie » sur la fiche du
///    Media_Item courant (requirement 10.5).
/// 7. **Retour immédiat** — la commande ne bloque pas : la session
///    IPC continue dans la task spawnée, le frontend écoute les
///    événements `playback-ended` / `external-player-non-mpv`.
///
/// _Requirements: 9.1, 9.3, 9.4, 10.3, 10.4, 10.5, 12.4_
#[tauri::command]
pub async fn play_media(
    window: tauri::Window,
    repo: tauri::State<'_, Repo>,
    target_id: i64,
    kind: String,
) -> Result<(), KyroError> {
    let kind = PlaybackKind::parse(&kind)?;

    // 1. Lecture du chemin External_Player.
    let player_path_str = repo
        .get_setting(SETTING_KEY_EXTERNAL_PLAYER_PATH)
        .await?
        .ok_or_else(|| {
            KyroError::Validation(
                "le lecteur externe n'est pas configuré (paramètres → \
                 lecteur externe)"
                    .to_string(),
            )
        })?;
    let player_path = PathBuf::from(&player_path_str);

    // 2. Validation existence + extension. Sur échec de validation,
    //    émission `external-player-missing` puis propagation de
    //    l'erreur (best-effort sur l'émission — la fenêtre peut être
    //    fermée juste avant, on ne masque pas l'erreur d'origine).
    if let Err(err) = player_bridge::validate_external_player_path(&player_path) {
        if let Err(emit_err) = window.emit(EVENT_EXTERNAL_PLAYER_MISSING, &player_path_str)
        {
            tracing::warn!(
                event = EVENT_EXTERNAL_PLAYER_MISSING,
                error = %emit_err,
                "Kyro playback: échec émission external-player-missing \
                 (best-effort, ignoré)"
            );
        }
        return Err(err);
    }

    // 3. Chargement de la cible (file_path + position de reprise).
    let info = load_target_info(repo.inner(), target_id, kind).await?;
    let file_path_buf = PathBuf::from(&info.file_path);

    // 4. Lancement (route mpv.net vs lecteur tiers selon le nom de
    //    l'exécutable).
    //
    //    Pour mpv.net, **on crée le named pipe AVANT de spawn mpv.net**
    //    (sinon le script Lua, lancé immédiatement par mpv, tente
    //    d'ouvrir le pipe avant qu'il n'existe côté serveur, échoue
    //    silencieusement, et plus aucune progression n'est jamais
    //    transmise — bug observé le 17/05).
    //
    //    Pour les autres lecteurs (External), on délègue directement
    //    sans créer de pipe.
    use crate::infra::player_bridge::PlayerKind;
    let kind = player_bridge::branch(&player_path);
    match kind {
        PlayerKind::MpvNet => {
            // 4.a — Génère la session, ouvre le pipe server, *puis*
            // spawn mpv.net avec --script-opts pointant sur ce pipe.
            let session = player_bridge::new_session();

            let (rx, _read_handle) = match ipc_pipe::listen(&session.pipe_name) {
                Ok(handles) => handles,
                Err(err) => {
                    tracing::error!(
                        pipe = %session.pipe_name,
                        error = %err,
                        "Kyro playback: échec création named pipe IPC"
                    );
                    return Err(KyroError::Io(err));
                }
            };

            let _child = match player_bridge::launch_mpv_net(
                &player_path,
                &file_path_buf,
                info.start_seconds,
                &session,
            )
            .await
            {
                Ok(c) => c,
                Err(err) => return Err(KyroError::Io(err)),
            };

            let repo_for_task = repo.inner().clone();
            let window_for_task = window.clone();
            let target = info.target;

            tauri::async_runtime::spawn(async move {
                if let Err(err) = player_bridge::run_ipc_session_emit(
                    &window_for_task,
                    &repo_for_task,
                    target,
                    rx,
                )
                .await
                {
                    tracing::warn!(
                        error = %err,
                        "Kyro playback: session IPC mpv terminée avec erreur"
                    );
                }
            });

            tracing::info!(
                target = ?target,
                pipe = %session.pipe_name,
                start_seconds = info.start_seconds,
                "Kyro playback: mpv.net lancé, session IPC attachée"
            );
        }
        PlayerKind::External => {
            // 4.b — Pas d'IPC. Spawn direct, puis émission de
            // `external-player-non-mpv` pour que le frontend rende le
            // badge « reprise exacte non garantie ».
            let _child = match player_bridge::launch_external_player(
                &player_path,
                &file_path_buf,
            )
            .await
            {
                Ok(c) => c,
                Err(err) => return Err(KyroError::Io(err)),
            };

            if let Err(emit_err) =
                window.emit(EVENT_EXTERNAL_PLAYER_NON_MPV, &info.file_path)
            {
                tracing::warn!(
                    event = EVENT_EXTERNAL_PLAYER_NON_MPV,
                    error = %emit_err,
                    "Kyro playback: échec émission external-player-non-mpv \
                     (best-effort, ignoré)"
                );
            }
            tracing::info!(
                target = ?info.target,
                file_path = %info.file_path,
                "Kyro playback: lecteur tiers lancé (pas d'IPC, badge UI activé)"
            );
        }
    }

    Ok(())
}

/// Reprend la lecture d'une série : résout l'épisode cible via
/// `Repo::next_resume_episode` (premier `EnCours` ou premier `NonVu`
/// selon l'ordre `(season, episode)` ASC) puis délègue à
/// [`play_media`] avec `kind = "episode"`.
///
/// Si la série n'a aucun épisode disponible (tous indisponibles ou
/// tous vus dans une fenêtre sans premier épisode jouable), on
/// retourne [`KyroError::Validation`] : il revient au frontend
/// d'afficher « Revoir depuis le début » via
/// [`play_rewatch_from_start`] dans ce cas (requirement 12.3).
///
/// _Requirements: 12.2, 12.3_
#[tauri::command]
pub async fn play_resume(
    window: tauri::Window,
    repo: tauri::State<'_, Repo>,
    series_id: i64,
) -> Result<(), KyroError> {
    let next = repo
        .next_resume_episode(MediaItemId(series_id))
        .await?
        .ok_or_else(|| {
            KyroError::Validation(format!(
                "série id={series_id} sans épisode jouable pour la reprise"
            ))
        })?;
    play_media(window, repo, next.get(), "episode".to_string()).await
}

/// Remet à 0 la progression de tous les épisodes de la série, puis
/// joue le premier épisode dans l'ordre `(season, episode)` ASC.
///
/// Le tri est garanti par `Repo::list_episodes` (cf. doc de cette
/// méthode + index `idx_episode_series`). Une série vide retourne
/// `KyroError::Validation` plutôt que d'essayer un appel `play_media`
/// sans cible — l'UI ne devrait jamais permettre ce cas, mais on est
/// défensif.
///
/// La remise à zéro et le lancement sont **non transactionnels** :
/// si le lancement échoue après la remise à zéro, la progression a
/// déjà été perdue. C'est conforme au requirement 12.3 (« Revoir
/// depuis le début ») — l'utilisateur a explicitement demandé un
/// reset.
///
/// _Requirements: 12.3_
#[tauri::command]
pub async fn play_rewatch_from_start(
    window: tauri::Window,
    repo: tauri::State<'_, Repo>,
    series_id: i64,
) -> Result<(), KyroError> {
    let series_id_typed = MediaItemId(series_id);
    let episode_ids = reset_all_episodes_progress(repo.inner(), series_id_typed).await?;

    let first = episode_ids.into_iter().next().ok_or_else(|| {
        KyroError::Validation(format!(
            "série id={series_id} sans épisode, impossible de revoir depuis le début"
        ))
    })?;

    play_media(window, repo, first.get(), "episode".to_string()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::TempDir;

    use crate::domain::status::Status;
    use crate::domain::types::{Category, MediaType};
    use crate::infra::repo::{Repo, UpsertEpisodeInput, UpsertMediaInput};

    // -----------------------------------------------------------------
    // Tests `install_lua_script_to` (tâche 11.2).
    // -----------------------------------------------------------------

    /// Le contenu embarqué doit correspondre exactement au fichier
    /// `resources/kyro.lua` du repo — invariant qui garantit que la
    /// cible installée est bien le script de référence (et non un
    /// artefact d'une compilation antérieure).
    #[test]
    fn embedded_lua_bytes_match_resources_file() {
        let from_disk = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("resources")
                .join("kyro.lua"),
        )
        .expect("resources/kyro.lua doit exister à la racine du crate");
        assert_eq!(KYRO_LUA_BYTES, from_disk.as_slice());
        assert!(
            !KYRO_LUA_BYTES.is_empty(),
            "kyro.lua embarqué ne doit pas être vide"
        );
    }

    /// Cas nominal : le dossier de destination existe déjà, vide. Le
    /// fichier doit être créé avec le contenu exact embarqué et le
    /// chemin renvoyé pointe bien sur cette cible.
    #[test]
    fn installs_lua_into_existing_directory() {
        let tmp = TempDir::new().expect("tempdir");
        let dest_dir = tmp.path();

        let written = install_lua_script_to(dest_dir).expect("install ok");

        assert_eq!(written, dest_dir.join("kyro.lua"));
        assert!(written.exists(), "le fichier kyro.lua doit exister");
        let on_disk = std::fs::read(&written).expect("read kyro.lua");
        assert_eq!(on_disk, KYRO_LUA_BYTES);
    }

    /// Le helper crée récursivement les dossiers manquants : on cible
    /// un sous-chemin `mpv.net/scripts` à plusieurs niveaux sous le
    /// `tempdir`, qui n'existe pas encore — l'installation doit le
    /// créer (requirement 9.7 « créer le dossier si manquant »).
    #[test]
    fn creates_missing_parent_directories_recursively() {
        let tmp = TempDir::new().expect("tempdir");
        let dest_dir = tmp.path().join("mpv.net").join("scripts");
        assert!(
            !dest_dir.exists(),
            "précondition : le dossier ne doit pas exister"
        );

        let written = install_lua_script_to(&dest_dir).expect("install ok");

        assert!(dest_dir.is_dir(), "le dossier doit avoir été créé");
        assert_eq!(written, dest_dir.join("kyro.lua"));
        assert!(written.is_file());
    }

    /// Une seconde installation par-dessus un `kyro.lua` déjà présent
    /// (avec un contenu différent) doit écraser la cible sans erreur
    /// (requirement 9.7 « écraser si déjà présent »). Le fichier
    /// résultant doit contenir exactement les bytes embarqués.
    #[test]
    fn overwrites_existing_lua_file() {
        let tmp = TempDir::new().expect("tempdir");
        let dest_dir = tmp.path();
        let dest_file = dest_dir.join("kyro.lua");

        // Pré-place un contenu différent (ancien script, contenu
        // arbitraire) dans la cible.
        std::fs::write(&dest_file, b"-- ancien script obsolete\n")
            .expect("seed existing file");
        assert!(dest_file.exists());

        let written = install_lua_script_to(dest_dir).expect("install ok");

        assert_eq!(written, dest_file);
        let on_disk = std::fs::read(&written).expect("read kyro.lua");
        assert_eq!(
            on_disk, KYRO_LUA_BYTES,
            "le fichier existant doit être écrasé par le contenu embarqué"
        );
    }

    /// Idempotence : appeler le helper deux fois de suite sur le même
    /// dossier doit aboutir à un état identique (même chemin, même
    /// contenu). Garantit qu'un utilisateur peut cliquer plusieurs fois
    /// sur « Installer le script Lua » sans effet de bord.
    #[test]
    fn install_is_idempotent() {
        let tmp = TempDir::new().expect("tempdir");
        let dest_dir = tmp.path();

        let first = install_lua_script_to(dest_dir).expect("install 1");
        let second = install_lua_script_to(dest_dir).expect("install 2");

        assert_eq!(first, second);
        let on_disk = std::fs::read(&second).expect("read kyro.lua");
        assert_eq!(on_disk, KYRO_LUA_BYTES);
    }

    /// Le résolveur de chemin construit bien le sous-chemin attendu —
    /// indépendamment de la racine résolue (`APPDATA` ou
    /// `dirs::config_dir()`), il termine par `mpv.net\scripts`.
    #[test]
    fn resolved_path_ends_with_mpv_net_scripts() {
        // Force une racine connue pour ce test afin de ne pas dépendre
        // de l'environnement d'exécution. Les tests Rust étant
        // potentiellement parallèles, on garde la fenêtre où `APPDATA`
        // est modifiée la plus courte possible et on restaure ensuite.
        let original = std::env::var_os("APPDATA");
        let tmp = TempDir::new().expect("tempdir");
        std::env::set_var("APPDATA", tmp.path());

        let resolved =
            resolve_mpv_net_scripts_dir().expect("resolve mpv.net scripts dir");

        // Restauration immédiate avant les assertions pour ne pas
        // polluer d'autres tests si l'assertion panique.
        match original {
            Some(v) => std::env::set_var("APPDATA", v),
            None => std::env::remove_var("APPDATA"),
        }

        assert!(
            resolved.ends_with(std::path::Path::new("mpv.net").join("scripts")),
            "chemin résolu inattendu : {resolved:?}"
        );
        assert!(
            resolved.starts_with(tmp.path()),
            "le chemin résolu doit être ancré sur la racine APPDATA simulée"
        );
    }

    // -----------------------------------------------------------------
    // Tests `PlaybackKind::parse` (tâche 12.3).
    // -----------------------------------------------------------------

    #[test]
    fn playback_kind_parses_media_and_episode() {
        assert_eq!(PlaybackKind::parse("media").unwrap(), PlaybackKind::Media);
        assert_eq!(
            PlaybackKind::parse("episode").unwrap(),
            PlaybackKind::Episode
        );
    }

    #[test]
    fn playback_kind_rejects_unknown_value_with_validation() {
        let err = PlaybackKind::parse("Movie").expect_err("variante inconnue");
        assert!(matches!(err, KyroError::Validation(_)));

        let err = PlaybackKind::parse("").expect_err("variante vide");
        assert!(matches!(err, KyroError::Validation(_)));

        // Sensibilité à la casse : `MEDIA` doit être refusé pour
        // garder un contrat strict côté frontend.
        let err = PlaybackKind::parse("MEDIA").expect_err("casse alternative");
        assert!(matches!(err, KyroError::Validation(_)));
    }

    // -----------------------------------------------------------------
    // Tests `load_target_info` et `reset_all_episodes_progress` (tâche
    // 12.3) — exécutés sur un pool SQLite temporaire avec migrations.
    // -----------------------------------------------------------------

    /// Construit un runtime tokio current_thread pour les tests qui
    /// exercent du code asynchrone (la feature `macros` de tokio est
    /// activée mais on évite `#[tokio::test]` pour rester aligné avec
    /// les autres tests du crate — cf. `infra::repo::tests`).
    fn block_on<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("création d'un runtime tokio current_thread")
            .block_on(fut)
    }

    /// Ouvre un pool SQLite sur fichier temporaire, applique les
    /// migrations versionnées, retourne un `Repo` prêt pour les tests.
    /// Reproduit fidèlement les pragmas de production
    /// (cf. `infra::repo::tests::fresh_pool`).
    async fn fresh_repo(db_dir: &Path) -> Repo {
        use sqlx::sqlite::{
            SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions,
            SqliteSynchronous,
        };

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

        Repo::new(pool)
    }

    /// Insère un Film via l'API publique du repo, en pré-remplissant
    /// `watch_progress_seconds` directement via SQL pour les besoins
    /// du test.
    async fn insert_film(
        repo: &Repo,
        path: &str,
        duration: i64,
        watch_progress: i64,
    ) -> MediaItemId {
        let input = UpsertMediaInput {
            folder_id: None,
            media_type: MediaType::Film,
            file_path: path.to_string(),
            size_bytes: 1_000_000,
            title: "Film".to_string(),
            year: None,
            quality: None,
            duration_seconds: Some(duration),
            video_codec: None,
            width: None,
            height: None,
            numbering_unknown: false,
        };
        let id = repo.upsert_media_item(&input).await.unwrap();
        sqlx::query("UPDATE media_item SET watch_progress_seconds = ? WHERE id = ?")
            .bind(watch_progress)
            .bind(id.get())
            .execute(repo.pool())
            .await
            .unwrap();
        id
    }

    /// Insère une série avec N épisodes via l'API publique du repo.
    /// Retourne `(series_id, vec<ep_id>)` dans l'ordre des épisodes
    /// fournis (qui peut différer de l'ordre `(season, episode)` —
    /// utile pour vérifier le tri appliqué par `list_episodes`).
    async fn insert_series_with_episodes(
        repo: &Repo,
        series_path: &str,
        episodes: &[(i64, i64, &str)],
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
        for (season, episode, path) in episodes {
            let ep = UpsertEpisodeInput {
                series_id,
                season_number: *season,
                episode_number: *episode,
                file_path: path.to_string(),
                size_bytes: 1_000,
                duration_seconds: Some(100),
                video_codec: None,
                width: None,
                height: None,
                numbering_unknown: false,
            };
            ids.push(repo.upsert_episode_item(&ep).await.unwrap());
        }
        (series_id, ids)
    }

    #[test]
    fn load_target_info_returns_path_and_progress_for_media() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let repo = fresh_repo(db_dir.path()).await;

            let id = insert_film(&repo, "C:/movies/A.mkv", 100, 42).await;

            let info = load_target_info(&repo, id.get(), PlaybackKind::Media)
                .await
                .expect("media existant doit être chargé");

            assert_eq!(info.target, ProgressTarget::Media(id));
            assert_eq!(info.file_path, "C:/movies/A.mkv");
            assert_eq!(info.start_seconds, 42);
        });
    }

    #[test]
    fn load_target_info_returns_path_and_progress_for_episode() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let repo = fresh_repo(db_dir.path()).await;

            let (_series_id, ep_ids) = insert_series_with_episodes(
                &repo,
                "C:/series/Show",
                &[(1, 1, "C:/series/Show/E1.mkv")],
            )
            .await;
            // Pré-positionner une progression connue.
            sqlx::query(
                "UPDATE episode_item SET watch_progress_seconds = 17 WHERE id = ?",
            )
            .bind(ep_ids[0].get())
            .execute(repo.pool())
            .await
            .unwrap();

            let info = load_target_info(&repo, ep_ids[0].get(), PlaybackKind::Episode)
                .await
                .expect("épisode existant doit être chargé");

            assert_eq!(info.target, ProgressTarget::Episode(ep_ids[0]));
            assert_eq!(info.file_path, "C:/series/Show/E1.mkv");
            assert_eq!(info.start_seconds, 17);
        });
    }

    #[test]
    fn load_target_info_rejects_missing_media_with_validation() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let repo = fresh_repo(db_dir.path()).await;

            let err = load_target_info(&repo, 99_999, PlaybackKind::Media)
                .await
                .expect_err("id inexistant doit être rejeté");
            assert!(matches!(err, KyroError::Validation(_)));
        });
    }

    #[test]
    fn load_target_info_rejects_missing_episode_with_validation() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let repo = fresh_repo(db_dir.path()).await;

            let err = load_target_info(&repo, 99_999, PlaybackKind::Episode)
                .await
                .expect_err("id inexistant doit être rejeté");
            assert!(matches!(err, KyroError::Validation(_)));
        });
    }

    #[test]
    fn load_target_info_rejects_media_with_null_file_path() {
        // Cas typique : série placeholder « À classer » insérée sans
        // file_path. La lecture doit être refusée avec Validation.
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let repo = fresh_repo(db_dir.path()).await;

            // Crée un library_folder + un placeholder via l'API repo.
            let lib = TempDir::new().unwrap();
            let folder_id = repo
                .add_library_folder(lib.path(), Category::Series)
                .await
                .unwrap();
            let placeholder_id = repo
                .ensure_unsorted_series(folder_id, Category::Series)
                .await
                .unwrap();

            let err =
                load_target_info(&repo, placeholder_id.get(), PlaybackKind::Media)
                    .await
                    .expect_err("placeholder sans file_path doit être refusé");
            match err {
                KyroError::Validation(msg) => {
                    assert!(
                        msg.contains("file_path"),
                        "le message doit mentionner file_path (got: {msg:?})"
                    );
                }
                other => panic!("expected Validation, got {other:?}"),
            }
        });
    }

    #[test]
    fn reset_all_episodes_progress_resets_watch_progress_and_status() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let repo = fresh_repo(db_dir.path()).await;

            let (series_id, ep_ids) = insert_series_with_episodes(
                &repo,
                "C:/series/Show",
                &[
                    (1, 1, "C:/series/Show/E1.mkv"),
                    (1, 2, "C:/series/Show/E2.mkv"),
                    (2, 1, "C:/series/Show/E2-S2.mkv"),
                ],
            )
            .await;

            // Pré-remplit progressions et statuts variés.
            for (i, ep) in ep_ids.iter().enumerate() {
                let pos = ((i + 1) as i64) * 30;
                sqlx::query(
                    "UPDATE episode_item \
                       SET watch_progress_seconds = ?, status = 'en_cours' \
                     WHERE id = ?",
                )
                .bind(pos)
                .bind(ep.get())
                .execute(repo.pool())
                .await
                .unwrap();
            }

            let returned = reset_all_episodes_progress(&repo, series_id)
                .await
                .expect("reset doit réussir");

            // Tous les épisodes doivent être remis à 0 + statut NonVu.
            assert_eq!(returned.len(), ep_ids.len());
            for ep in &ep_ids {
                let (wp, st): (i64, String) = sqlx::query_as(
                    "SELECT watch_progress_seconds, status \
                       FROM episode_item WHERE id = ?",
                )
                .bind(ep.get())
                .fetch_one(repo.pool())
                .await
                .unwrap();
                assert_eq!(wp, 0, "watch_progress_seconds doit être 0");
                assert_eq!(
                    st,
                    Status::NonVu.as_str(),
                    "status doit être recalculé à NonVu"
                );
            }
        });
    }

    #[test]
    fn reset_all_episodes_progress_returns_ids_in_season_episode_order() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let repo = fresh_repo(db_dir.path()).await;

            // Insertion volontairement désordonnée.
            let (series_id, _) = insert_series_with_episodes(
                &repo,
                "C:/series/Show",
                &[
                    (2, 1, "C:/series/Show/S02E01.mkv"),
                    (1, 2, "C:/series/Show/S01E02.mkv"),
                    (1, 1, "C:/series/Show/S01E01.mkv"),
                ],
            )
            .await;

            let returned = reset_all_episodes_progress(&repo, series_id)
                .await
                .expect("reset doit réussir");

            // L'ordre retourné doit être (1,1) → (1,2) → (2,1) :
            // c'est l'ordre garanti par list_episodes (utile pour
            // play_rewatch_from_start qui prend le premier).
            let listed = repo.list_episodes(series_id).await.unwrap();
            let listed_ids: Vec<EpisodeItemId> = listed.iter().map(|e| e.id).collect();
            assert_eq!(returned, listed_ids);
            // Sanity : on a bien 3 épisodes.
            assert_eq!(returned.len(), 3);
        });
    }

    #[test]
    fn reset_all_episodes_progress_on_empty_series_returns_empty_vec() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let repo = fresh_repo(db_dir.path()).await;

            let (series_id, _) =
                insert_series_with_episodes(&repo, "C:/series/Empty", &[]).await;

            let returned = reset_all_episodes_progress(&repo, series_id)
                .await
                .expect("reset sur série vide doit réussir (no-op)");
            assert!(returned.is_empty());
        });
    }
}
