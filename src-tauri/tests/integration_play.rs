//! Tâche 17.2 — test d'intégration **scan + lecture**.
//!
//! Ce test exerce de bout en bout la chaîne suivante, telle que décrite
//! au design § « Cycle de vie de lecture » :
//!
//! ```text
//!   add_library_folder
//!     └─► scanner::scan_folder (avec MockProbeBackend)
//!           └─► Repo::upsert_media_item ⇒ media_item en DB
//!
//!   new_session() ──► PlaybackSession { pipe_name = \\.\pipe\kyro-<uuid>, ... }
//!     └─► ipc_pipe::listen(pipe_name) ──► (mpsc::Receiver<PipeMessage>, JoinHandle)
//!     └─► tokio::spawn(support::mock_mpv::run_mock_mpv(...))
//!           ──► écrit { progress … progress, closed } sur le pipe
//!     └─► player_bridge::run_ipc_session(repo, target, rx)
//!           ──► consomme les messages, persiste la progression,
//!               recalcule le statut via `compute_status`, retourne la
//!               dernière position écrite.
//!
//!   assert media_item.watch_progress_seconds == dernière position envoyée
//!   assert media_item.status                 == compute_status(pos, dur, None)
//! ```
//!
//! # Pourquoi pas un véritable `play_media` Tauri ?
//!
//! `commands::playback::play_media` est annoté `#[tauri::command]` et
//! requiert un `tauri::Window` réel ainsi qu'un binaire mpv.net.exe
//! valide. Pour un test d'intégration sans GUI ni binaire externe, le
//! brief de la tâche 17.2 autorise explicitement « *or the underlying
//! lower-level function* » : on attaque directement
//! [`ipc_pipe::listen`] + [`player_bridge::run_ipc_session`], qui sont
//! les deux briques que `play_media` appelle sur la branche mpv.net
//! (cf. `commands/playback.rs`). Aucun `Child` n'est spawné — le rôle
//! du processus mpv.net + script Lua est joué par
//! [`support::mock_mpv::run_mock_mpv`] (tâche 17.1) qui pousse
//! directement les NDJSON sur le named pipe.
//!
//! # Pourquoi `#![cfg(target_os = "windows")]` ?
//!
//! Le canal IPC repose sur les *Windows named pipes*
//! (`tokio::net::windows::named_pipe`). Ces APIs n'existent pas sur
//! Linux/macOS. Comme Kyro lui-même cible exclusivement Windows
//! (requirement 1.1), on gate ce test sur la plateforme cible plutôt
//! que d'introduire un fallback Unix.
//!
//! # Validates
//!
//! * Requirement 3.1 — le scanner indexe les fichiers vidéo découverts
//!   dans un `library_folder` enregistré.
//! * Requirement 8.2 — `watch_progress_seconds == duration_seconds`
//!   produit `status == 'vu'`.
//! * Requirement 8.3 — `0 < watch_progress_seconds ≤ 90 %` produit
//!   `status == 'en_cours'`.
//! * Requirement 9.4 — la **dernière** position transmise par le
//!   client (message `closed`) est celle persistée.
//! * Requirement 17.2 — la mutation est commitée avant de signaler
//!   succès (write-then-read cohérent).

#![cfg(target_os = "windows")]

mod support;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
};
use tempfile::TempDir;

use kyro::domain::types::{Category, MediaItemId};
use kyro::infra::ipc_pipe;
use kyro::infra::player_bridge::{self, PlaybackSession};
use kyro::infra::probe::{
    AudioTrack, ProbeBackend, ProbeError, SubtitleTrack, TechnicalMetadata,
};
use kyro::infra::repo::{ProgressTarget, Repo};
use kyro::infra::scanner;

use support::mock_mpv::{run_mock_mpv, ClosedMsg, ProgressMsg};

// ---------------------------------------------------------------------------
// MockProbeBackend
//
// Reproduit la stratégie utilisée par les tests internes du scanner
// (cf. `infra/scanner.rs::tests::StubProbe`) : retourne une
// `TechnicalMetadata` figée pour chaque fichier sondé, en
// resynchronisant uniquement `size_bytes` avec la métadonnée FS réelle
// (le scanner s'attend à ce que `size_bytes` corresponde au fichier sur
// disque, requirement 5.2). Le binaire ffprobe n'est jamais invoqué.
// ---------------------------------------------------------------------------

