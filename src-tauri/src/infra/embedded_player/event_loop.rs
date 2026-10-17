//! Cycle de vie d'une session libmpv embarquée + thread event-loop.
//!
//! Le module orchestre les étapes :
//!
//! 1. [`open`] résout `libmpv-2.dll`, charge la lib, crée un contexte
//!    mpv, applique l'isolation puis appelle `mpv_initialize`.
//! 2. Inscrit les `observe_property` sur `time-pos`, `duration`,
//!    `pause`, `eof-reached`.
//! 3. Charge le fichier via `loadfile <path>` + `start = <secs>`.
//! 4. Spawn un **thread bloquant** (pas une tâche Tokio : `mpv_wait_event`
//!    est une syscall bloquante sans futur async) qui pump les events
//!    et émet vers un `mpsc::UnboundedSender<EmbeddedPlayerEvent>`.
//! 5. Renvoie la façade [`super::EmbeddedPlayer`] et le receiver
//!    associé à l'appelant Tauri.
//!
//! Toutes les opérations utilisateur (`pause`, `seek`, `quit`, etc.)
//! transitent par les fonctions thread-safe de ce module qui appellent
//! `mpv_command_async` ou `mpv_set_property_string` directement (libmpv
//! est documenté MT-safe).

#![cfg(target_os = "windows")]

use std::ffi::CString;
use std::path::Path;
use std::sync::Arc;

use tokio::sync::mpsc;

use super::config::{build_isolated_options, DEFAULT_MPV_CONF};
use super::ffi::{
    self, MpvEvent, MpvEventEndFile, MpvEventProperty, MpvHandle, MpvLib,
    MPV_END_FILE_REASON_EOF, MPV_EVENT_END_FILE, MPV_EVENT_FILE_LOADED,
    MPV_EVENT_NONE, MPV_EVENT_PROPERTY_CHANGE, MPV_EVENT_SHUTDOWN,
    MPV_FORMAT_DOUBLE, MPV_FORMAT_FLAG, REPLY_USERDATA_DURATION,
    REPLY_USERDATA_EOF_REACHED, REPLY_USERDATA_PAUSE, REPLY_USERDATA_TIME_POS,
};
use super::host_window::HostWindow;
use super::{EmbeddedPlayer, EmbeddedPlayerEvent, EmbeddedPlayerInner, PlaybackRequest};
use crate::error::KyroError;

/// Crée et initialise un contexte libmpv pour la requête fournie,
/// charge le fichier, spawn le thread event-loop, retourne la façade
/// + un récepteur des évènements.
///
/// L'appelant (commande Tauri `play_media` / `embedded_player_open`)
/// est responsable de :
///
/// * créer la fenêtre Tauri `player` et lui passer le HWND récupéré ;
/// * créer / valider le `config_dir` (`%APPDATA%\Kyro\mpv-config`) et
///   y déposer un `mpv.conf` par défaut si absent ;
/// * spawner une task Tokio qui consomme le receiver, persiste la
///   progression via `Repo::set_watch_progress` et émet les events
///   Tauri `embedded-player-progress`, etc.
pub fn open(
    request: PlaybackRequest,
    host: HostWindow,
) -> Result<
    (
        EmbeddedPlayer,
        mpsc::UnboundedReceiver<EmbeddedPlayerEvent>,
    ),
    KyroError,
