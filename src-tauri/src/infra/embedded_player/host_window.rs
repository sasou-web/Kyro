//! Fenêtre Win32 enfant qui héberge le rendu libmpv.
//!
//! ## Pourquoi un HWND séparé plutôt que celui de la WebView ?
//!
//! Dans la première version de ce module, libmpv recevait directement
//! le HWND de la fenêtre Tauri. Problème : Tauri 2 utilise WebView2
//! (Chromium Edge) qui pose son propre HWND child opaque au-dessus du
//! HWND parent. libmpv rendait sur le parent → la WebView masquait
//! intégralement la vidéo. Astuce du transparent CSS impossible :
//! WebView2 désactive son rendu hors-écran quand la composition de
//! fenêtre n'est pas explicitement gérée (ce qui demande des
//! manipulations DComp au-delà de Tauri).
//!
//! La solution éprouvée (Hayase, MPC-Qt, mpv-easy, OBS embed) est de
//! **créer un troisième HWND** — un child Win32 vide — et de demander
//! à libmpv de rendre dans **celui-ci**. La WebView2 et le HWND du
//! player sont alors deux frères child du même parent (la fenêtre
//! Kyro main), et le z-order détermine qui est visible :
//!
//! * z-order normal → WebView devant, vidéo cachée derrière
//! * appel `SetWindowPos(HWND_TOP)` → vidéo devant la WebView
//!   (mais la WebView dessine quand même son fond → on ne peut pas
//!   faire d'overlay UI)
//!
//! Pour avoir l'UI Svelte par-dessus la vidéo (timeline, contrôles,
//! titre), Kyro positionne le HWND vidéo **dans une zone précise**
//! laissée libre par la WebView. Le frontend transmet les coordonnées
//! d'une `<div>` vide (le "trou pour la vidéo") via la commande
//! `embedded_player_set_geometry`. Le HWND vidéo est dimensionné
//! pour exactement remplir cette zone, et la WebView dessine ses
//! contrôles tout autour (et même par-dessus si la div a
//! `pointer-events: none` sur les contrôles, qui flottent en
//! position absolute).
//!
//! ## API publique
//!
//! * [`HostWindow::create`] — crée le HWND child invisible, prêt à
//!   être passé à libmpv via `wid=<hwnd>`.
//! * [`HostWindow::set_geometry`] — repositionne / redimensionne le
//!   HWND. Appelé en réaction au resize de la div Svelte qui sert
//!   de zone vidéo.
//! * [`HostWindow::set_visible`] — affiche / masque le HWND
//!   (équivalent fermeture sans détruire l'instance libmpv).
//! * [`HostWindow::handle`] — retourne le HWND brut (`isize` pour
//!   Send-safety) pour le passage à libmpv.
//! * `Drop` — `DestroyWindow` automatique à la fin de la session.

#![cfg(target_os = "windows")]

use std::sync::OnceLock;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetStockObject, HBRUSH, BLACK_BRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassExW, SetWindowPos,
    ShowWindow, CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, HMENU, HWND_TOP, SWP_NOACTIVATE,
    SW_HIDE, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE, WNDCLASSEXW, WS_CHILD,
    WS_CLIPCHILDREN, WS_CLIPSIBLINGS,
};

use crate::error::KyroError;

/// Nom de classe Win32 enregistrée pour la fenêtre vidéo Kyro.
/// Une seule fois par process — `RegisterClassExW` retourne un atome
/// existant si déjà enregistré.
const HOST_CLASS_NAME: PCWSTR = w!("KyroEmbeddedPlayerHost");

/// Drapeau d'enregistrement de la classe Win32. `OnceLock` garantit
/// qu'on appelle `RegisterClassExW` au plus une fois par process,
/// peu importe combien de sessions de lecture sont ouvertes.
static CLASS_REGISTERED: OnceLock<()> = OnceLock::new();

/// Callback global appelé par la wnd proc à chaque clic / double-clic
/// reçu par le HWND vidéo. Permet à la couche commands de remonter
/// l'info au frontend via un event Tauri sans qu'`host_window` ait à
/// connaître `tauri::AppHandle`.
///
/// `Click` = simple clic gauche → toggle pause souhaité côté UI.
/// `DoubleClick` = bascule plein écran système.
#[derive(Debug, Clone, Copy)]
pub enum HostInputEvent {
    Click,
    DoubleClick,
}

type HostInputCallback = dyn Fn(HostInputEvent) + Send + Sync + 'static;

static INPUT_CALLBACK: OnceLock<Box<HostInputCallback>> = OnceLock::new();

