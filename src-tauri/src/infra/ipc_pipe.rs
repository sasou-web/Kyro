//! Kyro_IPC_Pipe — serveur named pipe Windows + lecture NDJSON (tâche 10.2).
//!
//! Cette couche complète `Kyro_Player_Bridge` (tâche 10.1) en exposant
//! la lecture asynchrone du flux NDJSON émis par `kyro.lua` côté
//! mpv.net. Le bridge crée d'abord la session (`PlaybackSession` →
//! `pipe_name = \\.\pipe\kyro-<session_id>`), puis appelle [`listen`]
//! sur ce nom **avant** de spawner le processus mpv.net : ainsi le pipe
//! est garanti d'exister au moment où le client tente l'ouverture.
//!
//! # Contrat NDJSON
//!
//! Une ligne JSON par message, encodée UTF-8, séparateur `\n`. Schéma
//! [`PipeMessage`] (cf. design § « Kyro_Player_Bridge et Kyro_IPC_Pipe
//! — Protocole IPC ») :
//!
//! * `progress` — émis toutes les 5 s par le timer Lua (req. 9.2)
//! * `paused`   — émis sur l'événement mpv `pause`
//! * `resumed`  — émis sur l'événement mpv `unpause`
//! * `closed`   — émis sur `shutdown`, dernière position (req. 9.3)
//!
//! # Robustesse — lignes invalides
//!
//! Si une ligne ne se désérialise pas (JSON corrompu, type inconnu),
//! elle est journalisée via `tracing::warn!(line, error)` et la lecture
//! continue avec la ligne suivante (req. 9.3, design § « Robustesse »).
//!
//! # Cycle de vie
//!
//! 1. [`listen`] crée le `NamedPipeServer` (instance unique :
//!    `first_pipe_instance(true)`) et retourne un
//!    `mpsc::Receiver<PipeMessage>` ainsi que la `JoinHandle` de la
//!    tâche de lecture.
//! 2. Une tâche tokio est spawnée : elle attend `connect()` du client,
//!    enveloppe le serveur dans un `BufReader`, lit ligne par ligne via
//!    `AsyncBufReadExt::lines()`.
//! 3. Quand le client ferme le pipe (EOF) ou que le receiver est drop,
//!    la tâche s'arrête proprement et le canal se ferme côté émetteur.
//!
//! _Requirements: 9.2, 9.3_

use serde::Deserialize;

#[cfg(target_os = "windows")]
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::windows::named_pipe::{NamedPipeServer, ServerOptions},
    sync::mpsc,
    task::JoinHandle,
};

/// Schéma `PipeMessage` — copie fidèle du design (§ Protocole IPC).
///
/// Le tag `type` discrimine la variante (`rename_all = "lowercase"` →
/// `progress` / `paused` / `resumed` / `closed`). Les champs inconnus
/// (par ex. `duration` sur `paused`/`resumed`/`closed`, que `kyro.lua`
/// émet pour tous les types) sont **ignorés silencieusement** par
/// `serde_json` par défaut, ce qui garantit que le format de ligne du
/// script Lua reste compatible avec ce schéma.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum PipeMessage {
    Progress {
        position: f64,
        duration: f64,
        session: String,
    },
    Paused {
        position: f64,
        session: String,
    },
    Resumed {
        position: f64,
        session: String,
    },
    Closed {
        position: f64,
        session: String,
    },
    /// Émis par `kyro.lua` quand mpv signale `end-file` avec
    /// `reason == "eof"` : le fichier a été lu jusqu'au bout. Permet
    /// au backend de forcer le statut `vu` même si mpv libère
    /// `time-pos` avant le `shutdown`.
    ///
    /// Le champ `path` (optionnel pour rétrocompatibilité avec les
    /// versions du Lua antérieures au 17/05) porte le chemin du
    /// fichier qui vient de se terminer. Quand il est présent, le
    /// backend l'utilise pour résoudre la cible de complétion plutôt
    /// que de se reposer sur la `current_target` de la session — qui
    /// peut déjà avoir été basculée par un `FileChanged` arrivé en
    /// course (autoplay mpv).
    Completed {
        position: f64,
        duration: f64,
        session: String,
        #[serde(default)]
        path: Option<String>,
    },
    /// Émis par `kyro.lua` quand le `path` courant de mpv change en
    /// cours de session — typiquement lors d'un saut playlist-next
    /// (touche `>`) ou d'un chargement automatique du fichier suivant.
    /// Le payload porte le **nouveau chemin absolu** du fichier en
    /// cours de lecture. Le backend doit alors résoudre ce chemin
    /// vers une cible (`media_item` ou `episode_item`) et basculer
    /// la session IPC sur cette nouvelle cible — sinon toutes les
    /// progressions du nouveau fichier seraient écrites sur l'ancien
    /// (bug observé sur les épisodes en autoplay).
    #[serde(rename = "file-changed")]
    FileChanged {
        path: String,
        session: String,
    },
}

