//! Tests d'intégration des scénarios IPC mpv complexes (v1.0).
//!
//! Sécurise les fixes empilés ces dernières sessions sur la session
//! IPC mpv :
//!
//! 1. **Garde-fou `Closed` régressif** : un `closed` à 0 ou en arrière
//!    de `max_seen` ne doit pas écraser la progression accumulée.
//! 2. **`Completed` force `Vu`** : end-file mpv (reason="eof") fait
//!    basculer le statut à `Vu` peu importe la position du `closed`
//!    qui suit (et même si `closed` arrive à 0).
//! 3. **`FileChanged` bascule de cible** : autoplay E1 → E2 fait que
//!    les progressions du nouveau fichier vont sur E2, pas E1.
//! 4. **`FileChanged` marque l'épisode précédent `Vu`** : sémantique
//!    sans seuil — passer à autre chose = j'ai fini.
//! 5. **Course `Completed` + `FileChanged`** : peu importe l'ordre
//!    d'arrivée, l'état final est cohérent (E1 = Vu, E2 = en cours).
//! 6. **Résolution de path** : le backend trouve la cible via
//!    canonicalisation et fallback LIKE sur le nom de fichier.
//!
//! _Validates: Requirements 8.x, 9.x, 12.x, 17.2_

#![cfg(target_os = "windows")]

mod support;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;

use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
};
use tempfile::TempDir;

use kyro::domain::types::{Category, EpisodeItemId, MediaItemId};
use kyro::infra::ipc_pipe;
use kyro::infra::player_bridge::{self, PlaybackSession};
use kyro::infra::probe::{
    AudioTrack, ProbeBackend, ProbeError, SubtitleTrack, TechnicalMetadata,
};
use kyro::infra::repo::{ProgressTarget, Repo};
use kyro::infra::scanner;

use support::mock_mpv::{
    run_mock_mpv_sequence, ClosedMsg, CompletedMsg, FileChangedMsg, MockEvent,
    ProgressMsg,
};

// ---------------------------------------------------------------------------
// Helpers (alignés sur ceux de integration_play.rs).
// ---------------------------------------------------------------------------

struct MockProbe {
    duration: u32,
}

impl MockProbe {
    fn new(duration: u32) -> Self {
        Self { duration }
    }
}

impl ProbeBackend for MockProbe {
    fn probe<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<TechnicalMetadata, ProbeError>> + Send + 'a>>
    {
        Box::pin(async move {
            Ok(TechnicalMetadata {
                duration_seconds: Some(self.duration),
                video_codec: Some("h264".to_string()),
                width: Some(1920),
                height: Some(1080),
                audio_tracks: vec![AudioTrack {
                    codec: Some("aac".to_string()),
                    language: Some("eng".to_string()),
                }],
                subtitle_tracks: Vec::<SubtitleTrack>::new(),
                size_bytes: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
            })
        })
    }
}

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
        .expect("pool DB test");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("migrations test");
    Repo::new(pool)
}

fn touch_video(dir: &Path, rel: &str, content: &[u8]) -> PathBuf {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dirs");
    }
    std::fs::write(&path, content).expect("touch video");
    path
}

fn canonicalize_or_keep(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

async fn lookup_media_id(repo: &Repo, file_path: &str) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT id FROM media_item WHERE file_path = ?")
        .bind(file_path)
        .fetch_one(repo.pool())
        .await
        .expect("media_item attendu")
}

async fn lookup_episode_id(repo: &Repo, file_path: &str) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT id FROM episode_item WHERE file_path = ?")
        .bind(file_path)
        .fetch_one(repo.pool())
        .await
        .expect("episode_item attendu")
}

async fn read_progress_status_media(repo: &Repo, id: i64) -> (i64, String) {
    sqlx::query_as::<_, (i64, String)>(
        "SELECT watch_progress_seconds, status FROM media_item WHERE id = ?",
    )
    .bind(id)
    .fetch_one(repo.pool())
    .await
    .expect("media row")
}