/// Enregistre le callback global appelé par la wnd proc à chaque
/// clic souris reçu par le HWND vidéo. Idempotent : un seul callback
/// pour la durée du process. Doit être posé tôt (idéalement au
/// démarrage Tauri).
pub fn set_input_callback<F>(callback: F)
where
    F: Fn(HostInputEvent) + Send + Sync + 'static,
{
    let _ = INPUT_CALLBACK.set(Box::new(callback));
}

/// HWND child Win32 hébergeant le rendu libmpv.
///
/// L'enveloppe garde son HWND et le détruit dans `Drop`. Volontairement
/// pas `Clone` : la possession est unique côté `EmbeddedPlayer`.
pub struct HostWindow {
    hwnd: HWND,
}

// SAFETY: HWND est un handle opaque manipulé via les API Win32 qui
// sont thread-safe au niveau message-loop. La struct est consommée
// uniquement via `&self` après création — toutes les mutations vont
// par message Windows à la pompe Win32 du process.
unsafe impl Send for HostWindow {}
unsafe impl Sync for HostWindow {}

impl HostWindow {
    /// Crée une fenêtre Win32 enfant invisible sous `parent_hwnd`.
    ///
    /// La fenêtre est créée :
    /// - en `WS_CHILD` (parent défini),
    /// - en `WS_CLIPSIBLINGS` + `WS_CLIPCHILDREN` (le rendu mpv ne
    ///   déborde pas sur la WebView frère),
    /// - **sans** `WS_VISIBLE` initial : l'appelant choisit quand la
    ///   montrer via `set_visible(true)` après avoir appelé
    ///   `set_geometry` au moins une fois.
    ///
    /// L'écouteur de messages utilise `DefWindowProcW` (pas de
    /// traitement custom : libmpv s'occupe du rendu via `wid`).
    pub fn create(parent_hwnd: isize) -> Result<Self, KyroError> {
        ensure_class_registered()?;

        let parent = HWND(parent_hwnd as *mut _);

        // SAFETY: tous les pointeurs sont valides ; les flags choisis
        // sont documentés dans MSDN et compatibles Windows 10+.
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                HOST_CLASS_NAME,
                w!("KyroEmbeddedPlayer"),
                WS_CHILD | WS_CLIPSIBLINGS | WS_CLIPCHILDREN,
                0, // x, y, width, height : (0, 0, 0, 0) ; le frontend
                0, // appellera set_geometry juste après création.
                0,
                0,
                parent,
                HMENU(std::ptr::null_mut()),
                module_handle()?,
                None,
            )
        }
        .map_err(|e| {
            KyroError::Validation(format!("CreateWindowExW host player a échoué: {e}"))
        })?;

        if hwnd.0.is_null() {
            return Err(KyroError::Validation(
                "CreateWindowExW a retourné NULL".to_string(),
            ));
        }
        Ok(Self { hwnd })
    }

    /// Retourne le HWND sous forme `isize`, prêt à être passé à
    /// libmpv via la propriété `wid`.
    pub fn handle(&self) -> isize {
        self.hwnd.0 as isize
    }

    /// Repositionne et redimensionne le HWND.
    ///
    /// Coordonnées en **pixels physiques** dans le repère du parent
    /// (la fenêtre Kyro main). Le frontend doit multiplier les
    /// coordonnées CSS par `window.devicePixelRatio` avant l'invoke
    /// pour obtenir les bonnes valeurs sur écran HiDPI.
    ///
    /// Passe `HWND_TOP` **sans** `SWP_NOZORDER` : le HWND vidéo doit
    /// être placé au-dessus de la WebView2 dans le z-order des frères,
    /// sinon il est masqué par celle-ci (qui occupe toute la zone
    /// client de la fenêtre Kyro). `SWP_NOACTIVATE` empêche le vol
    /// de focus pendant que l'utilisateur tape dans Kyro.
    pub fn set_geometry(
        &self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Result<(), KyroError> {
        // SAFETY: hwnd valide. SWP_NOACTIVATE conserve le focus
        // courant ; on omet SWP_NOZORDER pour autoriser HWND_TOP à
        // remonter le child au-dessus de la WebView2.
        let _ = unsafe {
            SetWindowPos(
                self.hwnd,
                HWND_TOP,
                x,
                y,
                width.max(1),
                height.max(1),
                SWP_NOACTIVATE,
            )
        };
        Ok(())
    }

    /// Affiche ou masque le HWND (équivalent show/hide).
    ///
    /// Quand l'utilisateur ferme le mini-player en mode PiP, on
    /// préfère masquer plutôt que détruire — l'instance libmpv reste
    /// vivante et la lecture peut reprendre instantanément. La
    /// destruction effective se fait via `embedded_player_quit`.
    pub fn set_visible(&self, visible: bool) -> Result<(), KyroError> {
        // SAFETY: hwnd valide.
        let cmd = if visible {
            SW_SHOWNOACTIVATE
        } else {
            SW_HIDE
        };
        let _ = unsafe { ShowWindow(self.hwnd, cmd) };
        Ok(())
    }
}

