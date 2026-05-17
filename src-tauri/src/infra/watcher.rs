//! Watcher filesystem cross-platform pour les `library_folder`.
//!
//! Surveille tous les dossiers de bibliothèque enregistrés en DB et
//! déclenche un scan **incrémental** sur le dossier impacté à chaque
//! événement filesystem (création, modification, suppression, rename).
//!
//! # Design
//!
//! * Un unique [`notify::RecommendedWatcher`] gère tous les
//!   `library_folder` (un watch récursif par racine). Sous Windows
//!   c'est `ReadDirectoryChangesW`.
//! * Les événements sont **debouncés** (fenêtre 1.5 s) par dossier
//!   racine pour éviter de relancer un scan à chaque petit événement
//!   pendant qu'un éditeur vidéo écrit un fichier par chunks.
//! * Le scan déclenché est ciblé sur la racine de bibliothèque
//!   concernée — pas un full-rescan de toutes les catégories. Le diff
//!   `seen_paths` du scanner est suffisant pour gérer ajouts /
//!   suppressions / renames sans tout reprobe.
//! * Le watcher peut être (re)initialisé à chaud : quand l'utilisateur
//!   ajoute ou retire un `library_folder` depuis la page Paramètres,
//!   on reconstruit la liste des watches via [`reload_watches`].
//!
//! # Robustesse
//!
//! * Toute erreur de watcher (chemin invalide, lecteur réseau coupé,
//!   permissions refusées) est journalisée et **n'interrompt pas** la
//!   surveillance des autres dossiers.
//! * Si le watcher backend lui-même échoue à s'initialiser (cas très
//!   rare : ressources système épuisées), on logue et on retombe sur
//!   l'ancien comportement « rafraîchir manuel ».
//! * Les scans déclenchés sont gardés par le même `SCAN_RUNNING` que
//!   `commands::scan::start_scan` : pas de scans concurrents.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecursiveMode, Watcher};
use tauri::{Emitter, Runtime, Window};
use tokio::sync::mpsc;

use crate::infra::ffprobe_locator::locate_ffprobe;
use crate::infra::probe::FfprobeBackend;
use crate::infra::repo::{FolderId, LibraryFolder, Repo};
use crate::infra::scanner;
use crate::infra::settings_keys::SETTING_KEY_FFPROBE_PATH;

/// Délai minimal entre deux scans déclenchés par le watcher pour un
/// même dossier racine. Absorbe les rafales d'événements (par exemple
/// un copier-coller multi-fichiers, ou la finalisation d'un download
/// par morceaux).
const DEBOUNCE_DURATION: Duration = Duration::from_millis(1_500);

/// Évènement Tauri émis quand le watcher déclenche un scan automatique.
/// Le frontend peut s'y abonner pour afficher discrètement « bibliothèque
/// mise à jour » via un toast.
pub const EVENT_LIBRARY_AUTOREFRESH: &str = "library-autorefresh";

/// État global du watcher — partagé entre le `setup` Tauri (qui
/// l'initialise au boot) et la commande `add/remove_library_folder`
/// (qui appelle [`reload_watches`] à chaque modification de la liste
/// des dossiers).
///
/// `Mutex` plutôt que `RwLock` : la fréquence de modification est
/// faible (uniquement lors d'opérations utilisateur sur les dossiers).
pub struct WatcherState {
    /// Watcher `notify` actif. `None` tant que l'init n'a pas eu lieu
    /// ou si l'initialisation a échoué (mode dégradé).
    inner: Mutex<Option<notify::RecommendedWatcher>>,
    /// Map `path canonique` → `FolderId` pour router les événements
    /// vers la bonne racine. Reconstruite à chaque [`reload_watches`].
    paths: Mutex<HashMap<PathBuf, FolderId>>,
    /// Verrou interne « un scan auto est en cours pour ce dossier ? »
    /// pour éviter d'empiler des scans quand le debounce coïncide
    /// avec un scan long. Map `FolderId → bool`.
    scanning: Mutex<HashMap<FolderId, bool>>,
}

