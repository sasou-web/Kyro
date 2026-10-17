//! Commandes Tauri du **player embarqué** (libmpv chargé in-process).
//!
//! Ce module orchestre :
//!
//! * la résolution du HWND de la **fenêtre principale Kyro** (qui
//!   sert de parent pour la fenêtre Win32 enfant créée par libmpv) ;
//! * la création de l'instance [`infra::embedded_player::EmbeddedPlayer`]
//!   et son enregistrement dans le state Tauri (un slot
//!   `Mutex<Option<...>>` partagé) ;
//! * la persistance de la progression au fil des évènements émis par
//!   le thread `mpv_wait_event` (réutilise la même logique
//!   `Repo::set_watch_progress` que la branche externe) ;
//! * l'émission des événements Tauri consommés par l'UI Svelte ;
//! * le **positionnement / visibilité** du HWND vidéo, piloté par le
//!   frontend (commandes `set_geometry` et `set_visible`) pour
//!   supporter le PiP, le plein écran et la fermeture du player sans
//!   perdre la session.
//!
//! ## Cycle de vie
//!
//! ```text
//!   front: invoke('embedded_player_open', { targetId, kind })
//!     ─► Rust: lit playback_mode (doit être "embedded")
//!         ─► load_target_info(repo, target_id, kind)
//!         ─► récupère HWND de la fenêtre main
//!         ─► EmbeddedPlayer::open() crée :
//!              - HostWindow Win32 child (HWND vidéo, INVISIBLE au
//!                début, taille 0×0)
//!              - contexte libmpv qui rend dans ce HWND
//!              - thread mpv_wait_event
//!         ─► spawn task qui pump le receiver et :
//!              - persiste Progress / Completed via Repo
//!              - émet 'embedded-player-progress' / '-completed' / '-closed'
//!         ─► commande retourne immédiatement.
//!
//!   front (au resize de la div .video-zone) :
//!     ─► invoke('embedded_player_set_geometry', { x, y, width, height })
//!         ─► HostWindow::set_geometry(...)
//!     ─► invoke('embedded_player_set_visible', { visible: true })
//!         ─► HostWindow::set_visible(...) (la première fois)
//!
//!   front: invoke('embedded_player_pause' | '_seek' | '_quit' | ...)
//!     ─► commandes synchrones (mpv_command_async non-bloquant).
//! ```

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{Emitter, Manager};

use crate::commands::playback::{load_target_info, PlaybackKind};
use crate::commands::settings::get_playback_mode_with;
use crate::error::KyroError;
use crate::infra::embedded_player::{
    EmbeddedPlayer, EmbeddedPlayerEvent, PlaybackRequest,
};
use crate::infra::repo::{ProgressTarget, Repo};

// =====================================================================
// Constantes & noms d'événements
// =====================================================================

/// Label de la fenêtre Tauri principale Kyro, parente du HWND vidéo.
pub const MAIN_WINDOW_LABEL: &str = "main";

/// Évènement émis sur la fenêtre `main` à chaque progression.
pub const EVENT_EMBEDDED_PLAYER_PROGRESS: &str = "embedded-player-progress";

/// Évènement émis quand mpv signale `eof-reached`.
pub const EVENT_EMBEDDED_PLAYER_COMPLETED: &str = "embedded-player-completed";

/// Évènement émis sur changement pause/lecture.
pub const EVENT_EMBEDDED_PLAYER_PAUSE_CHANGED: &str =
    "embedded-player-pause-changed";

/// Évènement final : la session libmpv est terminée.
pub const EVENT_EMBEDDED_PLAYER_CLOSED: &str = "embedded-player-closed";

/// Évènement émis quand l'utilisateur **clique** sur la zone vidéo
/// (HWND mpv) — sert à déclencher pause/lecture côté UI.
pub const EVENT_EMBEDDED_PLAYER_CLICKED: &str = "embedded-player-clicked";

/// Évènement émis quand l'utilisateur **double-clique** sur la zone
/// vidéo — sert à basculer le plein écran système côté UI.
pub const EVENT_EMBEDDED_PLAYER_DOUBLE_CLICKED: &str =
    "embedded-player-double-clicked";

