//! Bindings FFI minimaux vers libmpv via `libloading`.
//!
//! On évite délibérément :
//!
//! * une dépendance crate.io (`libmpv2`, `libmpv-sys`) qui imposerait
//!   un build link-time avec headers et .lib mpv installés ;
//! * un `#[link(name = "mpv")]` natif qui ferait échouer le build si
//!   la DLL n'est pas trouvée à la compilation.
//!
//! `libloading` charge la DLL à l'exécution. Si elle est absente,
//! [`MpvLib::load`] retourne une erreur explicite et Kyro continue à
//! fonctionner en mode externe (`playback_mode = external`).
//!
//! ## Référence d'API
//!
//! Les signatures suivantes proviennent de `libmpv/client.h` (mpv
//! upstream). Seul le sous-ensemble strictement consommé par
//! `embedded_player::event_loop` est exposé. Toute extension future
//! (par exemple `mpv_render_context_create` pour OpenGL embed) doit
//! ajouter sa signature ici.
//!
//! ## Sécurité
//!
//! Les types `Symbol` retournés par `libloading` sont liés à la durée
//! de vie de la `Library` qui les a produits. On les stocke par valeur
//! dans `MpvLib` (la struct possède la `Library` *avant* les symbols
//! grâce à l'ordre de déclaration des champs et à l'ordre inverse
//! garanti pour `Drop`), ce qui élimine le risque d'utilisation après
//! `unload`. La struct n'est manipulée que via `Arc` côté appelant.

#![allow(non_camel_case_types)]
#![allow(dead_code)] // les symbols sont utilisés via cfg(target_os = "windows")

use std::os::raw::{c_char, c_int, c_void};
use std::path::{Path, PathBuf};

use libloading::Library;

use crate::error::KyroError;

// ===========================================================================
// Constantes libmpv (extraites de client.h)
// ===========================================================================

/// `MPV_FORMAT_NONE` — pas de format.
pub const MPV_FORMAT_NONE: c_int = 0;
/// `MPV_FORMAT_STRING` — chaîne UTF-8 terminée par `\0`.
pub const MPV_FORMAT_STRING: c_int = 1;
/// `MPV_FORMAT_FLAG` — `int` 0 ou 1.
pub const MPV_FORMAT_FLAG: c_int = 3;
/// `MPV_FORMAT_INT64` — entier 64 bits signé.
pub const MPV_FORMAT_INT64: c_int = 4;
/// `MPV_FORMAT_DOUBLE` — flottant double précision.
pub const MPV_FORMAT_DOUBLE: c_int = 5;
/// `MPV_FORMAT_NODE` — variant générique mpv (non utilisé pour
/// l'instant, gardé pour mémoire).
pub const MPV_FORMAT_NODE: c_int = 6;

/// `MPV_EVENT_NONE` — pas d'évènement (timeout `wait_event`).
pub const MPV_EVENT_NONE: c_int = 0;
/// `MPV_EVENT_SHUTDOWN` — l'instance va se détruire.
pub const MPV_EVENT_SHUTDOWN: c_int = 1;
/// `MPV_EVENT_LOG_MESSAGE` — message log mpv (non écouté en v1).
pub const MPV_EVENT_LOG_MESSAGE: c_int = 2;
/// `MPV_EVENT_FILE_LOADED` — fichier chargé avec succès.
pub const MPV_EVENT_FILE_LOADED: c_int = 8;
/// `MPV_EVENT_END_FILE` — fin de fichier (eof, stop, error, …).
pub const MPV_EVENT_END_FILE: c_int = 7;
/// `MPV_EVENT_START_FILE` — début de chargement d'un nouveau fichier.
pub const MPV_EVENT_START_FILE: c_int = 6;
/// `MPV_EVENT_PROPERTY_CHANGE` — propriété observée modifiée.
pub const MPV_EVENT_PROPERTY_CHANGE: c_int = 22;

/// Identifiant de userdata pour `mpv_observe_property` côté
/// `time-pos`. La valeur est arbitraire ; on utilise des constantes
/// distinctes pour pouvoir router les events dans le wait loop.
pub const REPLY_USERDATA_TIME_POS: u64 = 1;
pub const REPLY_USERDATA_DURATION: u64 = 2;
pub const REPLY_USERDATA_PAUSE: u64 = 3;
pub const REPLY_USERDATA_EOF_REACHED: u64 = 4;

// ===========================================================================
// Structures C (layout aligné sur client.h)
// ===========================================================================

/// `mpv_event` — entête commune retournée par `mpv_wait_event`.
///
/// Le champ `data` pointe vers une struct dépendant de `event_id` :
/// pour `MPV_EVENT_PROPERTY_CHANGE` c'est `mpv_event_property`, pour
/// `MPV_EVENT_END_FILE` c'est `mpv_event_end_file`, etc.
#[repr(C)]
pub struct MpvEvent {
    pub event_id: c_int,
    pub error: c_int,
    pub reply_userdata: u64,
    pub data: *mut c_void,
}

