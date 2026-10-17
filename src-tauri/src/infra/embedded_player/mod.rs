//! Player embarqué Kyro — orchestration de libmpv chargé dynamiquement.
//!
//! Ce module fournit une alternative au flux `player_bridge` (mpv.net
//! externe + named pipe NDJSON) en embarquant **libmpv** directement
//! dans une fenêtre Tauri Kyro. Les deux flux coexistent : la commande
//! `play_media` route entre `embedded` et `external` via le réglage
//! [`crate::infra::settings_keys::SETTING_KEY_PLAYBACK_MODE`].
//!
//! ## Garanties d'isolation
//!
//! L'instance libmpv est créée avec un environnement strictement isolé
//! (cf. `config::build_isolated_options`) :
//!
//! * `config-dir = %APPDATA%\Kyro\mpv-config` — aucune lecture du
//!   `%APPDATA%\mpv.net\` global.
//! * `input-default-bindings = no` — Kyro câble ses propres raccourcis.
//! * `input-vo-keyboard = no` — pas de capture clavier mpv.
//! * `osc = no` — pas d'OSC mpv, l'UI est rendue par Svelte.
//!
//! Aucun hook FS ni named pipe n'est partagé avec le mode externe.
//!
//! ## Chargement dynamique de libmpv-2.dll
//!
//! La DLL est résolue puis chargée à l'ouverture du player via
//! [`ffi::MpvLib::load`]. Si elle est absente, `EmbeddedPlayer::open`
//! retourne une erreur explicite ; Kyro reste fonctionnel et
//! l'utilisateur peut basculer sur `playback_mode = "external"`.
//!
//! ## Architecture
//!
//! ```text
//! EmbeddedPlayer (façade thread-safe, exposée aux commands Tauri)
//!     │
//!     ├─ Arc<MpvLib>          (FFI bindings libloading + Drop = unload)
//!     ├─ MpvHandle (raw)      (mpv_create + mpv_initialize)
//!     ├─ JoinHandle event_loop (thread bloquant mpv_wait_event)
//!     └─ mpsc::Sender<EmbeddedPlayerEvent>
//!                              └─► consommé par la task Tauri qui
//!                                  émet sur la fenêtre `player`
//!                                  + persiste la progression via Repo.
//! ```

pub mod config;
pub mod ffi;

#[cfg(target_os = "windows")]
mod event_loop;

#[cfg(target_os = "windows")]
mod host_window;

use std::path::PathBuf;
use std::sync::Arc;

use crate::error::KyroError;

/// Évènement remonté par le thread `mpv_wait_event` vers la couche
/// commandes Tauri (qui se charge ensuite d'émettre vers le frontend
/// et de persister la progression via [`crate::infra::repo::Repo`]).
///
/// Le mapping avec les évènements `kyro.lua` (mode externe) est
/// volontairement isomorphe — le code de persistance progress / status
/// peut être partagé.
#[derive(Debug, Clone, PartialEq)]
pub enum EmbeddedPlayerEvent {
    /// Position courante observée (`time-pos`) + durée totale
    /// (`duration`). Émise au fil de l'eau (pacing libmpv ≈ 1/s).
    Progress { position: f64, duration: f64 },

    /// Lecture terminée naturellement (`eof-reached = true`). Le
    /// frontend doit basculer le statut à `Vu`.
    Completed,

    /// Le user (ou un `loadfile`) a basculé sur un autre fichier en
    /// cours de session. Permet aux séries de chaîner des épisodes
    /// sans recréer une `EmbeddedPlayer` complète.
    FileChanged { file_path: String },

    /// Pause / reprise observée (utile pour l'UI, pas pour la
    /// persistance).
    PauseChanged { paused: bool },

    /// La session libmpv s'est terminée (quit OS-level, fenêtre
    /// fermée). Plus aucun event ne suivra.
    Closed { last_position: f64 },
}