// =====================================================================
// Payloads JSON
// =====================================================================

#[derive(Debug, Clone, Serialize)]
pub struct EmbeddedPlayerProgressPayload {
    pub position: f64,
    pub duration: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct EmbeddedPlayerPausePayload {
    pub paused: bool,
}

// =====================================================================
// État global : un slot pour la session embarquée courante.
// =====================================================================

/// Slot partagé qui héberge l'instance courante d'`EmbeddedPlayer`
/// (au plus une à la fois).
#[derive(Default)]
pub struct EmbeddedPlayerSlot {
    inner: Mutex<Option<Arc<EmbeddedPlayer>>>,
}

impl EmbeddedPlayerSlot {
    pub fn new() -> Self {
        Self::default()
    }
    fn install(&self, player: Arc<EmbeddedPlayer>) {
        let mut guard = self.inner.lock().expect("EmbeddedPlayerSlot poisoned");
        if let Some(prev) = guard.take() {
            let _ = prev.quit();
        }
        *guard = Some(player);
    }

    fn take(&self) -> Option<Arc<EmbeddedPlayer>> {
        let mut guard = self.inner.lock().expect("EmbeddedPlayerSlot poisoned");
        guard.take()
    }

    fn current(&self) -> Result<Arc<EmbeddedPlayer>, KyroError> {
        let guard = self.inner.lock().expect("EmbeddedPlayerSlot poisoned");
        guard.clone().ok_or_else(|| {
            KyroError::Validation(
                "aucune session de lecture embarquée active".to_string(),
            )
        })
    }
}

// =====================================================================
// Helpers internes — pure-logique
// =====================================================================

fn resolve_kyro_mpv_config_dir(app_data_dir: &std::path::Path) -> PathBuf {
    app_data_dir.join("mpv-config")
}

/// Câble le callback Win32 du HostWindow pour qu'il émette
/// `embedded-player-clicked` / `embedded-player-double-clicked` sur
/// la fenêtre `main` à chaque interaction souris dans la zone vidéo.
///
/// Doit être appelé une fois au boot Tauri (cf. `main.rs::setup`).
/// Idempotent côté `host_window::set_input_callback` (OnceLock).
#[cfg(target_os = "windows")]
pub fn init_input_routing(app: tauri::AppHandle) {
    use crate::infra::embedded_player::{set_input_callback, HostInputEvent};
    set_input_callback(move |event| {
        let Some(webview) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
            return;
        };
        let window = webview.as_ref().window();
        let event_name = match event {
            HostInputEvent::Click => EVENT_EMBEDDED_PLAYER_CLICKED,
            HostInputEvent::DoubleClick => EVENT_EMBEDDED_PLAYER_DOUBLE_CLICKED,
        };
        let _ = window.emit(event_name, ());
    });
}

#[cfg(not(target_os = "windows"))]
pub fn init_input_routing(_app: tauri::AppHandle) {}

#[cfg(target_os = "windows")]
fn resolve_main_hwnd(app: &tauri::AppHandle) -> Result<isize, KyroError> {
    let webview = app.get_webview_window(MAIN_WINDOW_LABEL).ok_or_else(|| {
        KyroError::Validation(format!(
            "fenêtre Tauri {MAIN_WINDOW_LABEL:?} introuvable"
        ))
    })?;
    let window = webview.as_ref().window();
    let hwnd = window.hwnd().map_err(|e| {
        KyroError::Validation(format!(
            "impossible de récupérer le HWND de la fenêtre main: {e}"
        ))
    })?;
    Ok(hwnd.0 as isize)
}

#[cfg(not(target_os = "windows"))]
fn resolve_main_hwnd(_app: &tauri::AppHandle) -> Result<isize, KyroError> {
    Err(KyroError::Validation(
        "le player embarqué n'est disponible que sur Windows".to_string(),
    ))
}

// =====================================================================
// Commandes Tauri
// =====================================================================