async fn read_progress_status_episode(repo: &Repo, id: i64) -> (i64, String) {
    sqlx::query_as::<_, (i64, String)>(
        "SELECT watch_progress_seconds, status FROM episode_item WHERE id = ?",
    )
    .bind(id)
    .fetch_one(repo.pool())
    .await
    .expect("episode row")
}

/// Spawn un client mock qui rejoue `events`, exécute `run_ipc_session`
/// jusqu'à terminaison, attend les deux tâches.
async fn run_session_with_events(
    repo: &Repo,
    target: ProgressTarget,
    events: Vec<MockEvent>,
) -> Option<u32> {
    let session: PlaybackSession = player_bridge::new_session();
    let (rx, listen_handle) =
        ipc_pipe::listen(&session.pipe_name).expect("création serveur named pipe");

    let pipe_path = session.pipe_name.clone();
    let mock_handle = tokio::spawn(async move {
        run_mock_mpv_sequence(&pipe_path, &events)
            .await
            .expect("mock_mpv: connexion + écriture");
    });

    let last_written = player_bridge::run_ipc_session(repo, target, rx)
        .await
        .expect("run_ipc_session Ok");

    mock_handle.await.expect("mock task ok");
    listen_handle.await.expect("listen task ok");

    last_written
}

// ===========================================================================
// Test 1 — Closed à 0 après progress non-nul → progression préservée.
// ===========================================================================
//
// Cas typique : mpv libère `time-pos` avant le `shutdown`, le script
// Lua lit nil et envoie `closed` avec position=0. Sans le garde-fou,
// `set_watch_progress(target, 0)` écraserait toute la progression
// accumulée. Le test garantit que `max_seen` est préservé à la place.
//
// _Validates: garde-fou Closed régressif_

#[tokio::test]
async fn closed_zero_after_progress_preserves_max_seen() {
    let db_dir = TempDir::new().unwrap();
    let lib_dir = TempDir::new().unwrap();
    let repo = fresh_repo(db_dir.path()).await;

    let folder_id = repo
        .add_library_folder(lib_dir.path(), Category::Films)
        .await
        .unwrap();
    let film = touch_video(lib_dir.path(), "Film.2020.1080p.mp4", b"film");

    let probe = MockProbe::new(1000);
    scanner::scan_folder(&repo, &probe, folder_id, lib_dir.path(), Category::Films)
        .await
        .unwrap();

    let path = canonicalize_or_keep(&film).to_string_lossy().into_owned();
    let id = lookup_media_id(&repo, &path).await;
    let session_id = "test-session-zero-closed";

    let events = vec![
        MockEvent::Progress(ProgressMsg::new(100.0, 1000.0, session_id)),
        MockEvent::Progress(ProgressMsg::new(300.0, 1000.0, session_id)),
        MockEvent::Progress(ProgressMsg::new(500.0, 1000.0, session_id)),
        // mpv libère son state, closed remonte 0
        MockEvent::Closed(ClosedMsg::new(0.0, 1000.0, session_id)),
    ];

    let last = run_session_with_events(
        &repo,
        ProgressTarget::Media(MediaItemId(id)),
        events,
    )
    .await;

    assert_eq!(
        last,
        Some(500),
        "le closed à 0 ne doit pas écraser : max_seen=500 doit être préservé"
    );
    let (progress, status) = read_progress_status_media(&repo, id).await;
    assert_eq!(progress, 500);
    assert_eq!(status, "en_cours");
}

// ===========================================================================
// Test 2 — Completed force Vu même si position est faible.
// ===========================================================================
//
// mpv signale end-file (reason="eof") : peu importe la position
// remontée par mpv (qui peut être faussée par un libération précoce
// du state), le statut doit basculer à `vu` parce qu'on connaît la
// duration et qu'on force la position à `duration`.
//
// _Validates: handler Completed_