/// Cible de lecture passée à [`EmbeddedPlayer::open`] — typage
/// identique à `ProgressTarget` mais découplé pour ne pas créer de
/// dépendance cyclique côté tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackRequest {
    /// Chemin absolu du fichier média à charger.
    pub file_path: PathBuf,
    /// Position de reprise en secondes (passe `--start=<n>` à mpv via
    /// la propriété `start`). 0 = depuis le début.
    pub start_seconds: u32,
    /// HWND **parent** (typiquement la fenêtre Kyro `main`) sous
    /// lequel sera créée la fenêtre Win32 enfant qui héberge libmpv.
    /// libmpv ne reçoit pas ce HWND directement : il reçoit le HWND
    /// du child window créé par [`host_window::HostWindow`].
    /// Stocké en `isize` pour rester `Send + Sync` et neutre sur les
    /// autres plateformes.
    pub parent_hwnd: isize,
    /// Répertoire `config-dir` isolé que libmpv doit utiliser.
    /// Typiquement `%APPDATA%\Kyro\mpv-config`. Créé par l'appelant
    /// avant `open` (idempotent).
    pub config_dir: PathBuf,
}

/// Façade thread-safe consommée par les commandes Tauri.
///
/// L'instance encapsule le contexte libmpv brut, garde le `MpvLib`
/// vivant via un `Arc` (la DLL ne peut être unload qu'après
/// `mpv_terminate_destroy`) et expose des méthodes synchrones pour
/// piloter la lecture (`pause`, `seek`, `set_audio_track`, etc.).
///
/// Le drop appelle `mpv_terminate_destroy` puis libère la DLL.
pub struct EmbeddedPlayer {
    inner: Arc<EmbeddedPlayerInner>,
}

impl std::fmt::Debug for EmbeddedPlayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Pas d'exposition du handle brut (pointeur opaque) ni de la
        // DLL chargée — informations sans valeur ajoutée pour le
        // diagnostic, et bruyantes dans les logs / sorties de tests.
        f.debug_struct("EmbeddedPlayer").finish_non_exhaustive()
    }
}

/// État interne partagé entre la façade et le thread event-loop.
pub(crate) struct EmbeddedPlayerInner {
    /// Bindings libmpv (libloading). Conservé par `Arc` pour que la
    /// DLL ne soit pas drop avant que le contexte mpv soit détruit.
    #[allow(dead_code)] // utilisé dans les implémentations target_os = "windows"
    lib: Arc<ffi::MpvLib>,
    /// Handle opaque retourné par `mpv_create`. `*mut c_void` n'est
    /// pas `Send` par défaut ; on l'enveloppe dans `MpvHandle` qui
    /// porte un `unsafe impl Send + Sync` documenté.
    #[allow(dead_code)]
    handle: ffi::MpvHandle,
    /// Fenêtre Win32 child qui héberge le rendu vidéo. Possédée par
    /// l'instance ; `Drop` détruit le HWND quand `EmbeddedPlayer`
    /// tombe.
    #[cfg(target_os = "windows")]
    #[allow(dead_code)]
    pub(crate) host: host_window::HostWindow,
}

// SAFETY: libmpv est documenté comme « MT-safe : toute fonction de
// l'API peut être appelée depuis n'importe quel thread, sauf si
// indiqué autrement ». Cf. mpv/libmpv/client.h, section « Threading ».
// Notre wrapper synchronise les accès via &self → &Arc<inner>, donc
// l'enveloppe est sûre tant qu'on ne mute pas le handle après init.
unsafe impl Send for EmbeddedPlayerInner {}
unsafe impl Sync for EmbeddedPlayerInner {}

impl EmbeddedPlayer {
    /// Ouvre une session libmpv : charge la DLL, crée le contexte,
    /// applique l'isolation puis charge le fichier demandé.
    ///
    /// Le HWND vidéo (`HostWindow`) est créé **par l'appelant** sur le
    /// thread UI Win32 et passé via `request.host`. Cette inversion
    /// est indispensable : créer un HWND depuis un worker Tokio puis
    /// le manipuler depuis un autre thread Tokio gèle le process
    /// (Windows attache la fenêtre au thread créateur, qui doit
    /// pomper sa propre message-queue).
    ///
    /// Sur les plateformes autres que Windows, retourne
    /// `KyroError::Validation`.
    #[cfg(target_os = "windows")]
    pub fn open(
        request: PlaybackRequest,
        host: host_window::HostWindow,
    ) -> Result<(Self, tokio::sync::mpsc::UnboundedReceiver<EmbeddedPlayerEvent>), KyroError>
    {
        event_loop::open(request, host)
    }