/// Capacité du canal `mpsc` — assez pour absorber une rafale de
/// messages (push 5 s + transitions pause/resume) sans bloquer la
/// tâche de lecture en cas de consommateur lent.
#[cfg(target_os = "windows")]
const CHANNEL_CAPACITY: usize = 64;

/// Démarre un serveur named pipe Windows sur `pipe_name` et lance une
/// tâche de lecture NDJSON.
///
/// Retourne :
/// * un `mpsc::Receiver<PipeMessage>` qui yield chaque message valide
///   dans l'ordre d'arrivée. Quand le client ferme le pipe, le sender
///   est drop et le receiver renvoie `None`.
/// * une `JoinHandle<()>` pour permettre à l'appelant d'attendre la
///   terminaison de la tâche de lecture si nécessaire (notamment dans
///   les tests d'intégration). En production, le bridge ignore le
///   handle et se contente de consommer le receiver.
///
/// Erreurs : la création du pipe (`ServerOptions::create`) peut
/// échouer si le nom est mal formé ou si une instance existe déjà
/// (`first_pipe_instance(true)` est exclusif). Dans ce cas, l'erreur
/// `std::io::Error` est propagée à l'appelant **avant** tout spawn.
#[cfg(target_os = "windows")]
pub fn listen(
    pipe_name: &str,
) -> std::io::Result<(mpsc::Receiver<PipeMessage>, JoinHandle<()>)> {
    let server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(pipe_name)?;

    let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
    let pipe_name_owned = pipe_name.to_string();

    let handle = tokio::spawn(async move {
        run_read_loop(server, tx, pipe_name_owned).await;
    });

    Ok((rx, handle))
}