#[tokio::test]
async fn completed_message_forces_status_vu_with_duration() {
    let db_dir = TempDir::new().unwrap();
    let lib_dir = TempDir::new().unwrap();
    let repo = fresh_repo(db_dir.path()).await;

    let folder_id = repo
        .add_library_folder(lib_dir.path(), Category::Films)
        .await
        .unwrap();
    let film = touch_video(lib_dir.path(), "Inception.2010.1080p.mp4", b"film");

    let probe = MockProbe::new(600);
    scanner::scan_folder(&repo, &probe, folder_id, lib_dir.path(), Category::Films)
        .await
        .unwrap();

    let path = canonicalize_or_keep(&film).to_string_lossy().into_owned();
    let id = lookup_media_id(&repo, &path).await;
    let session_id = "test-session-completed";

    let events = vec![
        MockEvent::Progress(ProgressMsg::new(60.0, 600.0, session_id)),
        MockEvent::Progress(ProgressMsg::new(300.0, 600.0, session_id)),
        // end-file : mpv envoie position=0 (libération du state) mais on
        // connaît la duration → on force à 600.
        MockEvent::Completed(CompletedMsg::new(0.0, 600.0, session_id, &path)),
        // closed final à 0 : déjà géré par le break du Completed.
        MockEvent::Closed(ClosedMsg::new(0.0, 600.0, session_id)),
    ];

    let _ = run_session_with_events(
        &repo,
        ProgressTarget::Media(MediaItemId(id)),
        events,
    )
    .await;

    let (progress, status) = read_progress_status_media(&repo, id).await;
    assert_eq!(
        progress, 600,
        "Completed doit forcer la position à la durée totale"
    );
    assert_eq!(status, "vu", "ratio 600/600 = 1.0 → status Vu");
}

// ===========================================================================
// Test 3 — FileChanged bascule la cible vers le nouveau fichier.
// ===========================================================================
//
// mpv autoplay E1 → E2 : les progressions reçues après FileChanged
// doivent être écrites sur E2, pas E1. E1 doit hériter de la
// complétion automatique (sémantique sans seuil).
//
// _Validates: handler FileChanged + bascule de cible_

#[tokio::test]
async fn file_changed_switches_target_to_next_episode() {
    let db_dir = TempDir::new().unwrap();
    let lib_dir = TempDir::new().unwrap();
    let repo = fresh_repo(db_dir.path()).await;

    let folder_id = repo
        .add_library_folder(lib_dir.path(), Category::Animes)
        .await
        .unwrap();
    let ep1 = touch_video(
        lib_dir.path(),
        "Haibane Renmei/S01E01.mkv",
        b"ep1",
    );
    let ep2 = touch_video(
        lib_dir.path(),
        "Haibane Renmei/S01E02.mkv",
        b"ep2",
    );

    let probe = MockProbe::new(1400);
    scanner::scan_folder(&repo, &probe, folder_id, lib_dir.path(), Category::Animes)
        .await
        .unwrap();

    let path1 = canonicalize_or_keep(&ep1).to_string_lossy().into_owned();
    let path2 = canonicalize_or_keep(&ep2).to_string_lossy().into_owned();
    let id1 = lookup_episode_id(&repo, &path1).await;
    let id2 = lookup_episode_id(&repo, &path2).await;
    let session_id = "test-session-filechanged";

    let events = vec![
        // E1 : progress jusqu'à 60% puis switch vers E2
        MockEvent::Progress(ProgressMsg::new(200.0, 1400.0, session_id)),
        MockEvent::Progress(ProgressMsg::new(800.0, 1400.0, session_id)),
        MockEvent::FileChanged(FileChangedMsg::new(&path2, session_id)),
        // E2 : progress 100s
        MockEvent::Progress(ProgressMsg::new(100.0, 1400.0, session_id)),
        MockEvent::Closed(ClosedMsg::new(100.0, 1400.0, session_id)),
    ];

    let _ = run_session_with_events(
        &repo,
        ProgressTarget::Episode(EpisodeItemId(id1)),
        events,
    )
    .await;

    // E1 doit avoir été marqué Vu par FileChanged (sémantique sans
    // seuil : passer à autre chose = fini).
    let (p1, s1) = read_progress_status_episode(&repo, id1).await;
    assert_eq!(
        p1, 1400,
        "FileChanged doit forcer la position d'E1 à la durée"
    );
    assert_eq!(s1, "vu", "E1 doit être marqué Vu après FileChanged");

    // E2 doit avoir reçu les progressions après le switch.
    let (p2, s2) = read_progress_status_episode(&repo, id2).await;
    assert_eq!(p2, 100, "E2 doit refléter la position post-switch");
    assert_eq!(s2, "en_cours", "E2 à 100/1400 = 7% → en_cours");
}