    /// Stub non-Windows : le player embarqué est désactivé et l'appel
    /// retourne immédiatement une erreur. Permet au code multi-cible
    /// (tests sur Linux/macOS) de compiler sans `cfg(target_os)`
    /// éparpillé dans les couches supérieures.
    #[cfg(not(target_os = "windows"))]
    pub fn open(
        _request: PlaybackRequest,
    ) -> Result<(Self, tokio::sync::mpsc::UnboundedReceiver<EmbeddedPlayerEvent>), KyroError>
    {
        Err(KyroError::Validation(
            "le player embarqué n'est disponible que sur Windows".to_string(),
        ))
    }

    /// Bascule pause / lecture.
    #[cfg(target_os = "windows")]
    pub fn toggle_pause(&self) -> Result<(), KyroError> {
        event_loop::toggle_pause(&self.inner)
    }

    /// Saute à `position_seconds` (absolu).
    #[cfg(target_os = "windows")]
    pub fn seek_absolute(&self, position_seconds: f64) -> Result<(), KyroError> {
        event_loop::seek_absolute(&self.inner, position_seconds)
    }

    /// Sélectionne une piste audio par identifiant mpv (`aid`). `None`
    /// = piste par défaut, `Some(0)` = pas de piste audio (mute), etc.
    #[cfg(target_os = "windows")]
    pub fn set_audio_track(&self, aid: Option<i64>) -> Result<(), KyroError> {
        event_loop::set_audio_track(&self.inner, aid)
    }

    /// Sélectionne une piste sous-titres (`sid`). `None` = pas de
    /// sous-titres ; `Some(n)` = piste n.
    #[cfg(target_os = "windows")]
    pub fn set_subtitle_track(&self, sid: Option<i64>) -> Result<(), KyroError> {
        event_loop::set_subtitle_track(&self.inner, sid)
    }

    /// Charge un nouveau fichier dans la session courante (équivalent
    /// `loadfile`). Permet le chaînage d'épisodes sans recréer le
    /// contexte ni la fenêtre.
    #[cfg(target_os = "windows")]
    pub fn load_file(&self, file_path: &std::path::Path) -> Result<(), KyroError> {
        event_loop::load_file(&self.inner, file_path)
    }

    /// Règle le volume (0 à 150).
    #[cfg(target_os = "windows")]
    pub fn set_volume(&self, volume: f64) -> Result<(), KyroError> {
        event_loop::set_volume(&self.inner, volume)
    }

    /// Mute / unmute.
    #[cfg(target_os = "windows")]
    pub fn set_mute(&self, muted: bool) -> Result<(), KyroError> {
        event_loop::set_mute(&self.inner, muted)
    }

    /// Vitesse de lecture (0.1 à 8.0).
    #[cfg(target_os = "windows")]
    pub fn set_speed(&self, speed: f64) -> Result<(), KyroError> {
        event_loop::set_speed(&self.inner, speed)
    }

    /// Renvoie la `track-list` mpv en JSON (chaîne brute).
    #[cfg(target_os = "windows")]
    pub fn track_list(&self) -> Result<String, KyroError> {
        event_loop::get_track_list(&self.inner)
    }

    /// Demande la fin de la session libmpv. Le thread event-loop
    /// émettra un `Closed` puis se terminera. `Drop` est idempotent
    /// avec ce quit (double-quit toléré côté libmpv).
    #[cfg(target_os = "windows")]
    pub fn quit(&self) -> Result<(), KyroError> {
        event_loop::quit(&self.inner)
    }