impl Drop for HostWindow {
    fn drop(&mut self) {
        // ATTENTION : DestroyWindow DOIT être appelé sur le thread
        // qui possède la fenêtre (le thread UI Win32). Si l'instance
        // tombe depuis un worker Tokio, l'appel direct gèlerait le
        // thread (Windows attend une réponse du thread propriétaire,
        // qui ne pompe pas les messages depuis cet appel).
        //
        // Solution éprouvée : on utilise `PostMessage` avec le message
        // standard `WM_CLOSE` qui sera traité par la pompe de messages
        // de Tauri sur le thread UI principal. La fenêtre se détruit
        // alors proprement via `DefWindowProcW` qui appelle
        // `DestroyWindow` en interne.
        if !self.hwnd.0.is_null() {
            use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
            // SAFETY: hwnd valide ; PostMessage est explicitement
            // conçu pour être appelé depuis n'importe quel thread.
            unsafe {
                let _ = PostMessageW(self.hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers internes
// ---------------------------------------------------------------------------

fn ensure_class_registered() -> Result<(), KyroError> {
    if CLASS_REGISTERED.get().is_some() {
        return Ok(());
    }

    let hinstance = module_handle()?;
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        // CS_DBLCLKS : autorise WM_LBUTTONDBLCLK pour le double-clic
        // = bascule plein écran système.
        style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
        lpfnWndProc: Some(host_wnd_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinstance.into(),
        hIcon: Default::default(),
        hCursor: Default::default(),
        hbrBackground: HBRUSH(unsafe { GetStockObject(BLACK_BRUSH).0 }),
        lpszMenuName: PCWSTR::null(),
        lpszClassName: HOST_CLASS_NAME,
        hIconSm: Default::default(),
    };

    // SAFETY: structure entièrement renseignée, pointeurs statiques
    // valides pour la durée de vie du process.
    let atom = unsafe { RegisterClassExW(&wc) };
    if atom == 0 {
        // Cas particulier : la classe peut déjà être enregistrée si
        // OnceLock a raté un appel concurrent. On vérifie la dernière
        // erreur Windows pour décider.
        let err = unsafe { windows::Win32::Foundation::GetLastError() };
        const ERROR_CLASS_ALREADY_EXISTS: u32 = 1410;
        if err.0 != ERROR_CLASS_ALREADY_EXISTS {
            return Err(KyroError::Validation(format!(
                "RegisterClassExW a échoué (code {})",
                err.0
            )));
        }
    }
    let _ = CLASS_REGISTERED.set(());
    Ok(())
}

fn module_handle() -> Result<windows::Win32::Foundation::HMODULE, KyroError> {
    // SAFETY: GetModuleHandleW(NULL) retourne le handle du process
    // courant, sans incrémenter de refcount → pas de free à faire.
    let res = unsafe { GetModuleHandleW(PCWSTR::null()) };
    res.map_err(|e| {
        KyroError::Validation(format!("GetModuleHandleW a échoué: {e}"))
    })
}

/// Window procedure : intercepte `WM_CLOSE` (cf. Drop) et les clics
/// souris pour les remonter au frontend via le callback global. Tout
/// le reste va à `DefWindowProcW` (libmpv s'occupe du rendu).
unsafe extern "system" fn host_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{
        WM_CLOSE, WM_LBUTTONDBLCLK, WM_LBUTTONUP,
    };
    match msg {
        WM_CLOSE => {
            // SAFETY: hwnd appartient au thread courant (la wnd proc est
            // toujours invoquée sur le thread UI propriétaire).
            let _ = DestroyWindow(hwnd);
            return LRESULT(0);
        }
        WM_LBUTTONUP => {
            if let Some(cb) = INPUT_CALLBACK.get() {
                cb(HostInputEvent::Click);
            }
            // On laisse aussi DefWindowProcW gérer le message standard
            // (focus, etc.), pas de return court-circuité.
        }
        WM_LBUTTONDBLCLK => {
            if let Some(cb) = INPUT_CALLBACK.get() {
                cb(HostInputEvent::DoubleClick);
            }
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}