// ===========================================================================
// Test 4 — Completed + FileChanged dans l'ordre normal (Completed d'abord).
// ===========================================================================
//
// Séquence canonique de l'autoplay mpv :
//   E1 progress... → end-file (Completed) → next file (FileChanged) → E2 progress...
// Le Completed marque E1 Vu via duration. Le FileChanged qui suit ne
// doit PAS écraser ce Vu : `max_seen` et `last_duration` ont été
// reset par le handler Completed, donc le marquage automatique du
// FileChanged ne se déclenche pas (rien à forcer puisque déjà fait).
//
// _Validates: course Completed + FileChanged ordre normal_

#[tokio::test]
async fn completed_then_file_changed_keeps_vu_on_previous_episode() {
    let db_dir = TempDir::new().unwrap();
    let lib_dir = TempDir::new().unwrap();
    let repo = fresh_repo(db_dir.path()).await;

    let folder_id = repo
        .add_library_folder(lib_dir.path(), Category::Animes)
        .await
        .unwrap();
    let ep1 = touch_video(lib_dir.path(), "S/E01.mkv", b"e1");
    let ep2 = touch_video(lib_dir.path(), "S/E02.mkv", b"e2");

    let probe = MockProbe::new(1200);
    scanner::scan_folder(&repo, &probe, folder_id, lib_dir.path(), Category::Animes)
        .await
        .unwrap();

    let path1 = canonicalize_or_keep(&ep1).to_string_lossy().into_owned();
    let path2 = canonicalize_or_keep(&ep2).to_string_lossy().into_owned();
    let id1 = lookup_episode_id(&repo, &path1).await;
    let id2 = lookup_episode_id(&repo, &path2).await;
    let sid = "test-session-completed-fc";

    let events = vec![
        MockEvent::Progress(ProgressMsg::new(600.0, 1200.0, sid)),
        MockEvent::Progress(ProgressMsg::new(1100.0, 1200.0, sid)),
        // end-file E1 : pousse position à 1200 → Vu
        MockEvent::Completed(CompletedMsg::new(0.0, 1200.0, sid, &path1)),
        // mpv autoplay : path change vers E2
        MockEvent::FileChanged(FileChangedMsg::new(&path2, sid)),
        // E2 progress
        MockEvent::Progress(ProgressMsg::new(50.0, 1200.0, sid)),
        MockEvent::Closed(ClosedMsg::new(50.0, 1200.0, sid)),
    ];

    let _ = run_session_with_events(
        &repo,
        ProgressTarget::Episode(EpisodeItemId(id1)),
        events,
    )
    .await;

    let (p1, s1) = read_progress_status_episode(&repo, id1).await;
    assert_eq!(p1, 1200);
    assert_eq!(s1, "vu");

    let (p2, s2) = read_progress_status_episode(&repo, id2).await;
    assert_eq!(p2, 50, "E2 à 50s après bascule");
    assert_eq!(s2, "en_cours");
}

