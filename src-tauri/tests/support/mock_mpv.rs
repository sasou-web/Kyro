//! Tâche 17.1 — mock client mpv émettant le flux NDJSON attendu par
//! `Kyro_IPC_Pipe`.
//!
//! # Rôle
//!
//! Côté production, le client du `NamedPipeServer` créé par
//! `Kyro_Player_Bridge` est `kyro.lua` qui s'exécute à l'intérieur de
//! `mpv.net.exe`. Pour les tests d'intégration (tâche 17.2 et
//! au-delà) on ne peut évidemment pas spawner un véritable mpv.net :
//! il faut donc une **implémentation de remplacement** qui parle
//! exactement le même protocole NDJSON (cf. design § « Kyro_Player_Bridge
//! et Kyro_IPC_Pipe — Protocole IPC ») mais qui reste pilotable
//! depuis Rust avec des positions / durées / nombres de messages
//! paramétrables.
//!
//! C'est précisément ce que fournit ce module via [`run_mock_mpv`] :
//!
//! 1. ouvre un `NamedPipeClient` Tokio (avec retry sur
//!    `ERROR_PIPE_BUSY` le temps que le serveur appelle `connect()`) ;
//! 2. émet `progresses.len()` messages `progress` séparés par `\n`,
//!    avec une petite pause de 10 ms entre chaque pour mimer le timer
//!    périodique de 5 s du script Lua (compressé en mode test) et
//!    laisser le serveur consommer chaque message individuellement ;
//! 3. émet le message final `closed` (typiquement avec la dernière
//!    position lue par mpv au moment du `shutdown`), flush, puis ferme
//!    le pipe pour déclencher l'EOF côté serveur.
//!
//! L'implémentation est strictement Windows-only (le named pipe Tokio
//! n'existe que sur cette cible) ; un appelant Linux/macOS n'a aucune
//! raison de l'invoquer puisque Kyro lui-même cible exclusivement
//! Windows (requirement 1.1).
//!
//! # Format NDJSON
//!
//! Pour chaque message, l'enveloppe sur le pipe est :
//!
//! ```text
//! {"type":"<kind>","position":<f64>,"duration":<f64>,"session":"<uuid>"}\n
//! ```
//!
//! Le champ `duration` est inclus pour **tous** les types — c'est
//! exactement ce que produit `kyro.lua` (voir
//! `tests/lua_ndjson.rs::lua_format_line`). Le schéma Rust côté
//! serveur (`infra::ipc_pipe::PipeMessage`) ignore silencieusement les
//! champs en trop pour les variantes `paused`/`resumed`/`closed`, donc
//! cette « sur-spécification » est compatible.
//!
//! # API publique
//!
//! Voir [`ProgressMsg`], [`ClosedMsg`] et [`run_mock_mpv`].
//!
//! _Requirements: 9.2, 9.3_

#![cfg(target_os = "windows")]

use std::time::Duration;

use serde::Serialize;
use tokio::io::AsyncWriteExt;
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

/// Code d'erreur Windows `ERROR_PIPE_BUSY` (231) — retourné par
/// `CreateFileW` quand toutes les instances du serveur named pipe
/// sont en cours de `ConnectNamedPipe` mais pas encore prêtes à
/// dispatcher une nouvelle connexion. C'est un cas transitoire :
/// quelques retries avec un petit `sleep` suffisent.
const ERROR_PIPE_BUSY: i32 = 231;

/// Pause inter-messages côté mock — assez longue pour que la boucle de
/// lecture serveur traite chaque ligne avant la suivante (évite les
/// rafales qui rendraient les assertions de test fragiles), mais
/// volontairement très courte (~10 ms) pour ne pas ralentir la suite
/// d'intégration. La période réelle de `kyro.lua` est de 5 s
/// (requirement 9.2) ; aucune assertion de Kyro ne dépend de cette
/// durée précise (la couche `run_ipc_session` traite les messages dans
/// l'ordre d'arrivée, sans contrainte temporelle).
const INTER_MESSAGE_DELAY: Duration = Duration::from_millis(10);

/// Nombre maximal de tentatives d'ouverture du pipe avant d'abandonner.
/// Couvre largement le cas où le serveur n'a pas encore atteint le
/// `connect().await` au moment où le mock est lancé en parallèle.
const OPEN_MAX_RETRIES: usize = 50;

/// Pause entre deux essais d'ouverture quand le pipe est `BUSY`.
const OPEN_RETRY_DELAY: Duration = Duration::from_millis(20);