    /// Repositionne / redimensionne la fenêtre vidéo.
    ///
    /// Coordonnées en pixels physiques dans le repère du parent
    /// (la fenêtre Kyro `main`). Le frontend convertit ses coordonnées
    /// CSS via `window.devicePixelRatio` avant l'invoke.
    #[cfg(target_os = "windows")]
    pub fn set_geometry(
        &self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Result<(), KyroError> {
        self.inner.host.set_geometry(x, y, width, height)
    }

    /// Affiche ou masque la fenêtre vidéo sans détruire l'instance
    /// libmpv. Utilisé pour le mode "fermé" du PiP : la lecture est
    /// suspendue côté UI, mais l'instance reste vivante pour reprise
    /// instantanée.
    #[cfg(target_os = "windows")]
    pub fn set_visible(&self, visible: bool) -> Result<(), KyroError> {
        self.inner.host.set_visible(visible)
    }

    /// Stubs non-Windows : retournent immédiatement `Ok(())` (no-op)
    /// pour ne pas casser la compilation des couches supérieures sur
    /// Linux/macOS. Une `EmbeddedPlayer` n'a jamais pu être
    /// construite sur ces plateformes (cf. `open`).
    #[cfg(not(target_os = "windows"))]
    pub fn toggle_pause(&self) -> Result<(), KyroError> {
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    pub fn seek_absolute(&self, _: f64) -> Result<(), KyroError> {
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    pub fn set_audio_track(&self, _: Option<i64>) -> Result<(), KyroError> {
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    pub fn set_subtitle_track(&self, _: Option<i64>) -> Result<(), KyroError> {
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    pub fn load_file(&self, _: &std::path::Path) -> Result<(), KyroError> {
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    pub fn quit(&self) -> Result<(), KyroError> {
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    pub fn set_geometry(&self, _: i32, _: i32, _: i32, _: i32) -> Result<(), KyroError> {
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    pub fn set_visible(&self, _: bool) -> Result<(), KyroError> {
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    pub fn set_volume(&self, _: f64) -> Result<(), KyroError> {
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    pub fn set_mute(&self, _: bool) -> Result<(), KyroError> {
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    pub fn set_speed(&self, _: f64) -> Result<(), KyroError> {
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    pub fn track_list(&self) -> Result<String, KyroError> {
        Ok("[]".to_string())
    }
}

// Exposé pour le module event_loop, qui construit la façade après
// chargement de la DLL et `mpv_create`.
#[cfg(target_os = "windows")]
impl EmbeddedPlayer {
    pub(crate) fn from_inner(inner: Arc<EmbeddedPlayerInner>) -> Self {
        Self { inner }
    }
}

// Ré-exporte HostWindow pour que la couche commands puisse le créer
// sur le thread UI avant de le passer à `EmbeddedPlayer::open`.
#[cfg(target_os = "windows")]
pub use host_window::{set_input_callback, HostInputEvent, HostWindow};

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test : la valeur de `PlaybackRequest` doit être clonable
    /// et comparable, ce qui permet de l'utiliser dans des tests
    /// table-driven (cf. tests des commandes Tauri qui valident le
    /// routing `embedded`/`external`).
    #[test]
    fn playback_request_is_clonable_and_eq() {
        let req = PlaybackRequest {
            file_path: PathBuf::from(r"C:\videos\film.mkv"),
            start_seconds: 120,
            parent_hwnd: 0xDEAD_BEEF,
            config_dir: PathBuf::from(r"C:\Users\ogsas\AppData\Roaming\Kyro\mpv-config"),
        };
        let cloned = req.clone();
        assert_eq!(req, cloned);
    }

    /// L'événement `Progress` doit être clonable pour pouvoir être
    /// dispatché simultanément vers la persistance Repo et l'émission
    /// Tauri.
    #[test]
    fn embedded_player_event_progress_clones() {
        let evt = EmbeddedPlayerEvent::Progress {
            position: 12.5,
            duration: 1234.0,
        };
        let cloned = evt.clone();
        assert_eq!(evt, cloned);
    }
}