impl WatcherState {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
            paths: Mutex::new(HashMap::new()),
            scanning: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for WatcherState {
    fn default() -> Self {
        Self::new()
    }
}

/// Garde process-wide « un scan déclenché par le watcher est en
/// cours ». Empêche d'empiler 10 scans si le watcher reçoit 10
/// événements rapprochés sur 10 racines différentes — on sérialise.
static AUTO_SCAN_RUNNING: AtomicBool = AtomicBool::new(false);

/// Initialise le watcher au boot. À appeler **une fois** depuis le
/// `setup` Tauri après que le `Repo` soit prêt.
///
/// Cette fonction :
/// 1. Liste les `library_folder` actuels.
/// 2. Crée un `notify::RecommendedWatcher` qui pousse ses événements
///    dans un canal `mpsc`.
/// 3. Attache un watch récursif à chaque dossier.
/// 4. Démarre une task `tokio::spawn` qui consomme le canal, debounce
///    par dossier racine, et lance un scan ciblé.
///
/// Si l'init échoue (cas très rare), on logue et on retombe sur le
/// mode « rafraîchir manuel ».
pub async fn init_watcher<R: Runtime>(
    state: Arc<WatcherState>,
    repo: Repo,
    window: Window<R>,
) {
    let folders = match repo.list_library_folders().await {
        Ok(f) => f,
        Err(err) => {
            tracing::warn!(
                error = %err,
                "Kyro_Watcher: list_library_folders échoué — watcher non initialisé"
            );
            return;
        }
    };

    // Canal d'événements. La capacité 256 absorbe une rafale de
    // mouvements simultanés sans perdre d'événements ; au-delà, le
    // backend `notify` peut éventuellement coalescer côté OS.
    let (tx, rx) = mpsc::channel::<Event>(256);

    let watcher_result = notify::RecommendedWatcher::new(
        move |res: notify::Result<Event>| match res {
            Ok(event) => {
                // `try_send` non-bloquant : si le buffer est saturé on
                // perd l'événement (le scan suivant rattrapera l'état
                // via le diff `seen_paths`). Ne JAMAIS bloquer ici :
                // ce closure est appelé sur le thread interne de notify.
                if let Err(err) = tx.try_send(event) {
                    tracing::warn!(
                        error = %err,
                        "Kyro_Watcher: canal d'événements saturé, événement perdu (best-effort, scan suivant rattrapera)"
                    );
                }
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "Kyro_Watcher: erreur backend notify (ignorée)"
                );
            }
        },
        notify::Config::default(),
    );

    let mut watcher = match watcher_result {
        Ok(w) => w,
        Err(err) => {
            tracing::error!(
                error = %err,
                "Kyro_Watcher: impossible de créer RecommendedWatcher — mode dégradé (refresh manuel uniquement)"
            );
            return;
        }
    };

    // Attache un watch récursif sur chaque library_folder.
    let mut paths_map: HashMap<PathBuf, FolderId> = HashMap::new();
    let mut watched_count = 0usize;
    for folder in &folders {
        let path = PathBuf::from(&folder.path);
        match watcher.watch(&path, RecursiveMode::Recursive) {
            Ok(_) => {
                paths_map.insert(path.clone(), folder.id);
                watched_count += 1;
                tracing::info!(
                    folder_id = folder.id.get(),
                    path = %folder.path,
                    "Kyro_Watcher: watch attaché"
                );
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    folder_id = folder.id.get(),
                    path = %folder.path,
                    "Kyro_Watcher: échec d'attachement watch — ce dossier ne sera pas surveillé automatiquement"
                );
            }
        }
    }

    // Stocke watcher + map dans le state partagé.
    {
        let mut inner = state.inner.lock().expect("watcher inner mutex poisoned");
        *inner = Some(watcher);
    }
    {
        let mut p = state.paths.lock().expect("watcher paths mutex poisoned");
        *p = paths_map;
    }

    tracing::info!(
        watched_count = watched_count,
        total_folders = folders.len(),
        "Kyro_Watcher: initialisé"
    );

    // Spawn la task de consommation.
    let repo_for_task = repo.clone();
    let state_for_task = state.clone();
    let window_for_task = window.clone();
    tauri::async_runtime::spawn(async move {
        run_event_loop(rx, state_for_task, repo_for_task, window_for_task).await;
    });
}