/// Message `progress` émis périodiquement par `kyro.lua` (timer 5 s)
/// pour pousser la position courante de mpv vers Kyro
/// (requirement 9.2).
///
/// Le champ `kind` est figé à `"progress"` via `#[serde(rename = "type")]`
/// + valeur par défaut dans [`ProgressMsg::new`] : l'appelant n'a pas
/// à le préciser et ne peut pas accidentellement émettre un autre
/// type via cette structure.
#[derive(Debug, Clone, Serialize)]
pub struct ProgressMsg {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub position: f64,
    pub duration: f64,
    pub session: String,
}

impl ProgressMsg {
    /// Construit un message `progress` prêt à être sérialisé en NDJSON.
    ///
    /// * `position` — secondes lues à l'instant du push (champ
    ///   `position` côté `PipeMessage::Progress`).
    /// * `duration` — durée totale du média (champ `duration`).
    /// * `session` — identifiant de session (UUID textuel) ; doit
    ///   correspondre au suffixe `kyro-<uuid>` du nom de pipe pour
    ///   permettre une corrélation en log côté serveur.
    pub fn new(position: f64, duration: f64, session: impl Into<String>) -> Self {
        Self {
            kind: "progress",
            position,
            duration,
            session: session.into(),
        }
    }
}

/// Message `closed` émis par `kyro.lua` sur l'événement mpv
/// `shutdown` — porte la **dernière position** lue avant la fermeture
/// du processus joueur (requirement 9.3).
///
/// `duration` est conservé sur ce type pour rester strictement aligné
/// sur la ligne réelle produite par `kyro.lua` (voir
/// `tests/lua_ndjson.rs`). Le serveur l'ignore pour cette variante,
/// mais l'inclure ici garantit que toute future contrainte de schéma
/// plus stricte (ex. `#[serde(deny_unknown_fields)]`) sera détectée
/// par les tests d'intégration.
#[derive(Debug, Clone, Serialize)]
pub struct ClosedMsg {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub position: f64,
    pub duration: f64,
    pub session: String,
}

impl ClosedMsg {
    /// Construit un message `closed` (position finale + session).
    pub fn new(position: f64, duration: f64, session: impl Into<String>) -> Self {
        Self {
            kind: "closed",
            position,
            duration,
            session: session.into(),
        }
    }
}

/// Message `completed` émis par `kyro.lua` quand mpv signale
/// `end-file` avec `reason == "eof"`. Permet au backend de forcer
/// le statut `vu` indépendamment de la position remontée par `closed`.
#[derive(Debug, Clone, Serialize)]
pub struct CompletedMsg {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub position: f64,
    pub duration: f64,
    pub session: String,
    /// Chemin du fichier qui se termine. Inclus pour permettre au
    /// backend de résoudre la cible de complétion sans ambiguïté
    /// même si un `file-changed` arrive en course.
    pub path: String,
}

impl CompletedMsg {
    pub fn new(
        position: f64,
        duration: f64,
        session: impl Into<String>,
        path: impl Into<String>,
    ) -> Self {
        Self {
            kind: "completed",
            position,
            duration,
            session: session.into(),
            path: path.into(),
        }
    }
}

/// Message `file-changed` émis par `kyro.lua` quand mpv passe au
/// fichier suivant (autoplay, playlist-next, touche `>`).
#[derive(Debug, Clone, Serialize)]
pub struct FileChangedMsg {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub path: String,
    pub session: String,
}

impl FileChangedMsg {
    pub fn new(path: impl Into<String>, session: impl Into<String>) -> Self {
        Self {
            kind: "file-changed",
            path: path.into(),
            session: session.into(),
        }
    }
}

/// Représente un message générique consommable par
/// [`run_mock_mpv_sequence`] qui prend une séquence hétérogène
/// d'événements mpv pour simuler des scénarios complexes (autoplay
/// E1 → E2 → E3 avec `completed` + `file-changed` + `closed` à des
/// instants différents).
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum MockEvent {
    Progress(ProgressMsg),
    Completed(CompletedMsg),
    FileChanged(FileChangedMsg),
    Closed(ClosedMsg),
}