/// Démarre une session de lecture embarquée pour la cible
/// `(target_id, kind)`.
///
/// Le HWND vidéo est créé invisible et de taille 0×0. Le frontend
/// doit appeler `embedded_player_set_geometry` puis
/// `embedded_player_set_visible(true)` une fois la zone vidéo prête.
#[tauri::command]
pub async fn embedded_player_open(
    app: tauri::AppHandle,
    repo: tauri::State<'_, Repo>,
    slot: tauri::State<'_, Arc<EmbeddedPlayerSlot>>,
    target_id: i64,
    kind: String,
) -> Result<(), KyroError> {
    let mode = get_playback_mode_with(repo.inner()).await?;
    if mode != "embedded" {
        return Err(KyroError::Validation(format!(
            "playback_mode = {mode:?} : utilisez la commande \
             play_media pour le mode externe"
        )));
    }

    let kind = PlaybackKind::parse(&kind)?;
    let info = load_target_info(repo.inner(), target_id, kind).await?;

    open_internal(app, repo.inner().clone(), slot.inner().clone(), info).await
}

/// Bascule pause / lecture.
#[tauri::command]
pub async fn embedded_player_toggle_pause(
    slot: tauri::State<'_, Arc<EmbeddedPlayerSlot>>,
) -> Result<(), KyroError> {
    slot.current()?.toggle_pause()
}

/// Saute à `position_seconds` (absolu).
#[tauri::command]
pub async fn embedded_player_seek(
    slot: tauri::State<'_, Arc<EmbeddedPlayerSlot>>,
    position_seconds: f64,
) -> Result<(), KyroError> {
    slot.current()?.seek_absolute(position_seconds)
}

/// Sélectionne une piste audio.
#[tauri::command]
pub async fn embedded_player_set_audio_track(
    slot: tauri::State<'_, Arc<EmbeddedPlayerSlot>>,
    aid: Option<i64>,
) -> Result<(), KyroError> {
    slot.current()?.set_audio_track(aid)
}

/// Sélectionne une piste sous-titres.
#[tauri::command]
pub async fn embedded_player_set_subtitle_track(
    slot: tauri::State<'_, Arc<EmbeddedPlayerSlot>>,
    sid: Option<i64>,
) -> Result<(), KyroError> {
    slot.current()?.set_subtitle_track(sid)
}

/// Termine la session embarquée courante (DestroyWindow + libmpv quit).
#[tauri::command]
pub async fn embedded_player_quit(
    slot: tauri::State<'_, Arc<EmbeddedPlayerSlot>>,
) -> Result<(), KyroError> {
    if let Some(player) = slot.take() {
        player.quit()?;
    }
    Ok(())
}

/// Repositionne / redimensionne la fenêtre vidéo Win32 dans le repère
/// du parent (fenêtre `main`). Coordonnées en **pixels physiques**.
///
/// Le frontend convertit ses coordonnées CSS via
/// `window.devicePixelRatio` avant l'invoke.
#[tauri::command]
pub async fn embedded_player_set_geometry(
    slot: tauri::State<'_, Arc<EmbeddedPlayerSlot>>,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Result<(), KyroError> {
    slot.current()?.set_geometry(x, y, width, height)
}

/// Affiche / masque le HWND vidéo (sans détruire l'instance libmpv).
///
/// Utilisé pour le "PiP fermé" : la lecture continue mais le rendu
/// vidéo est caché. L'utilisateur peut le ré-afficher en cliquant
/// sur la fiche en cours.
#[tauri::command]
pub async fn embedded_player_set_visible(
    slot: tauri::State<'_, Arc<EmbeddedPlayerSlot>>,
    visible: bool,
) -> Result<(), KyroError> {
    slot.current()?.set_visible(visible)
}

/// Règle le volume (0 à 150).
#[tauri::command]
pub async fn embedded_player_set_volume(
    slot: tauri::State<'_, Arc<EmbeddedPlayerSlot>>,
    volume: f64,
) -> Result<(), KyroError> {
    slot.current()?.set_volume(volume)
}

/// Mute / unmute.
#[tauri::command]
pub async fn embedded_player_set_mute(
    slot: tauri::State<'_, Arc<EmbeddedPlayerSlot>>,
    muted: bool,
) -> Result<(), KyroError> {
    slot.current()?.set_mute(muted)
}

/// Vitesse de lecture (0.1 à 8.0). 1.0 = normale.
#[tauri::command]
pub async fn embedded_player_set_speed(
    slot: tauri::State<'_, Arc<EmbeddedPlayerSlot>>,
    speed: f64,
) -> Result<(), KyroError> {
    slot.current()?.set_speed(speed)
}