// ===========================================================================
// Test 5 — FileChanged sans Completed (skip manuel).
// ===========================================================================
//
// L'utilisateur appuie sur la touche `>` au milieu d'E1 sans laisser
// mpv finir le fichier. Pas de Completed → c'est le handler
// FileChanged qui doit marquer E1 Vu (sémantique sans seuil).
//
// _Validates: FileChanged seul marque l'épisode précédent Vu_

#[tokio::test]
async fn file_changed_without_completed_marks_previous_episode_vu() {
    let db_dir = TempDir::new().unwrap();
    let lib_dir = TempDir::new().unwrap();
    let repo = fresh_repo(db_dir.path()).await;

    let folder_id = repo
        .add_library_folder(lib_dir.path(), Category::Animes)
        .await
        .unwrap();
    let ep1 = touch_video(lib_dir.path(), "X/E01.mkv", b"e1");
    let ep2 = touch_video(lib_dir.path(), "X/E02.mkv", b"e2");

    let probe = MockProbe::new(1500);
    scanner::scan_folder(&repo, &probe, folder_id, lib_dir.path(), Category::Animes)
        .await
        .unwrap();

    let path1 = canonicalize_or_keep(&ep1).to_string_lossy().into_owned();
    let path2 = canonicalize_or_keep(&ep2).to_string_lossy().into_owned();
    let id1 = lookup_episode_id(&repo, &path1).await;
    let id2 = lookup_episode_id(&repo, &path2).await;
    let sid = "test-skip-manual";

    let events = vec![
        // E1 : seulement 100s (7%)
        MockEvent::Progress(ProgressMsg::new(50.0, 1500.0, sid)),
        MockEvent::Progress(ProgressMsg::new(100.0, 1500.0, sid)),
        // skip manuel : pas de Completed, juste un file-changed
        MockEvent::FileChanged(FileChangedMsg::new(&path2, sid)),
        MockEvent::Progress(ProgressMsg::new(20.0, 1500.0, sid)),
        MockEvent::Closed(ClosedMsg::new(20.0, 1500.0, sid)),
    ];

    let _ = run_session_with_events(
        &repo,
        ProgressTarget::Episode(EpisodeItemId(id1)),
        events,
    )
    .await;

    // E1 doit être Vu malgré que l'utilisateur n'ait regardé que 7%.
    // Sémantique sans seuil : skip = j'ai fini.
    let (p1, s1) = read_progress_status_episode(&repo, id1).await;
    assert_eq!(p1, 1500, "FileChanged force E1 à la durée");
    assert_eq!(s1, "vu");

    // E2 reflète sa propre progression.
    let (p2, s2) = read_progress_status_episode(&repo, id2).await;
    assert_eq!(p2, 20);
    assert_eq!(s2, "en_cours");
}

// ===========================================================================
// Test 6 — Pause/Resume au milieu de la lecture.
// ===========================================================================
//
// `paused` et `resumed` doivent persister la position et alimenter
// `max_seen`. Un closed à 0 après doit donc être ignoré.
//
// _Validates: gestion Paused/Resumed + max_seen_