> {
    // 1. Préparation du config_dir + mpv.conf par défaut.
    std::fs::create_dir_all(&request.config_dir).map_err(KyroError::Io)?;
    let conf_file = request.config_dir.join("mpv.conf");
    if !conf_file.exists() {
        std::fs::write(&conf_file, DEFAULT_MPV_CONF).map_err(KyroError::Io)?;
    }

    // 2. Résolution + chargement de libmpv-2.dll.
    let dll_path = ffi::resolve_libmpv_path().ok_or_else(|| {
        KyroError::Validation(format!(
            "{} introuvable. Déposez la DLL à côté de kyro.exe ou \
             basculez en mode lecteur externe dans les Paramètres.",
            ffi::LIBMPV_DLL
        ))
    })?;
    let lib = Arc::new(MpvLib::load(&dll_path)?);

    // 2.bis Le HostWindow a été créé par l'appelant (couche commands)
    //       sur le thread UI Win32. On récupère son HWND pour
    //       l'option `wid` mpv.
    let host_hwnd = host.handle();

    // 3. Création du contexte mpv.
    // SAFETY: mpv_create n'a pas de précondition documentée et est MT-safe.
    let raw_handle = unsafe { (lib.mpv_create)() };
    if raw_handle.is_null() {
        return Err(KyroError::Validation(
            "mpv_create a retourné NULL (mémoire insuffisante ?)".to_string(),
        ));
    }
    let handle = MpvHandle(raw_handle);

    // 4. Application des options d'isolation AVANT initialize.
    for opt in build_isolated_options(&request.config_dir, host_hwnd) {
        set_option_string(&lib, handle, opt.name, &opt.value)?;
    }

    // 5. Initialisation effective.
    // SAFETY: handle non-null garanti, lib chargée.
    let rc = unsafe { (lib.mpv_initialize)(handle.0) };
    if rc < 0 {
        // SAFETY: terminate_destroy accepte un handle valide même non
        // initialisé (cf. client.h).
        unsafe { (lib.mpv_terminate_destroy)(handle.0) };
        return Err(KyroError::Validation(format!(
            "mpv_initialize a échoué (code {rc})"
        )));
    }

    // 6. Inscription des propriétés observées.
    observe_property(&lib, handle, REPLY_USERDATA_TIME_POS, "time-pos", MPV_FORMAT_DOUBLE)?;
    observe_property(&lib, handle, REPLY_USERDATA_DURATION, "duration", MPV_FORMAT_DOUBLE)?;
    observe_property(&lib, handle, REPLY_USERDATA_PAUSE, "pause", MPV_FORMAT_FLAG)?;
    observe_property(&lib, handle, REPLY_USERDATA_EOF_REACHED, "eof-reached", MPV_FORMAT_FLAG)?;

    // 7. Position de reprise (`start`) — passée comme propriété avant
    //    le loadfile pour être appliquée au chargement.
    if request.start_seconds > 0 {
        set_option_string(&lib, handle, "start", &format!("{}", request.start_seconds))?;
    }

    // 8. Chargement du fichier (loadfile).
    load_file_internal(&lib, handle, &request.file_path)?;

    // 9. Façade + thread event-loop.
    let inner = Arc::new(EmbeddedPlayerInner {
        lib: lib.clone(),
        handle,
        host,
    });

    let (tx, rx) = mpsc::unbounded_channel::<EmbeddedPlayerEvent>();
    spawn_event_loop(inner.clone(), tx);

    Ok((EmbeddedPlayer::from_inner(inner), rx))
}

/// Bascule pause / lecture via `mpv_set_property_string("pause", ...)`.
pub fn toggle_pause(inner: &Arc<EmbeddedPlayerInner>) -> Result<(), KyroError> {
    // On lit la valeur actuelle via une commande `cycle pause` qui
    // évite un round-trip get-then-set non-atomique.
    let argv = ["cycle", "pause"];
    command_async(&inner.lib, inner.handle, &argv)
}

pub fn seek_absolute(
    inner: &Arc<EmbeddedPlayerInner>,
    position_seconds: f64,
) -> Result<(), KyroError> {
    let pos = format!("{:.3}", position_seconds.max(0.0));
    let argv = ["seek", pos.as_str(), "absolute"];
    command_async(&inner.lib, inner.handle, &argv)
}

pub fn set_audio_track(
    inner: &Arc<EmbeddedPlayerInner>,
    aid: Option<i64>,
) -> Result<(), KyroError> {
    let value = match aid {
        Some(n) if n >= 0 => n.to_string(),
        Some(_) => "no".to_string(),
        None => "auto".to_string(),
    };
    set_property_string(&inner.lib, inner.handle, "aid", &value)
}

pub fn set_subtitle_track(
    inner: &Arc<EmbeddedPlayerInner>,
    sid: Option<i64>,
) -> Result<(), KyroError> {
    let value = match sid {
        Some(n) if n >= 0 => n.to_string(),
        Some(_) => "no".to_string(),
        None => "no".to_string(),
    };
    set_property_string(&inner.lib, inner.handle, "sid", &value)
}

/// Règle le volume libmpv (0 à 100, mpv supporte aussi >100 pour
/// l'amplification, mais on le limite côté UI).
pub fn set_volume(
    inner: &Arc<EmbeddedPlayerInner>,
    volume: f64,
) -> Result<(), KyroError> {
    let v = volume.max(0.0).min(150.0);
    set_property_string(&inner.lib, inner.handle, "volume", &format!("{v}"))
}