/// Retourne la `track-list` mpv au format JSON (string).
/// Le frontend la parse pour construire les menus pistes audio /
/// sous-titres / vidéo.
#[tauri::command]
pub async fn embedded_player_track_list(
    slot: tauri::State<'_, Arc<EmbeddedPlayerSlot>>,
) -> Result<String, KyroError> {
    slot.current()?.track_list()
}

// =====================================================================
// Implémentation `open_internal`
// =====================================================================

#[cfg(target_os = "windows")]
async fn open_internal(
    app: tauri::AppHandle,
    repo: Repo,
    slot: Arc<EmbeddedPlayerSlot>,
    info: crate::commands::playback::TargetInfo,
) -> Result<(), KyroError> {
    let parent_hwnd = resolve_main_hwnd(&app)?;

    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| KyroError::Validation(format!("app_data_dir: {e}")))?;
    let config_dir = resolve_kyro_mpv_config_dir(&app_data_dir);

    // Création du HWND child sur le thread UI Win32 (indispensable :
    // une fenêtre est attachée au thread créateur et ne peut être
    // détruite que par lui ; faire ça depuis un worker Tokio gèle
    // l'app au moment du quit).
    let host = run_on_main_thread(&app, move || {
        crate::infra::embedded_player::HostWindow::create(parent_hwnd)
    })
    .await??;

    let request = PlaybackRequest {
        file_path: PathBuf::from(&info.file_path),
        start_seconds: info.start_seconds,
        parent_hwnd,
        config_dir,
    };

    let (player, mut rx) = crate::infra::embedded_player::EmbeddedPlayer::open(
        request, host,
    )?;
    let player_arc = Arc::new(player);
    slot.install(player_arc.clone());

    // Récupère la fenêtre main pour émettre les events vers son
    // WebView (les events sont reçus par le composant `<EmbeddedPlayer>`
    // monté dans la page de détail).
    let window = app
        .get_webview_window(MAIN_WINDOW_LABEL)
        .ok_or_else(|| {
            KyroError::Validation(format!(
                "fenêtre Tauri {MAIN_WINDOW_LABEL:?} introuvable"
            ))
        })?
        .as_ref()
        .window()
        .clone();

    let target = info.target;
    let window_for_task = window.clone();
    tauri::async_runtime::spawn(async move {
        let mut last_persisted_position: f64 = -1.0;
        while let Some(evt) = rx.recv().await {
            match evt {
                EmbeddedPlayerEvent::Progress { position, duration } => {
                    if (position - last_persisted_position).abs() >= 1.0 {
                        let pos_u32 = position.max(0.0).min(u32::MAX as f64) as u32;
                        if let Err(err) =
                            repo.set_watch_progress(target, pos_u32).await
                        {
                            tracing::warn!(
                                error = %err,
                                position,
                                "Kyro embedded_player: échec persistance progress"
                            );
                        } else {
                            last_persisted_position = position;
                        }
                    }
                    let _ = window_for_task.emit(
                        EVENT_EMBEDDED_PLAYER_PROGRESS,
                        EmbeddedPlayerProgressPayload { position, duration },
                    );
                }
                EmbeddedPlayerEvent::Completed => {
                    let _ = window_for_task.emit(
                        EVENT_EMBEDDED_PLAYER_COMPLETED,
                        target_payload(&target),
                    );
                }
                EmbeddedPlayerEvent::PauseChanged { paused } => {
                    let _ = window_for_task.emit(
                        EVENT_EMBEDDED_PLAYER_PAUSE_CHANGED,
                        EmbeddedPlayerPausePayload { paused },
                    );
                }
                EmbeddedPlayerEvent::FileChanged { file_path } => {
                    tracing::info!(
                        file_path = %file_path,
                        "Kyro embedded_player: file-changed (v1: ignoré)"
                    );
                }
                EmbeddedPlayerEvent::Closed { last_position } => {
                    let pos_u32 = last_position.max(0.0).min(u32::MAX as f64) as u32;
                    if pos_u32 > 0 {
                        if let Err(err) = repo.set_watch_progress(target, pos_u32).await
                        {
                            tracing::warn!(
                                error = %err,
                                position = last_position,
                                "Kyro embedded_player: échec persistance closed"
                            );
                        }
                    }
                    let _ = window_for_task.emit(
                        EVENT_EMBEDDED_PLAYER_CLOSED,
                        EmbeddedPlayerProgressPayload {
                            position: last_position,
                            duration: 0.0,
                        },
                    );
                    break;
                }
            }
        }
    });

    tracing::info!(
        target = ?info.target,
        file_path = %info.file_path,
        start_seconds = info.start_seconds,
        "Kyro embedded_player: session libmpv lancée (HWND child sous main)"
    );
    Ok(())
}