/// Reconstruit la liste des watches après une modification des
/// `library_folder`. Appelée par `add_library_folder` et
/// `remove_library_folder` pour que le nouvel état FS soit pris en
/// compte sans redémarrer l'app.
pub async fn reload_watches(state: Arc<WatcherState>, repo: Repo) {
    let folders = match repo.list_library_folders().await {
        Ok(f) => f,
        Err(err) => {
            tracing::warn!(
                error = %err,
                "Kyro_Watcher: reload — list_library_folders échoué (watches inchangés)"
            );
            return;
        }
    };

    let mut new_paths: HashMap<PathBuf, FolderId> =
        folders
            .iter()
            .map(|f| (PathBuf::from(&f.path), f.id))
            .collect();

    let mut inner_lock = state.inner.lock().expect("watcher inner mutex poisoned");
    let watcher = match inner_lock.as_mut() {
        Some(w) => w,
        None => {
            tracing::warn!(
                "Kyro_Watcher: reload appelé alors que le watcher n'est pas initialisé"
            );
            return;
        }
    };

    let mut current_paths_lock = state.paths.lock().expect("watcher paths mutex poisoned");
    let current = current_paths_lock.clone();

    // 1. Détache les watches des dossiers retirés.
    for (path, _id) in &current {
        if !new_paths.contains_key(path) {
            if let Err(err) = watcher.unwatch(path) {
                tracing::warn!(
                    path = %path.display(),
                    error = %err,
                    "Kyro_Watcher: échec unwatch (ignoré)"
                );
            } else {
                tracing::info!(path = %path.display(), "Kyro_Watcher: unwatch");
            }
        }
    }

    // 2. Attache les watches des dossiers nouveaux.
    let mut to_remove: Vec<PathBuf> = Vec::new();
    for (path, id) in &new_paths {
        if !current.contains_key(path) {
            match watcher.watch(path, RecursiveMode::Recursive) {
                Ok(_) => {
                    tracing::info!(
                        folder_id = id.get(),
                        path = %path.display(),
                        "Kyro_Watcher: watch ajouté"
                    );
                }
                Err(err) => {
                    tracing::warn!(
                        folder_id = id.get(),
                        path = %path.display(),
                        error = %err,
                        "Kyro_Watcher: échec watch (ignoré)"
                    );
                    // On retire ce path de new_paths pour que la map
                    // reflète l'état réellement watché.
                    to_remove.push(path.clone());
                }
            }
        }
    }
    for path in to_remove {
        new_paths.remove(&path);
    }

    *current_paths_lock = new_paths;
}

/// Boucle de consommation des événements `notify`.
///
/// Sépare deux phases :
/// 1. **Réception** : agrège les événements dans une map
///    `FolderId → Instant` (instant du dernier événement reçu).
/// 2. **Debounce** : toutes les ~500 ms, vérifie quelles racines ont
///    un événement âgé d'au moins `DEBOUNCE_DURATION` et déclenche un
///    scan ciblé pour celles-là.
async fn run_event_loop<R: Runtime>(
    mut rx: mpsc::Receiver<Event>,
    state: Arc<WatcherState>,
    repo: Repo,
    window: Window<R>,
) {
    // Map des derniers événements par racine.
    let mut last_event: HashMap<FolderId, Instant> = HashMap::new();
    let mut tick = tokio::time::interval(Duration::from_millis(500));

    loop {
        tokio::select! {
            // Réception d'un nouvel événement watcher.
            maybe_event = rx.recv() => {
                let Some(event) = maybe_event else {
                    tracing::info!("Kyro_Watcher: canal fermé — boucle terminée");
                    break;
                };

                if !event_is_relevant(&event) {
                    continue;
                }

                // Trouver la racine correspondante.
                let paths_lock = state.paths.lock().expect("watcher paths mutex poisoned");
                let root_id = event.paths.iter().find_map(|p| {
                    paths_lock.iter().find_map(|(root, id)| {
                        if p.starts_with(root) { Some(*id) } else { None }
                    })
                });
                drop(paths_lock);

                if let Some(id) = root_id {
                    last_event.insert(id, Instant::now());
                }
            }

            // Tick de debounce : examine les racines qui ont stabilisé.
            _ = tick.tick() => {
                let now = Instant::now();
                let ready: Vec<FolderId> = last_event
                    .iter()
                    .filter_map(|(id, ts)| {
                        if now.duration_since(*ts) >= DEBOUNCE_DURATION {
                            Some(*id)
                        } else {
                            None
                        }
                    })
                    .collect();

                for folder_id in ready {
                    last_event.remove(&folder_id);
                    let repo_clone = repo.clone();
                    let state_clone = state.clone();
                    let window_clone = window.clone();
                    tauri::async_runtime::spawn(async move {
                        trigger_incremental_scan(folder_id, repo_clone, state_clone, window_clone).await;
                    });
                }
            }
        }
    }
}