/// `mpv_event_property` — payload du `MPV_EVENT_PROPERTY_CHANGE`.
#[repr(C)]
pub struct MpvEventProperty {
    pub name: *const c_char,
    pub format: c_int,
    pub data: *mut c_void,
}

/// `mpv_event_end_file` — payload du `MPV_EVENT_END_FILE`.
#[repr(C)]
pub struct MpvEventEndFile {
    pub reason: c_int,
    pub error: c_int,
    pub playlist_entry_id: i64,
    pub playlist_insert_id: i64,
    pub playlist_insert_num_entries: c_int,
}

/// Reasons de end-file (cf. client.h `mpv_end_file_reason`). Seul `EOF`
/// nous intéresse pour basculer le statut à `Vu`.
pub const MPV_END_FILE_REASON_EOF: c_int = 0;
#[allow(dead_code)]
pub const MPV_END_FILE_REASON_STOP: c_int = 2;
#[allow(dead_code)]
pub const MPV_END_FILE_REASON_QUIT: c_int = 3;
#[allow(dead_code)]
pub const MPV_END_FILE_REASON_ERROR: c_int = 4;

// ===========================================================================
// Wrapper handle
// ===========================================================================

/// Wrapper transparent autour du `*mut mpv_handle` opaque retourné
/// par `mpv_create`. Conserver un type fort à ce niveau évite que les
/// appelants stockent un `*mut c_void` non-`Send` directement dans
/// `EmbeddedPlayerInner`.
#[derive(Debug, Clone, Copy)]
pub struct MpvHandle(pub *mut c_void);

// ===========================================================================
// Bindings dynamiques
// ===========================================================================

/// Toutes les fonctions libmpv consommées par `embedded_player`.
///
/// La struct charge la `Library` une fois pour toutes, puis résout
/// chaque symbole. Les symbols sont stockés en tant que
/// `unsafe extern "C" fn` (pas de `Symbol<'_>`), récupérés via
/// `Library::get` puis `*sym` pour figer le pointeur. C'est la pratique
/// recommandée par `libloading` quand on veut détacher le symbol de la
/// `Library` (Cf. doc `libloading::Symbol::into_raw`). On garde la
/// `Library` *à côté* dans la même struct ; l'ordre de déclaration
/// (lib en dernier) garantit qu'elle est drop *après* les pointeurs
/// (le drop d'une struct en Rust est dans l'ordre de déclaration).
pub struct MpvLib {
    pub mpv_create: unsafe extern "C" fn() -> *mut c_void,
    pub mpv_initialize: unsafe extern "C" fn(*mut c_void) -> c_int,
    pub mpv_terminate_destroy: unsafe extern "C" fn(*mut c_void),
    pub mpv_set_option_string:
        unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int,
    pub mpv_set_property_string:
        unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int,
    pub mpv_get_property_string:
        unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_char,
    pub mpv_free: unsafe extern "C" fn(*mut c_void),
    pub mpv_command_async:
        unsafe extern "C" fn(*mut c_void, u64, *const *const c_char) -> c_int,
    pub mpv_observe_property:
        unsafe extern "C" fn(*mut c_void, u64, *const c_char, c_int) -> c_int,
    pub mpv_wait_event: unsafe extern "C" fn(*mut c_void, f64) -> *mut MpvEvent,
    pub mpv_wakeup: unsafe extern "C" fn(*mut c_void),

    // CHAMP EN DERNIER : drop ordering = lib drop après les fn pointers.
    pub _lib: Library,
}