/// Boucle interne de lecture : `connect()` → `lines()` → désérialisation
/// ligne par ligne → envoi sur le canal. Sortie sur EOF, erreur d'IO,
/// ou drop du receiver.
#[cfg(target_os = "windows")]
async fn run_read_loop(
    server: NamedPipeServer,
    tx: mpsc::Sender<PipeMessage>,
    pipe_name: String,
) {
    if let Err(error) = server.connect().await {
        tracing::warn!(
            pipe = %pipe_name,
            error = %error,
            "échec de la connexion client sur le named pipe Kyro"
        );
        return;
    }

    let reader = BufReader::new(server);
    let mut lines = reader.lines();

    loop {
        match lines.next_line().await {
            Ok(Some(line)) => match serde_json::from_str::<PipeMessage>(&line) {
                Ok(msg) => {
                    if tx.send(msg).await.is_err() {
                        // Receiver drop : le bridge a fermé la session,
                        // on quitte proprement la boucle.
                        break;
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        pipe = %pipe_name,
                        line = %line,
                        error = %error,
                        "ligne NDJSON invalide ignorée sur le named pipe Kyro"
                    );
                    // Continue avec la ligne suivante (req. 9.3).
                }
            },
            Ok(None) => {
                // EOF : le client (mpv.net via kyro.lua) a fermé le
                // pipe. Sortie normale de la boucle de lecture.
                break;
            }
            Err(error) => {
                tracing::warn!(
                    pipe = %pipe_name,
                    error = %error,
                    "erreur d'IO en lisant le named pipe Kyro"
                );
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Cibles non-Windows : le named pipe Windows n'existe pas. On expose un
// stub qui retourne une erreur explicite, pour préserver la
// compilation cross-platform sans introduire de branches conditionnelles
// chez les appelants. En production, Kyro ne cible que Windows
// (req. 1.1) ; ce stub n'est utile que pour `cargo check` sur les CI
// non-Windows éventuelles.
// ---------------------------------------------------------------------------

#[cfg(not(target_os = "windows"))]
pub fn listen(
    _pipe_name: &str,
) -> std::io::Result<(
    tokio::sync::mpsc::Receiver<PipeMessage>,
    tokio::task::JoinHandle<()>,
)> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Kyro_IPC_Pipe nécessite Windows (named pipes)",
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::AsyncWriteExt;
    use tokio::net::windows::named_pipe::ClientOptions;
    use uuid::Uuid;

    /// Construit un nom de pipe unique pour isoler chaque test (les
    /// pipes créés avec `first_pipe_instance(true)` sont exclusifs).
    fn unique_pipe_name() -> String {
        format!(r"\\.\pipe\kyro-test-{}", Uuid::new_v4())
    }

    /// Helper : ouvre un client, parfois `ClientOptions::open` doit
    /// être réessayé brièvement le temps que le serveur ait bien
    /// initialisé son `connect()` interne sous Windows.
    async fn open_client(
        pipe_name: &str,
    ) -> tokio::net::windows::named_pipe::NamedPipeClient {
        for _ in 0..50 {
            match ClientOptions::new().open(pipe_name) {
                Ok(c) => return c,
                Err(e)
                    if e.raw_os_error()
                        == Some(windows_errors::ERROR_PIPE_BUSY) =>
                {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(e) => panic!("ClientOptions::open a échoué : {e}"),
            }
        }
        panic!("ClientOptions::open : pipe encore occupé après plusieurs essais");
    }

    /// Codes d'erreur Windows nécessaires localement aux tests, sans
    /// introduire de dépendance crate `windows-sys`.
    mod windows_errors {
        pub const ERROR_PIPE_BUSY: i32 = 231;
    }

    /// Cas nominal : envoi de progress + paused + closed → réception
    /// dans le même ordre, exactement.
    #[tokio::test]
    async fn listen_yields_messages_in_order() {
        let pipe_name = unique_pipe_name();
        let (mut rx, handle) =
            listen(&pipe_name).expect("création du serveur named pipe");

        let mut client = open_client(&pipe_name).await;

        let session = "9c8b1ad2-f9e0-4a7e-a9f4-1234567890ab";
        let progress = format!(
            "{{\"type\":\"progress\",\"position\":12.500,\"duration\":7200.000,\"session\":\"{session}\"}}\n"
        );
        let paused = format!(
            "{{\"type\":\"paused\",\"position\":12.500,\"duration\":7200.000,\"session\":\"{session}\"}}\n"
        );
        let closed = format!(
            "{{\"type\":\"closed\",\"position\":42.250,\"duration\":7200.000,\"session\":\"{session}\"}}\n"
        );

        client.write_all(progress.as_bytes()).await.unwrap();
        client.write_all(paused.as_bytes()).await.unwrap();
        client.write_all(closed.as_bytes()).await.unwrap();
        client.flush().await.unwrap();
        // Fermeture du client → EOF côté serveur.
        drop(client);

        let m1 = rx.recv().await.expect("progress");
        let m2 = rx.recv().await.expect("paused");
        let m3 = rx.recv().await.expect("closed");

        assert_eq!(
            m1,
            PipeMessage::Progress {
                position: 12.500,
                duration: 7200.000,
                session: session.to_string(),
            }
        );
        assert_eq!(
            m2,
            PipeMessage::Paused {
                position: 12.500,
                session: session.to_string(),
            }
        );
        assert_eq!(
            m3,
            PipeMessage::Closed {
                position: 42.250,
                session: session.to_string(),
            }
        );

        // Plus aucun message → canal fermé après EOF.
        assert!(rx.recv().await.is_none());

        handle.await.expect("la tâche de lecture doit terminer proprement");
    }

    /// Robustesse : une ligne malformée est ignorée et la lecture
    /// continue avec les lignes suivantes (req. 9.3).
    #[tokio::test]
    async fn listen_skips_invalid_lines_and_continues() {
        let pipe_name = unique_pipe_name();
        let (mut rx, handle) =
            listen(&pipe_name).expect("création du serveur named pipe");

        let mut client = open_client(&pipe_name).await;

        let session = "deadbeef-cafe-1234-5678-abcdef012345";
        let valid = format!(
            "{{\"type\":\"progress\",\"position\":1.000,\"duration\":2.000,\"session\":\"{session}\"}}\n"
        );
        // 1) JSON syntaxiquement invalide
        let malformed_syntax = "this is not json\n";
        // 2) JSON valide mais type inconnu (n'appartient pas au schéma)
        let malformed_type = format!(
            "{{\"type\":\"unknown\",\"position\":3.0,\"session\":\"{session}\"}}\n"
        );
        let resumed = format!(
            "{{\"type\":\"resumed\",\"position\":4.000,\"session\":\"{session}\"}}\n"
        );

        client.write_all(valid.as_bytes()).await.unwrap();
        client.write_all(malformed_syntax.as_bytes()).await.unwrap();
        client.write_all(malformed_type.as_bytes()).await.unwrap();
        client.write_all(resumed.as_bytes()).await.unwrap();
        client.flush().await.unwrap();
        drop(client);

        let m1 = rx.recv().await.expect("progress");
        let m2 = rx.recv().await.expect("resumed");
        assert_eq!(
            m1,
            PipeMessage::Progress {
                position: 1.0,
                duration: 2.0,
                session: session.to_string(),
            }
        );
        assert_eq!(
            m2,
            PipeMessage::Resumed {
                position: 4.0,
                session: session.to_string(),
            }
        );

        // Aucune autre ligne valide → canal fermé.
        assert!(rx.recv().await.is_none());

        handle.await.expect("la tâche de lecture doit terminer proprement");
    }

    /// Vérifie que le schéma `PipeMessage` accepte la ligne brute émise
    /// par `kyro.lua` (qui contient `duration` sur tous les types) :
    /// les champs surnuméraires sont silencieusement ignorés par serde.
    #[test]
    fn pipe_message_schema_ignores_extra_duration_field() {
        let line =
            "{\"type\":\"paused\",\"position\":1.5,\"duration\":42.0,\"session\":\"sid\"}";
        let parsed: PipeMessage = serde_json::from_str(line).unwrap();
        assert_eq!(
            parsed,
            PipeMessage::Paused {
                position: 1.5,
                session: "sid".to_string(),
            }
        );
    }
}