/// Connecte un client named pipe à `pipe_path`, émet la séquence
/// `progresses[0..n]` puis le message `close`, et ferme le pipe.
///
/// `pipe_path` doit être le chemin Windows complet, de la forme
/// `\\.\pipe\kyro-<session_id>` — c'est exactement la chaîne retournée
/// par [`crate::infra::player_bridge::PlaybackSession::pipe_name`] et
/// passée à [`crate::infra::ipc_pipe::listen`] côté serveur. Aucun
/// préfixe n'est ajouté ici afin que l'appelant garde le contrôle
/// total sur la cible.
///
/// # Comportement
///
/// 1. Ouvre le client avec retry sur `ERROR_PIPE_BUSY` (jusqu'à
///    [`OPEN_MAX_RETRIES`] essais espacés de [`OPEN_RETRY_DELAY`]).
/// 2. Pour chaque `ProgressMsg` : sérialise via `serde_json` (ne peut
///    pas échouer sur ces structures purement numériques + chaîne
///    UTF-8), écrit la ligne suivie de `\n`, attend
///    [`INTER_MESSAGE_DELAY`].
/// 3. Sérialise et écrit le `ClosedMsg`, flush, drop le client → le
///    serveur reçoit l'EOF.
///
/// # Erreurs
///
/// * Échec persistant d'ouverture du pipe (serveur jamais prêt
///   après [`OPEN_MAX_RETRIES`] tentatives) → `std::io::Error` de
///   type `TimedOut` ou erreur OS originale (autre que `BUSY`).
/// * Erreur d'IO en cours d'écriture (pipe fermé prématurément côté
///   serveur, par exemple) → propagée telle quelle.
///
/// # Note
///
/// La fonction prend les progrès par référence (`&[ProgressMsg]`) pour
/// permettre à l'appelant de les construire dans une `Vec<_>` locale
/// sans transférer la propriété, et le `close` par valeur car il est
/// consommé une seule fois. Cela correspond à la suggestion donnée
/// dans le brief de la tâche 17.1.
pub async fn run_mock_mpv(
    pipe_path: &str,
    progresses: &[ProgressMsg],
    close: ClosedMsg,
) -> std::io::Result<()> {
    let mut client = open_with_retry(pipe_path).await?;

    for p in progresses {
        let line = serde_json::to_string(p)
            .expect("ProgressMsg ne peut pas échouer à se sérialiser en JSON");
        client.write_all(line.as_bytes()).await?;
        client.write_all(b"\n").await?;
        tokio::time::sleep(INTER_MESSAGE_DELAY).await;
    }

    let line = serde_json::to_string(&close)
        .expect("ClosedMsg ne peut pas échouer à se sérialiser en JSON");
    client.write_all(line.as_bytes()).await?;
    client.write_all(b"\n").await?;
    client.flush().await?;

    // Drop explicite du client → fermeture du handle → EOF côté serveur,
    // ce qui termine la boucle `lines.next_line()` du `run_read_loop`.
    drop(client);
    Ok(())
}

/// Variante de [`run_mock_mpv`] qui accepte une séquence hétérogène
/// d'événements (`progress`, `completed`, `file-changed`, `closed`).
///
/// Utile pour reproduire des scénarios complexes :
/// - autoplay E1 → E2 (E1: progress…progress + completed + file-changed + E2: progress…closed)
/// - course completed/file-changed dans un ordre inversé
/// - skip manuel sans completed (file-changed seul)
///
/// Le client ne ferme **pas** automatiquement après la séquence : si
/// la séquence ne contient pas de `Closed` final, l'EOF est émis par
/// le drop du client à la sortie de la fonction. La boucle de lecture
/// serveur sortira sur EOF si elle n'a pas vu de `Closed`.
pub async fn run_mock_mpv_sequence(
    pipe_path: &str,
    events: &[MockEvent],
) -> std::io::Result<()> {
    let mut client = open_with_retry(pipe_path).await?;
    for ev in events {
        let line = serde_json::to_string(ev)
            .expect("MockEvent ne peut pas échouer à se sérialiser en JSON");
        client.write_all(line.as_bytes()).await?;
        client.write_all(b"\n").await?;
        tokio::time::sleep(INTER_MESSAGE_DELAY).await;
    }
    client.flush().await?;
    drop(client);
    Ok(())
}

/// Boucle d'ouverture avec retry borné sur `ERROR_PIPE_BUSY`.
///
/// Sous Windows, lorsque le serveur named pipe vient d'être créé via
/// `ServerOptions::create` mais que sa tâche d'arrière-plan n'a pas
/// encore appelé `connect().await`, un client `ClientOptions::new().open()`
/// peut renvoyer `ERROR_PIPE_BUSY`. C'est un cas transitoire : on
/// retente après [`OPEN_RETRY_DELAY`].
///
/// Les autres erreurs (pipe inexistant, accès refusé, nom invalide)
/// sont remontées immédiatement à l'appelant : aucun retry n'aiderait.
async fn open_with_retry(pipe_path: &str) -> std::io::Result<NamedPipeClient> {
    for _ in 0..OPEN_MAX_RETRIES {
        match ClientOptions::new().open(pipe_path) {
            Ok(c) => return Ok(c),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                tokio::time::sleep(OPEN_RETRY_DELAY).await;
            }
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        format!(
            "named pipe {pipe_path} encore occupé (BUSY) après {OPEN_MAX_RETRIES} tentatives"
        ),
    ))
}