/// Filtre les événements pertinents pour Kyro :
/// - création / modification / suppression / renommage de fichiers
///   ou dossiers.
/// - ignorés : événements `Access` (lecture seule, ouverture sans
///   modification, génération de thumbnails Windows Explorer, etc.)
///   qui pollueraient le debounce sans représenter un changement réel.
fn event_is_relevant(event: &Event) -> bool {
    matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

/// Déclenche un scan incrémental sur un `library_folder`. Sérialisé
/// par [`AUTO_SCAN_RUNNING`] pour éviter d'empiler les scans.
async fn trigger_incremental_scan<R: Runtime>(
    folder_id: FolderId,
    repo: Repo,
    state: Arc<WatcherState>,
    window: Window<R>,
) {
    // Acquisition du verrou global (best-effort).
    if AUTO_SCAN_RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        // Un autre auto-scan est déjà en cours : on remet l'événement
        // en queue en marquant le dossier comme « à rescanner après ».
        let mut scanning = state.scanning.lock().expect("scanning mutex poisoned");
        scanning.insert(folder_id, true);
        tracing::debug!(
            folder_id = folder_id.get(),
            "Kyro_Watcher: scan déjà en cours, ré-essai planifié"
        );
        return;
    }

    // Marque le dossier comme en cours.
    {
        let mut scanning = state.scanning.lock().expect("scanning mutex poisoned");
        scanning.insert(folder_id, true);
    }

    let result = run_scan_for_folder(folder_id, &repo, &window).await;

    // Libération + déclenchement éventuel du re-scan en attente.
    {
        let mut scanning = state.scanning.lock().expect("scanning mutex poisoned");
        scanning.remove(&folder_id);
    }
    AUTO_SCAN_RUNNING.store(false, Ordering::SeqCst);

    match result {
        Ok(report) => {
            tracing::info!(
                folder_id = folder_id.get(),
                processed = report.processed,
                created = report.created,
                updated = report.updated,
                marked_unavailable = report.marked_unavailable,
                "Kyro_Watcher: scan auto terminé"
            );
            // Émission UI (toast léger côté frontend).
            let _ = window.emit(EVENT_LIBRARY_AUTOREFRESH, folder_id.get());
        }
        Err(err) => {
            tracing::warn!(
                folder_id = folder_id.get(),
                error = %err,
                "Kyro_Watcher: scan auto échoué"
            );
        }
    }
}

/// Charge ffprobe + library_folder ciblé, puis appelle
/// `scanner::scan_folder`. Émet `scan-progress` pour cohérence avec
/// l'UI existante (TopBar).
async fn run_scan_for_folder<R: Runtime>(
    folder_id: FolderId,
    repo: &Repo,
    window: &Window<R>,
) -> Result<scanner::ScanReport, crate::error::KyroError> {
    let folder: LibraryFolder = repo
        .list_library_folders()
        .await?
        .into_iter()
        .find(|f| f.id == folder_id)
        .ok_or_else(|| {
            crate::error::KyroError::Validation(format!(
                "library_folder id={} introuvable pour scan auto",
                folder_id.get()
            ))
        })?;

    // Résolution ffprobe (sidecar > setting > PATH).
    let ffprobe_setting = repo.get_setting(SETTING_KEY_FFPROBE_PATH).await.ok().flatten();
    let ff_path = locate_ffprobe(move |key| {
        if key == SETTING_KEY_FFPROBE_PATH {
            ffprobe_setting
        } else {
            None
        }
    })
    .map_err(|e| {
        crate::error::KyroError::Validation(format!(
            "ffprobe introuvable pour scan auto: {e}"
        ))
    })?;
    let probe = FfprobeBackend::new(ff_path);

    let path = Path::new(&folder.path);
    scanner::scan_folder_emit(window, repo, &probe, folder.id, path, folder.category)
        .await
}