#[cfg(not(target_os = "windows"))]
async fn open_internal(
    _app: tauri::AppHandle,
    _repo: Repo,
    _slot: Arc<EmbeddedPlayerSlot>,
    _info: crate::commands::playback::TargetInfo,
) -> Result<(), KyroError> {
    Err(KyroError::Validation(
        "le player embarqué n'est disponible que sur Windows".to_string(),
    ))
}

fn target_payload(target: &ProgressTarget) -> serde_json::Value {
    match target {
        ProgressTarget::Media(id) => serde_json::json!({
            "kind": "media",
            "id": id.get(),
        }),
        ProgressTarget::Episode(id) => serde_json::json!({
            "kind": "episode",
            "id": id.get(),
        }),
    }
}

/// Exécute une closure sur le thread UI Tauri (le thread principal
/// Win32 qui pompe les messages) et renvoie son résultat.
///
/// Utile pour les opérations Win32 qui DOIVENT s'exécuter sur ce
/// thread (création/destruction de fenêtres notamment). Tauri 2
/// expose `AppHandle::run_on_main_thread(f)` qui pose la closure dans
/// la pompe principale ; on wrappe avec un `oneshot::channel` pour
/// récupérer le retour.
#[cfg(target_os = "windows")]
async fn run_on_main_thread<T, F>(
    app: &tauri::AppHandle,
    f: F,
) -> Result<T, KyroError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel::<T>();
    app.run_on_main_thread(move || {
        let _ = tx.send(f());
    })
    .map_err(|e| {
        KyroError::Validation(format!("run_on_main_thread post: {e}"))
    })?;
    rx.await.map_err(|e| {
        KyroError::Validation(format!("run_on_main_thread recv: {e}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_window_label_is_stable() {
        assert_eq!(MAIN_WINDOW_LABEL, "main");
    }

    #[test]
    fn event_names_are_stable() {
        assert_eq!(EVENT_EMBEDDED_PLAYER_PROGRESS, "embedded-player-progress");
        assert_eq!(EVENT_EMBEDDED_PLAYER_COMPLETED, "embedded-player-completed");
        assert_eq!(
            EVENT_EMBEDDED_PLAYER_PAUSE_CHANGED,
            "embedded-player-pause-changed"
        );
        assert_eq!(EVENT_EMBEDDED_PLAYER_CLOSED, "embedded-player-closed");
    }

    #[test]
    fn slot_starts_empty_and_returns_validation_error() {
        let slot = EmbeddedPlayerSlot::new();
        let err = slot
            .current()
            .expect_err("slot vide doit retourner une erreur");
        assert!(matches!(err, KyroError::Validation(_)));
    }

    #[test]
    fn slot_take_when_empty_returns_none() {
        let slot = EmbeddedPlayerSlot::new();
        assert!(slot.take().is_none());
    }

    #[test]
    fn target_payload_serializes_media_kind() {
        use crate::domain::types::MediaItemId;
        let target = ProgressTarget::Media(MediaItemId(42));
        let value = target_payload(&target);
        assert_eq!(value, serde_json::json!({ "kind": "media", "id": 42 }));
    }

    #[test]
    fn target_payload_serializes_episode_kind() {
        use crate::domain::types::EpisodeItemId;
        let target = ProgressTarget::Episode(EpisodeItemId(99));
        let value = target_payload(&target);
        assert_eq!(value, serde_json::json!({ "kind": "episode", "id": 99 }));
    }
}