struct MockProbeBackend {
    template: TechnicalMetadata,
}

impl MockProbeBackend {
    /// Construit un backend qui retournera systématiquement
    /// `(duration_seconds, "h264", 1920, 1080, audio aac/eng)` pour
    /// tout fichier sondé. Cette signature est exactement celle
    /// suggérée par le brief de la tâche 17.2.
    fn new(duration_seconds: u32) -> Self {
        Self {
            template: TechnicalMetadata {
                duration_seconds: Some(duration_seconds),
                video_codec: Some("h264".to_string()),
                width: Some(1920),
                height: Some(1080),
                audio_tracks: vec![AudioTrack {
                    codec: Some("aac".to_string()),
                    language: Some("eng".to_string()),
                }],
                subtitle_tracks: Vec::<SubtitleTrack>::new(),
                // Sera écrasé par l'implémentation `probe()` ci-dessous
                // avec la taille réelle lue par `std::fs::metadata`.
                size_bytes: 0,
            },
        }
    }
}

impl ProbeBackend for MockProbeBackend {
    fn probe<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<TechnicalMetadata, ProbeError>> + Send + 'a>> {
        Box::pin(async move {
            let mut meta = self.template.clone();
            meta.size_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            Ok(meta)
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers de bootstrap (DB, FS, requêtes utilitaires)
// ---------------------------------------------------------------------------

/// Ouvre un pool SQLite neuf sur un fichier temporaire et applique
/// toutes les migrations versionnées. Reproduit fidèlement la
/// configuration de `init_sqlite_pool` (`main.rs`) : WAL +
/// `synchronous=NORMAL` + `foreign_keys=ON` + `create_if_missing=true`.
async fn fresh_repo(db_dir: &Path) -> Repo {
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
        .expect("ouverture pool SQLite test");

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("exécution migrations test");

    Repo::new(pool)
}

/// Crée un fichier vidéo synthétique sous `dir/rel`. Le contenu n'est
/// pas vide pour donner une `size_bytes > 0` exploitable par
/// l'idempotence du scanner, mais aucune assertion ne dépend de la
/// valeur précise.
fn touch_video(dir: &Path, rel: &str, content: &[u8]) -> PathBuf {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dirs");
    }
    std::fs::write(&path, content).expect("touch video");
    path
}

/// Réplique du helper interne `infra::repo::canonicalize_or_keep`
/// (qui est `pub(crate)` et non accessible depuis les tests
/// d'intégration). Identique en sémantique : best-effort
/// canonicalisation, retombe sur la forme brute en cas d'échec
/// (typiquement : fichier supprimé entre temps).
fn canonicalize_or_keep(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Récupère l'`id` d'un Media_Item à partir de son `file_path`
/// canonique (forme stockée par le scanner). Échoue si la fiche
/// n'existe pas — ce qui signifierait que le scan n'a pas inséré
/// l'item attendu.
async fn lookup_media_id(repo: &Repo, file_path: &str) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT id FROM media_item WHERE file_path = ?")
        .bind(file_path)
        .fetch_one(repo.pool())
        .await
        .expect("media_item attendu après scan")
}

/// Lit `(watch_progress_seconds, status)` pour un Media_Item donné.
async fn read_progress_status(repo: &Repo, id: i64) -> (i64, String) {
    let row: (i64, String) = sqlx::query_as::<_, (i64, String)>(
        "SELECT watch_progress_seconds, status FROM media_item WHERE id = ?",
    )
    .bind(id)
    .fetch_one(repo.pool())
    .await
    .expect("relecture progression + status");
    row
}

/// Joue un fichier média via la chaîne IPC mpv.net mockée :
/// 1. Génère une `PlaybackSession` (UUID v4 → nom de pipe unique).
/// 2. Ouvre le serveur named pipe via [`ipc_pipe::listen`].
/// 3. Spawn la tâche `support::mock_mpv::run_mock_mpv` qui se connecte
///    au pipe en client, pousse les `progresses` puis le `close`.
/// 4. Lance [`player_bridge::run_ipc_session`] qui consomme le receiver,
///    persiste la progression et retourne la dernière position écrite.
/// 5. Attend la complétion des deux tâches (mock + boucle de lecture)
///    pour s'assurer qu'aucun handle ne reste pendant.
///
/// Retourne le `Some(position)` final écrit par `run_ipc_session`,
/// identique à la position du `closed` envoyé par le mock.
async fn play_with_mock_mpv(
    repo: &Repo,
    target: ProgressTarget,
    duration: f64,
    progresses_pos: &[f64],
    close_pos: f64,
) -> Option<u32> {
    // 1. Session : nom de pipe unique pour ce test (évite la collision
    //    avec d'autres tests Windows tournant en parallèle).
    let session: PlaybackSession = player_bridge::new_session();
    let session_id = session.session_id.to_string();

    // 2. Serveur named pipe + boucle de lecture NDJSON déjà spawnée.
    let (rx, listen_handle) =
        ipc_pipe::listen(&session.pipe_name).expect("création serveur named pipe");

    // 3. Mock client : se connecte au pipe, pousse les messages, drop.
    //    On clone les données en `'static` pour les déplacer dans la
    //    tâche spawnée.
    let pipe_path = session.pipe_name.clone();
    let progresses: Vec<ProgressMsg> = progresses_pos
        .iter()
        .map(|p| ProgressMsg::new(*p, duration, &session_id))
        .collect();
    let close = ClosedMsg::new(close_pos, duration, &session_id);

    let mock_handle = tokio::spawn(async move {
        run_mock_mpv(&pipe_path, &progresses, close)
            .await
            .expect("mock_mpv : la connexion au named pipe doit réussir");
    });

    // 4. Boucle de persistance : consomme rx, écrit en DB.
    let last_written = player_bridge::run_ipc_session(repo, target, rx)
        .await
        .expect("run_ipc_session doit terminer Ok");

    // 5. Cleanup : on attend la fin du mock client (drop = EOF) et la
    //    terminaison propre de la boucle de lecture du pipe.
    mock_handle
        .await
        .expect("mock_mpv : la tâche doit terminer sans panic");
    listen_handle
        .await
        .expect("ipc_pipe::listen : la tâche doit terminer sans panic");

    last_written
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Cas nominal **« en cours »** :
///
/// 1. Scan d'un dossier `Films/` contenant deux mp4 vides via le
///    [`MockProbeBackend`] (durée fixée à 600 s).
/// 2. Vérifie qu'un Media_Item a été inséré pour chaque fichier
///    (requirement 3.1).
/// 3. Joue le premier fichier en envoyant 60 s, 180 s, 300 s puis un
///    `closed` à 300 s — ratio `300 / 600 = 0.5` ⇒ statut attendu
///    `en_cours` (requirement 8.3).
/// 4. Vérifie via une **lecture immédiate** post-`run_ipc_session` que
///    `watch_progress_seconds = 300` et `status = 'en_cours'`. C'est
///    précisément le contrat du requirement 17.2 (write-then-read
///    cohérent : la valeur retournée par le canal IPC reflète bien
///    l'état persisté).
#[tokio::test]
async fn scan_then_play_persists_intermediate_progress_as_en_cours() {
    // ------- 1. Setup -------
    let db_dir = TempDir::new().expect("tempdir DB");
    let lib_dir = TempDir::new().expect("tempdir lib");
    let repo = fresh_repo(db_dir.path()).await;

    let folder_id = repo
        .add_library_folder(lib_dir.path(), Category::Films)
        .await
        .expect("ajout library_folder");

    let film_a = touch_video(lib_dir.path(), "Inception.2010.1080p.mp4", b"film-A");
    let film_b = touch_video(lib_dir.path(), "Dune.2021.1080p.mp4", b"film-B");

    // ------- 2. Scan avec MockProbeBackend (durée = 600 s) -------
    let probe = MockProbeBackend::new(600);
    let report = scanner::scan_folder(
        &repo,
        &probe,
        folder_id,
        lib_dir.path(),
        Category::Films,
    )
    .await
    .expect("scan_folder doit réussir");

    assert_eq!(report.total_files, 2, "deux fichiers candidats attendus");
    assert_eq!(report.created, 2, "deux fiches créées");
    assert!(
        report.failures.is_empty(),
        "aucun échec attendu, observé = {:?}",
        report.failures
    );

    // Forme stockée = canonique, comme `scan_folder` l'inscrit en DB.
    let path_a = canonicalize_or_keep(&film_a)
        .to_string_lossy()
        .into_owned();
    let path_b = canonicalize_or_keep(&film_b)
        .to_string_lossy()
        .into_owned();
    let id_a = lookup_media_id(&repo, &path_a).await;
    let id_b = lookup_media_id(&repo, &path_b).await;

    // Sanity : les champs techniques ont bien été persistés.
    let (duration_db, codec_db): (Option<i64>, Option<String>) = sqlx::query_as::<
        _,
        (Option<i64>, Option<String>),
    >(
        "SELECT duration_seconds, video_codec FROM media_item WHERE id = ?",
    )
    .bind(id_a)
    .fetch_one(repo.pool())
    .await
    .expect("relecture champs techniques");
    assert_eq!(duration_db, Some(600));
    assert_eq!(codec_db.as_deref(), Some("h264"));

    // ------- 3. Lecture mockée : progression 60 → 180 → 300 → close 300 -------
    let last_written = play_with_mock_mpv(
        &repo,
        ProgressTarget::Media(MediaItemId(id_a)),
        600.0,
        &[60.0, 180.0, 300.0],
        300.0,
    )
    .await;

    assert_eq!(
        last_written,
        Some(300),
        "run_ipc_session doit retourner la dernière position du `closed`"
    );

    // ------- 4. Assertions DB -------
    let (progress_db, status_db) = read_progress_status(&repo, id_a).await;
    assert_eq!(
        progress_db, 300,
        "watch_progress_seconds doit refléter la position du `closed` (req. 9.4)"
    );
    assert_eq!(
        status_db, "en_cours",
        "ratio 300/600 = 0.5 doit produire status='en_cours' \
         (req. 8.3, compute_status)"
    );

    // L'autre fiche n'a pas été touchée : status par défaut, progression
    // initiale à 0. C'est le complément du requirement 17.2 (la mutation
    // ne déborde pas au-delà de la cible).
    let (progress_other, status_other) = read_progress_status(&repo, id_b).await;
    assert_eq!(progress_other, 0);
    assert_eq!(status_other, "non_vu");
}

/// Cas nominal **« vu »** :
///
/// 1. Scan d'un dossier `Films/` contenant un mp4 vide via le
///    [`MockProbeBackend`] (durée fixée à 600 s).
/// 2. Joue le fichier en envoyant 200 s, 400 s puis un `closed` à
///    600 s (== durée) — ratio `1.0 > 0.9` ⇒ statut attendu
///    `vu` (requirement 8.2).
/// 3. Vérifie que `watch_progress_seconds = 600` et `status = 'vu'`
///    (write-then-read, requirement 17.2).
#[tokio::test]
async fn play_at_duration_marks_status_as_vu() {
    let db_dir = TempDir::new().expect("tempdir DB");
    let lib_dir = TempDir::new().expect("tempdir lib");
    let repo = fresh_repo(db_dir.path()).await;

    let folder_id = repo
        .add_library_folder(lib_dir.path(), Category::Films)
        .await
        .expect("ajout library_folder");

    let film = touch_video(lib_dir.path(), "Interstellar.2014.1080p.mp4", b"film");

    let probe = MockProbeBackend::new(600);
    let report = scanner::scan_folder(
        &repo,
        &probe,
        folder_id,
        lib_dir.path(),
        Category::Films,
    )
    .await
    .expect("scan_folder doit réussir");
    assert_eq!(report.created, 1);
    assert!(report.failures.is_empty());

    let path = canonicalize_or_keep(&film)
        .to_string_lossy()
        .into_owned();
    let id = lookup_media_id(&repo, &path).await;

    // Position finale = durée : ratio 600/600 = 1.0, strictement
    // supérieur à 0.9 ⇒ `Status::Vu` (cf. `compute_status`).
    let last_written = play_with_mock_mpv(
        &repo,
        ProgressTarget::Media(MediaItemId(id)),
        600.0,
        &[200.0, 400.0],
        600.0,
    )
    .await;
    assert_eq!(last_written, Some(600));

    let (progress_db, status_db) = read_progress_status(&repo, id).await;
    assert_eq!(
        progress_db, 600,
        "watch_progress_seconds doit valoir la durée totale après `closed`"
    );
    assert_eq!(
        status_db, "vu",
        "ratio 600/600 = 1.0 > 0.9 doit produire status='vu' (req. 8.2)"
    );
}