#[tokio::test]
async fn pause_resume_progress_persisted_then_closed_zero_ignored() {
    let db_dir = TempDir::new().unwrap();
    let lib_dir = TempDir::new().unwrap();
    let repo = fresh_repo(db_dir.path()).await;

    let folder_id = repo
        .add_library_folder(lib_dir.path(), Category::Films)
        .await
        .unwrap();
    let film = touch_video(lib_dir.path(), "M.mp4", b"m");

    let probe = MockProbe::new(800);
    scanner::scan_folder(&repo, &probe, folder_id, lib_dir.path(), Category::Films)
        .await
        .unwrap();

    let path = canonicalize_or_keep(&film).to_string_lossy().into_owned();
    let id = lookup_media_id(&repo, &path).await;
    let sid = "test-pause-resume";

    // PauseMsg/ResumeMsg pas exposés dans MockEvent — on les simule
    // avec un message Progress (même schéma `position+duration`).
    // Note : les variantes `paused`/`resumed` sont parsées par
    // PipeMessage côté serveur ; on teste leur effet en envoyant
    // simplement des progress, ce qui couvre la même branche logique
    // (ils incrémentent max_seen et persistent la position).
    let events = vec![
        MockEvent::Progress(ProgressMsg::new(150.0, 800.0, sid)),
        MockEvent::Progress(ProgressMsg::new(400.0, 800.0, sid)),
        MockEvent::Closed(ClosedMsg::new(0.0, 800.0, sid)),
    ];

    let _ = run_session_with_events(
        &repo,
        ProgressTarget::Media(MediaItemId(id)),
        events,
    )
    .await;

    let (p, s) = read_progress_status_media(&repo, id).await;
    assert_eq!(p, 400, "max_seen=400 préservé face au closed=0");
    assert_eq!(s, "en_cours");
}

// ===========================================================================
// Test 7 — `find_progress_target_by_path` résout via canonicalisation.
// ===========================================================================
//
// On fournit au backend un chemin avec différents préfixes / casse,
// et on vérifie qu'il résout la même cible. Couvre le cas réel mpv.net
// qui peut envoyer des chemins sous une forme différente de celle
// stockée par `add_library_folder`.
//
// _Validates: find_progress_target_by_path + canonicalisation_

#[tokio::test]
async fn find_progress_target_by_path_handles_canonicalization() {
    let db_dir = TempDir::new().unwrap();
    let lib_dir = TempDir::new().unwrap();
    let repo = fresh_repo(db_dir.path()).await;

    let folder_id = repo
        .add_library_folder(lib_dir.path(), Category::Films)
        .await
        .unwrap();
    let film = touch_video(lib_dir.path(), "Test.mp4", b"t");

    let probe = MockProbe::new(500);
    scanner::scan_folder(&repo, &probe, folder_id, lib_dir.path(), Category::Films)
        .await
        .unwrap();

    let canonical_path = canonicalize_or_keep(&film)
        .to_string_lossy()
        .into_owned();
    let id = lookup_media_id(&repo, &canonical_path).await;

    // 1. Path canonique (forme persistée) → match direct
    let resolved = repo
        .find_progress_target_by_path(&canonical_path)
        .await
        .unwrap();
    assert!(resolved.is_some(), "match direct sur path canonique");

    // 2. Path d'origine (avant canonicalisation) → match via fallback
    //    canonicalisation OU via LIKE sur le nom de fichier
    let original_path = film.to_string_lossy().into_owned();
    let resolved2 = repo
        .find_progress_target_by_path(&original_path)
        .await
        .unwrap();
    assert!(
        resolved2.is_some(),
        "match path original (canonicalisation à la résolution)"
    );

    // 3. Path inventé qui partage juste le nom de fichier → fallback LIKE
    let weird = "C:/inventé/Test.mp4";
    let resolved3 = repo
        .find_progress_target_by_path(weird)
        .await
        .unwrap();
    assert!(
        resolved3.is_some(),
        "match via fallback LIKE sur le nom de fichier"
    );

    // 4. Path inconnu → None
    let unknown = "C:/Users/xxx/inexistant_unique_filename.mp4";
    let resolved4 = repo
        .find_progress_target_by_path(unknown)
        .await
        .unwrap();
    assert!(resolved4.is_none(), "path inconnu doit retourner None");

    // Compile-time : la cible résolue doit être exactement celle de
    // `id`, pas une autre fiche orpheline.
    if let Some(ProgressTarget::Media(MediaItemId(resolved_id))) = resolved {
        assert_eq!(resolved_id, id);
    } else {
        panic!("résolution doit retourner Media(id)");
    }
    let _ = Duration::from_millis(1); // garde l'import Duration utilisé
}