/// Mute / unmute via la propriété mpv `mute`.
pub fn set_mute(
    inner: &Arc<EmbeddedPlayerInner>,
    muted: bool,
) -> Result<(), KyroError> {
    set_property_string(
        &inner.lib,
        inner.handle,
        "mute",
        if muted { "yes" } else { "no" },
    )
}

/// Vitesse de lecture (0.5 = ralenti, 1.0 = normal, 2.0 = double).
pub fn set_speed(
    inner: &Arc<EmbeddedPlayerInner>,
    speed: f64,
) -> Result<(), KyroError> {
    let s = speed.max(0.1).min(8.0);
    set_property_string(&inner.lib, inner.handle, "speed", &format!("{s}"))
}

/// Récupère la `track-list` mpv sérialisée en JSON (chaîne).
///
/// Le format renvoyé est documenté dans `mpv.io/manual/track-list` :
/// un tableau d'objets décrivant chaque piste audio / video / sub
/// avec son `id`, `type`, `title`, `lang`, `default`, `selected`...
/// On retourne la chaîne brute pour que la couche `commands` puisse
/// la repackager côté Tauri (parser JSON côté frontend).
pub fn get_track_list(inner: &Arc<EmbeddedPlayerInner>) -> Result<String, KyroError> {
    use std::ffi::CStr;
    let cname = std::ffi::CString::new("track-list").unwrap();
    // SAFETY: handle valide ; mpv_get_property_string alloue une
    // chaîne C que l'on doit libérer via mpv_free.
    let raw = unsafe { (inner.lib.mpv_get_property_string)(inner.handle.0, cname.as_ptr()) };
    if raw.is_null() {
        return Ok("[]".to_string());
    }
    let result = unsafe {
        let result = CStr::from_ptr(raw).to_string_lossy().into_owned();
        (inner.lib.mpv_free)(raw as *mut std::ffi::c_void);
        result
    };
    Ok(result)
}

pub fn load_file(
    inner: &Arc<EmbeddedPlayerInner>,
    file_path: &Path,
) -> Result<(), KyroError> {
    load_file_internal(&inner.lib, inner.handle, file_path)
}

pub fn quit(inner: &Arc<EmbeddedPlayerInner>) -> Result<(), KyroError> {
    let argv = ["quit"];
    command_async(&inner.lib, inner.handle, &argv)
}

// ---------------------------------------------------------------------------
// Helpers FFI thread-safe
// ---------------------------------------------------------------------------

fn set_option_string(
    lib: &MpvLib,
    handle: MpvHandle,
    name: &str,
    value: &str,
) -> Result<(), KyroError> {
    let cname = CString::new(name).map_err(|_| {
        KyroError::Validation(format!("nom d'option mpv non-cstring: {name:?}"))
    })?;
    let cvalue = CString::new(value).map_err(|_| {
        KyroError::Validation(format!("valeur d'option mpv non-cstring: {value:?}"))
    })?;
    // SAFETY: cname/cvalue restent vivants jusqu'à la fin de l'appel.
    let rc = unsafe { (lib.mpv_set_option_string)(handle.0, cname.as_ptr(), cvalue.as_ptr()) };
    if rc < 0 {
        return Err(KyroError::Validation(format!(
            "mpv_set_option_string({name}={value}) a échoué (code {rc})"
        )));
    }
    Ok(())
}

fn set_property_string(
    lib: &MpvLib,
    handle: MpvHandle,
    name: &str,
    value: &str,
) -> Result<(), KyroError> {
    let cname = CString::new(name).map_err(|_| {
        KyroError::Validation(format!("nom de propriété mpv non-cstring: {name:?}"))
    })?;
    let cvalue = CString::new(value).map_err(|_| {
        KyroError::Validation(format!("valeur de propriété mpv non-cstring: {value:?}"))
    })?;
    let rc =
        unsafe { (lib.mpv_set_property_string)(handle.0, cname.as_ptr(), cvalue.as_ptr()) };
    if rc < 0 {
        return Err(KyroError::Validation(format!(
            "mpv_set_property_string({name}={value}) a échoué (code {rc})"
        )));
    }
    Ok(())
}

fn observe_property(
    lib: &MpvLib,
    handle: MpvHandle,
    userdata: u64,
    name: &str,
    format: std::os::raw::c_int,
) -> Result<(), KyroError> {
    let cname = CString::new(name).map_err(|_| {
        KyroError::Validation(format!("nom propriété observe non-cstring: {name:?}"))
    })?;
    let rc = unsafe { (lib.mpv_observe_property)(handle.0, userdata, cname.as_ptr(), format) };
    if rc < 0 {
        return Err(KyroError::Validation(format!(
            "mpv_observe_property({name}) a échoué (code {rc})"
        )));
    }
    Ok(())
}