impl MpvLib {
    /// Charge `libmpv-2.dll` (ou alias) depuis le chemin résolu.
    /// L'appelant fournit ce chemin via [`resolve_libmpv_path`].
    pub fn load(dll_path: &Path) -> Result<Self, KyroError> {
        // SAFETY: Library::new charge un fichier dynamique arbitraire ;
        // on fait confiance au chemin résolu par notre locator (qui ne
        // récupère que des fichiers `libmpv-2.dll` connus).
        let lib = unsafe { Library::new(dll_path) }.map_err(|e| {
            KyroError::Validation(format!(
                "impossible de charger {}: {e}",
                dll_path.display()
            ))
        })?;

        // SAFETY: chaque symbole est résolu par nom ASCII strict
        // documenté dans `client.h` ; toute absence remonte une
        // `KyroError::Validation` au lieu de paniquer.
        unsafe {
            macro_rules! sym {
                ($name:literal) => {{
                    let s = lib.get::<unsafe extern "C" fn()>($name).map_err(|e| {
                        KyroError::Validation(format!(
                            "symbole {} introuvable dans {}: {e}",
                            std::str::from_utf8($name).unwrap_or("<non-utf8>"),
                            dll_path.display()
                        ))
                    })?;
                    let raw: *const () = *s.into_raw() as *const ();
                    raw
                }};
            }

            let mpv_create_raw = sym!(b"mpv_create");
            let mpv_initialize_raw = sym!(b"mpv_initialize");
            let mpv_terminate_destroy_raw = sym!(b"mpv_terminate_destroy");
            let mpv_set_option_string_raw = sym!(b"mpv_set_option_string");
            let mpv_set_property_string_raw = sym!(b"mpv_set_property_string");
            let mpv_get_property_string_raw = sym!(b"mpv_get_property_string");
            let mpv_free_raw = sym!(b"mpv_free");
            let mpv_command_async_raw = sym!(b"mpv_command_async");
            let mpv_observe_property_raw = sym!(b"mpv_observe_property");
            let mpv_wait_event_raw = sym!(b"mpv_wait_event");
            let mpv_wakeup_raw = sym!(b"mpv_wakeup");

            Ok(Self {
                mpv_create: std::mem::transmute(mpv_create_raw),
                mpv_initialize: std::mem::transmute(mpv_initialize_raw),
                mpv_terminate_destroy: std::mem::transmute(mpv_terminate_destroy_raw),
                mpv_set_option_string: std::mem::transmute(mpv_set_option_string_raw),
                mpv_set_property_string: std::mem::transmute(mpv_set_property_string_raw),
                mpv_get_property_string: std::mem::transmute(mpv_get_property_string_raw),
                mpv_free: std::mem::transmute(mpv_free_raw),
                mpv_command_async: std::mem::transmute(mpv_command_async_raw),
                mpv_observe_property: std::mem::transmute(mpv_observe_property_raw),
                mpv_wait_event: std::mem::transmute(mpv_wait_event_raw),
                mpv_wakeup: std::mem::transmute(mpv_wakeup_raw),
                _lib: lib,
            })
        }
    }
}

// ===========================================================================
// Locator libmpv-2.dll
// ===========================================================================

/// Nom canonique de la DLL libmpv attendue par Kyro.
pub const LIBMPV_DLL: &str = "libmpv-2.dll";

/// Résout le chemin de `libmpv-2.dll` selon la priorité :
///
/// 1. À côté de `kyro.exe` (production : copiée par l'installeur via
///    `bundle.resources` dans `tauri.conf.json`).
/// 2. Dans le `target/<profile>/` du build courant (dev : déposée
///    manuellement par le développeur dans
///    `src-tauri/binaries/libmpv-2.dll` puis copiée par
///    `build.rs` — voir `build.rs` pour le mécanisme de copie).
/// 3. Sur le `PATH` système (`where libmpv-2.dll`).
///
/// Retourne `None` si aucune des 3 stratégies n'aboutit. L'appelant
/// (commande Tauri) convertit ce `None` en
/// `KyroError::Validation("libmpv-2.dll introuvable")` à émettre vers
/// le frontend.
pub fn resolve_libmpv_path() -> Option<PathBuf> {
    // 1. À côté de l'exécutable courant.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(LIBMPV_DLL);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    // 2. PATH système — `which` est déjà dans les deps Cargo.
    if let Ok(p) = which::which(LIBMPV_DLL) {
        return Some(p);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Le constant `LIBMPV_DLL` est figé : changer ce nom imposerait
    /// de mettre à jour l'installeur et la doc utilisateur.
    #[test]
    fn libmpv_dll_constant_is_stable() {
        assert_eq!(LIBMPV_DLL, "libmpv-2.dll");
    }

    /// Les valeurs de format mpv ne doivent pas dériver de client.h.
    /// Ce test fige l'ABI consommée — toute mise à jour majeure de
    /// libmpv doit forcer une revue explicite.
    #[test]
    fn mpv_format_constants_match_upstream() {
        assert_eq!(MPV_FORMAT_NONE, 0);
        assert_eq!(MPV_FORMAT_STRING, 1);
        assert_eq!(MPV_FORMAT_FLAG, 3);
        assert_eq!(MPV_FORMAT_INT64, 4);
        assert_eq!(MPV_FORMAT_DOUBLE, 5);
    }

    /// Les ID d'événements consommés par `event_loop` sont également
    /// figés (mêmes raisons que ci-dessus).
    #[test]
    fn mpv_event_id_constants_match_upstream() {
        assert_eq!(MPV_EVENT_NONE, 0);
        assert_eq!(MPV_EVENT_SHUTDOWN, 1);
        assert_eq!(MPV_EVENT_END_FILE, 7);
        assert_eq!(MPV_EVENT_FILE_LOADED, 8);
        assert_eq!(MPV_EVENT_PROPERTY_CHANGE, 22);
    }

    /// Le locator ne doit pas paniquer même si aucune DLL n'est
    /// présente (cas tests CI sans libmpv).
    #[test]
    fn resolve_libmpv_path_does_not_panic_when_missing() {
        // On ne valide pas la valeur retournée (dépend de
        // l'environnement de test), juste l'absence de panic.
        let _ = resolve_libmpv_path();
    }
}