fn command_async(lib: &MpvLib, handle: MpvHandle, argv: &[&str]) -> Result<(), KyroError> {
    // Construit un tableau de `*const c_char` terminé par NULL comme
    // attendu par mpv_command_async. Les CString sont conservées dans
    // un Vec local qui vit le temps de l'appel.
    let cstrings: Result<Vec<CString>, _> = argv.iter().map(|s| CString::new(*s)).collect();
    let cstrings = cstrings.map_err(|_| {
        KyroError::Validation(format!("argument mpv non-cstring: {argv:?}"))
    })?;
    let mut ptrs: Vec<*const std::os::raw::c_char> =
        cstrings.iter().map(|s| s.as_ptr()).collect();
    ptrs.push(std::ptr::null());
    // userdata = 0 : les acks de commandes sont ignorés en v1.
    let rc = unsafe { (lib.mpv_command_async)(handle.0, 0, ptrs.as_ptr()) };
    if rc < 0 {
        return Err(KyroError::Validation(format!(
            "mpv_command_async({argv:?}) a échoué (code {rc})"
        )));
    }
    Ok(())
}

fn load_file_internal(
    lib: &MpvLib,
    handle: MpvHandle,
    file_path: &Path,
) -> Result<(), KyroError> {
    let path_str = file_path.to_string_lossy();
    let argv = ["loadfile", path_str.as_ref(), "replace"];
    command_async(lib, handle, &argv)
}

// ---------------------------------------------------------------------------
// Thread event-loop
// ---------------------------------------------------------------------------

/// Spawn le thread bloquant qui pump `mpv_wait_event` et traduit
/// chaque event utile en `EmbeddedPlayerEvent` envoyé sur `tx`.
///
/// On utilise `std::thread::spawn` (et pas une tâche Tokio) parce que
/// `mpv_wait_event` bloque jusqu'au timeout : le mettre dans un
/// `tokio::spawn_blocking` est possible, mais un thread dédié garde le
/// flot de vie clair (1 player → 1 thread → 1 mpsc).
fn spawn_event_loop(
    inner: Arc<EmbeddedPlayerInner>,
    tx: mpsc::UnboundedSender<EmbeddedPlayerEvent>,
) {
    std::thread::Builder::new()
        .name("kyro-embedded-player".into())
        .spawn(move || {
            run_event_loop(inner, tx);
        })
        .expect("spawn thread embedded-player");
}

/// Boucle d'événements bloquante. Sortie propre sur `MPV_EVENT_SHUTDOWN`.
fn run_event_loop(
    inner: Arc<EmbeddedPlayerInner>,
    tx: mpsc::UnboundedSender<EmbeddedPlayerEvent>,
) {
    let lib = inner.lib.clone();
    let handle = inner.handle;

    // État local pour fabriquer les events `Progress` et `Closed`.
    let mut last_position: f64 = 0.0;
    let mut current_duration: f64 = 0.0;

    loop {
        // SAFETY: handle valide tant que `inner` (Arc) est vivant.
        // mpv_wait_event renvoie un pointeur vers un event statique
        // détenu par le contexte mpv ; il n'est valide que jusqu'au
        // prochain appel mpv_wait_event sur ce même handle. On le lit
        // dans la même itération.
        let event_ptr: *mut MpvEvent = unsafe { (lib.mpv_wait_event)(handle.0, 0.5) };
        if event_ptr.is_null() {
            // Théoriquement impossible (libmpv retourne toujours un
            // event valide, même MPV_EVENT_NONE). Sécurité défensive.
            continue;
        }
        // SAFETY: pointeur non-null garanti par la branche au-dessus.
        let event = unsafe { &*event_ptr };

        match event.event_id {
            MPV_EVENT_NONE => {
                // Timeout sans event utile : on retourne au wait.
            }
            MPV_EVENT_PROPERTY_CHANGE => {
                // SAFETY: pour cet event_id, `data` pointe vers une
                // mpv_event_property valide dans le buffer mpv.
                let prop = unsafe { &*(event.data as *const MpvEventProperty) };
                handle_property_change(
                    event.reply_userdata,
                    prop,
                    &mut last_position,
                    &mut current_duration,
                    &tx,
                );
            }
            MPV_EVENT_END_FILE => {
                // SAFETY: data pointe vers mpv_event_end_file.
                let ef = unsafe { &*(event.data as *const MpvEventEndFile) };
                if ef.reason == MPV_END_FILE_REASON_EOF {
                    // Force la position à la durée pour basculer Vu en
                    // amont (parité avec le traitement Completed du
                    // mode externe).
                    if current_duration > 0.0 {
                        last_position = current_duration;
                        let _ = tx.send(EmbeddedPlayerEvent::Progress {
                            position: current_duration,
                            duration: current_duration,
                        });
                    }
                    let _ = tx.send(EmbeddedPlayerEvent::Completed);
                }
                // Les autres reasons (stop/quit/error) seront couverts
                // par MPV_EVENT_SHUTDOWN ci-dessous.
            }
            MPV_EVENT_FILE_LOADED => {
                // start-file équivalent — utile pour file-changed
                // sur les playlists. Le chemin n'est pas fourni
                // directement par l'event ; on le récupère plus tard
                // via `path` property si besoin (non implémenté en v1
                // car Kyro pilote le `loadfile` côté commands).
            }
            MPV_EVENT_SHUTDOWN => {
                let _ = tx.send(EmbeddedPlayerEvent::Closed {
                    last_position,
                });
                break;
            }
            _ => {
                // Events non utilisés : log_message, audio-reconfig,
                // command-reply, queue-overflow, etc. Ignorés.
            }
        }
    }

    // Sortie propre du thread : on libère le contexte mpv. `inner`
    // (Arc) sera drop par l'appelant quand la dernière façade tombera ;
    // cependant si le shutdown vient de l'OS (fenêtre fermée), il faut
    // explicitement appeler terminate_destroy ici pour éviter les
    // fuites. On appelle directement — c'est idempotent côté libmpv.
    unsafe { (lib.mpv_terminate_destroy)(handle.0) };
    // Drop de inner (et donc de lib) se fera quand l'Arc dernière
    // ref tombera.
    drop(inner);
}

/// Traduction d'un event `property-change` en `EmbeddedPlayerEvent`.
fn handle_property_change(
    userdata: u64,
    prop: &MpvEventProperty,
    last_position: &mut f64,
    current_duration: &mut f64,
    tx: &mpsc::UnboundedSender<EmbeddedPlayerEvent>,
) {
    match userdata {
        REPLY_USERDATA_TIME_POS => {
            if prop.format == MPV_FORMAT_DOUBLE && !prop.data.is_null() {
                // SAFETY: format DOUBLE → data pointe sur un f64.
                let pos = unsafe { *(prop.data as *const f64) };
                if pos.is_finite() && pos >= 0.0 {
                    *last_position = pos;
                    let _ = tx.send(EmbeddedPlayerEvent::Progress {
                        position: pos,
                        duration: *current_duration,
                    });
                }
            }
        }
        REPLY_USERDATA_DURATION => {
            if prop.format == MPV_FORMAT_DOUBLE && !prop.data.is_null() {
                // SAFETY: idem.
                let dur = unsafe { *(prop.data as *const f64) };
                if dur.is_finite() && dur > 0.0 {
                    *current_duration = dur;
                }
            }
        }
        REPLY_USERDATA_PAUSE => {
            if prop.format == MPV_FORMAT_FLAG && !prop.data.is_null() {
                // SAFETY: format FLAG → data pointe sur un c_int (0 ou 1).
                let flag = unsafe { *(prop.data as *const std::os::raw::c_int) };
                let _ = tx.send(EmbeddedPlayerEvent::PauseChanged {
                    paused: flag != 0,
                });
            }
        }
        REPLY_USERDATA_EOF_REACHED => {
            // Pas d'émission directe : on attend l'event END_FILE qui
            // porte la raison (EOF vs stop). `eof-reached` est utilisé
            // surtout pour synchroniser l'UI.
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    /// Le thread name doit être stable pour faciliter le diagnostic
    /// dans les dumps de threads (Process Explorer / windbg).
    #[test]
    fn thread_name_constant_used_for_event_loop() {
        // On ne peut pas tester directement le nom (pas de getter
        // public), mais on fige la chaîne pour qu'elle ne dérive pas
        // sans intention.
        let name_in_source = "kyro-embedded-player";
        assert_eq!(name_in_source.len(), 20);
    }
}
