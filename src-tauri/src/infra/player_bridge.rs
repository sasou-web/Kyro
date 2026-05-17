//! Kyro_Player_Bridge — fondation de la couche IPC mpv.net (tâche 10.1).
//!
//! Cette tâche couvre uniquement la **génération de l'état de session** :
//! à chaque lancement de `play_media`, un identifiant de session unique
//! (UUID v4) est tiré, puis utilisé pour fabriquer :
//!
//! * le **nom de pipe Windows complet** `\\.\pipe\kyro-<session_id>`
//!   utilisé côté serveur Rust par `tokio::net::windows::named_pipe::
//!   NamedPipeServer` (tâche 10.2) ;
//! * la **valeur `--script-opts`** transmise à mpv.net (tâche 10.3) sous
//!   la forme `kyro-pipe=kyro-<session_id>`. Le script Lua (tâche 10.6)
//!   lit cette option, reconstruit le chemin `\\.\pipe\` + nom logique,
//!   et ouvre le pipe en écriture.
//!
//! La tâche 10.3 ajoute en plus de la génération de session :
//!
//! * [`is_mpv_net`] — détection insensible à la casse de l'exécutable
//!   `mpv.net.exe` (ou `mpvnet.exe`) à partir du chemin configuré dans
//!   `External_Player` ;
//! * [`build_mpv_command`] — construction d'un [`tokio::process::Command`]
//!   pré-configuré avec le fichier média, l'argument `--start=<seconds>`
//!   pour la reprise exacte (requirement 9.1) et `--script-opts=<value>`
//!   issu du [`PlaybackSession`] courant, le tout sans le spawn (utile
//!   pour les tests qui inspectent les arguments) ;
//! * [`launch_mpv_net`] — wrapper asynchrone qui appelle `build_mpv_command`
//!   puis `spawn()` et retourne l'enfant `tokio::process::Child`.
//!
//! ## Tâche 10.4 — branche External_Player ≠ mpv.net
//!
//! Lorsque le chemin configuré dans `External_Player` ne se résout pas
//! en `mpv.net.exe` (ou `mpvnet.exe`), Kyro doit éviter d'appliquer le
//! protocole IPC mpv (qui ne serait jamais consommé par un autre lecteur)
//! et signaler explicitement à l'UI que la reprise exacte ne peut pas
//! être garantie pour cette session. Cette tâche introduit :
//!
//! * [`PlayerKind`] et [`branch`] — décision **pure** (sans IO, sans
//!   spawn) qui choisit entre `MpvNet` et `External` à partir du seul
//!   chemin du lecteur. Permet de tester unitairement la logique de
//!   routage sans dépendre de Tauri ou d'un véritable processus enfant.
//! * [`build_external_command`] — construction d'un
//!   [`tokio::process::Command`] minimaliste pour un lecteur tiers :
//!   programme = `player_path`, **un seul argument** = `file_path`. Ni
//!   `--start`, ni `--script-opts` ne sont passés (ces options sont
//!   propres à mpv et risqueraient d'être interprétées comme des noms
//!   de fichiers par d'autres lecteurs).
//! * [`launch_external_player`] — wrapper asynchrone qui spawn la
//!   commande retournée par `build_external_command`. **Ne crée aucun
//!   `NamedPipeServer`** (property 23, requirement 10.5).
//! * [`LaunchOutcome`] et [`launch_player`] — façade publique unifiée
//!   pour la couche `commands::play_media` (tâche 12.3) : un appel
//!   unique reçoit `(player_path, file_path, start_seconds)` et
//!   retourne soit `LaunchOutcome::MpvNet { session, child }` (IPC
//!   câblé, position de reprise propagée), soit
//!   `LaunchOutcome::External { child }` (pas d'IPC, badge UI à
//!   afficher). L'appelant Tauri-aware émet ensuite, sur la branche
//!   `External`, l'événement [`EVENT_EXTERNAL_PLAYER_NON_MPV`] vers le
//!   frontend pour activer l'affichage du badge **« reprise exacte non
//!   garantie »** sur la fiche du Media_Item courant. Cette séparation
//!   stricte « logique pure → enum → effet Tauri » garde le module
//!   `infra::player_bridge` testable sans handle de fenêtre Tauri.
//!
//! Les couches suivantes (serveur named pipe, parser NDJSON, mise à jour
//! `Watch_Progress`) sont implémentées dans 10.2 → 10.6 et réutilisent
//! `PlaybackSession` produit ici. La couche frontend qui rend le badge
//! « reprise exacte non garantie » est traitée dans la tâche 14.x à
//! partir d'un drapeau exposé via la commande `play_media` (12.3) qui
//! lit, lui, [`LaunchOutcome::External`] retourné par cette couche.
//!
//! ## Tâche 10.5 — Validation existence et exécutabilité du chemin
//!
//! Avant d'enregistrer un chemin External_Player (commande
//! `set_external_player_path`) ou de tenter de lancer le lecteur
//! externe (`play_media`), le backend valide le chemin via
//! [`validate_external_player_path`] :
//!
//! 1. `std::fs::metadata(path)?.is_file()` — refuse si le chemin
//!    n'existe pas, n'est pas accessible, ou ne pointe pas sur un
//!    fichier régulier (cas typiques : dossier, lien symbolique cassé,
//!    chemin vide).
//! 2. Extension du fichier égale à `exe` (comparaison
//!    **insensible à la casse** : `.exe`, `.EXE`, `.Exe`, etc. sont
//!    tous acceptés — Windows considère ces variantes équivalentes).
//!
//! Tout échec retourne [`KyroError::Validation`] avec un message qui
//! cite le chemin fautif (utile pour les logs et le diagnostic
//! utilisateur). Côté `play_media`, l'appelant peut convertir cette
//! erreur en émission de l'événement Tauri
//! [`EVENT_EXTERNAL_PLAYER_MISSING`] pour que le frontend affiche un
//! avis clair (« lecteur externe introuvable »).
//!
//! _Requirements: 9.1, 9.6, 10.3, 10.4, 10.5, 12.4_

use std::path::Path;

use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::error::KyroError;
use crate::infra::ipc_pipe::PipeMessage;
use crate::infra::repo::{ProgressTarget, Repo};

/// Préfixe canonique des pipes Windows alloués par Kyro.
///
/// Exposé pour que les tests d'intégration (tâche 17.1, mock client mpv)
/// et la couche `ipc_pipe` (tâche 10.2) puissent valider / reconstruire
/// le chemin complet sans dupliquer la chaîne magique.
pub const PIPE_NAME_PREFIX: &str = r"\\.\pipe\kyro-";

/// État d'une session de lecture créé par `Kyro_Player_Bridge` au moment
/// de `play_media`.
///
/// * `session_id` — UUID v4 unique pour cette session (requirement 9.6).
/// * `pipe_name` — chemin Windows complet du named pipe serveur, de la
///   forme `\\.\pipe\kyro-<session_id>`.
/// * `script_opts` — argument prêt à être passé à mpv.net via
///   `--script-opts=<value>`, de la forme `kyro-pipe=kyro-<session_id>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackSession {
    pub session_id: Uuid,
    pub pipe_name: String,
    pub script_opts: String,
}

/// Génère un nouvel `PlaybackSession` avec un UUID v4 frais.
///
/// L'unicité du nom de pipe entre deux appels consécutifs repose sur
/// l'unicité statistique d'UUID v4 — propriété testée dans la tâche
/// 10.10 (« Unicité du nom de pipe par session »).
pub fn new_session() -> PlaybackSession {
    let session_id = Uuid::new_v4();
    let pipe_name = format!("{PIPE_NAME_PREFIX}{session_id}");
    let script_opts = format!("kyro-pipe=kyro-{session_id}");
    PlaybackSession {
        session_id,
        pipe_name,
        script_opts,
    }
}

/// Noms d'exécutables reconnus comme mpv.net (insensible à la casse).
///
/// mpv.net est distribué historiquement sous le nom `mpv.net.exe` mais
/// certaines builds packagées par des gestionnaires de paquets exposent
/// l'alias `mpvnet.exe`. Les deux formes doivent activer le chemin IPC
/// (named pipe + script Lua) ; tout autre nom doit basculer sur le
/// chemin « lecteur externe générique » (requirement 10.5, tâche 10.4).
const MPV_NET_EXECUTABLE_NAMES: &[&str] = &["mpv.net.exe", "mpvnet.exe"];

/// Indique si `player_path` pointe vers un exécutable mpv.net.
///
/// La détection se fait uniquement sur le **nom de fichier** (composant
/// final du chemin), insensible à la casse — Windows ne fait pas de
/// distinction de casse sur les noms de fichiers, et l'utilisateur peut
/// très bien avoir tapé `MPV.NET.EXE` dans le sélecteur de fichier.
///
/// Retourne `false` si le chemin n'a pas de composant final (par
/// exemple un chemin vide ou se terminant par un séparateur).
///
/// _Requirements: 9.1, 10.5_
pub fn is_mpv_net(player_path: &Path) -> bool {
    let Some(file_name) = player_path.file_name() else {
        return false;
    };
    let Some(file_name) = file_name.to_str() else {
        return false;
    };
    let lower = file_name.to_ascii_lowercase();
    MPV_NET_EXECUTABLE_NAMES.iter().any(|n| *n == lower)
}

/// Construit un [`tokio::process::Command`] prêt à lancer mpv.net pour
/// la session `session`, sans le spawn.
///
/// L'ordre des arguments est **stable et observable** par les tests :
///
/// 1. `<file_path>` — chemin du fichier média à lire ;
/// 2. `--start=<start_seconds>` — position de reprise exacte en
///    secondes (requirement 9.1, 12.4) ;
/// 3. `--script-opts=<session.script_opts>` — option propagée au script
///    Lua (`kyro-pipe=kyro-<session_id>`) pour qu'il rejoigne le pipe
///    nommé créé côté Kyro (requirement 9.6).
///
/// Cette fonction est exposée publiquement principalement pour la
/// testabilité : un test peut récupérer le [`std::process::Command`]
/// sous-jacent via [`Command::as_std`] et vérifier programme + args
/// sans avoir à spawn un véritable processus.
///
/// _Requirements: 9.1, 12.4_
pub fn build_mpv_command(
    player_path: &Path,
    file_path: &Path,
    start_seconds: u32,
    session: &PlaybackSession,
) -> Command {
    let mut cmd = Command::new(player_path);
    cmd.arg(file_path)
        .arg(format!("--start={start_seconds}"))
        .arg(format!("--script-opts={}", session.script_opts));
    cmd
}

/// Lance mpv.net avec la position de reprise et l'option IPC, et
/// retourne l'enfant asynchrone résultant.
///
/// Wrapper minimal autour de [`build_mpv_command`] suivi de
/// [`Command::spawn`]. Toute la logique d'arguments est dans
/// `build_mpv_command` afin que les tests puissent la valider sans
/// déclencher un véritable lancement de processus.
///
/// _Requirements: 9.1, 12.4_
pub async fn launch_mpv_net(
    player_path: &Path,
    file_path: &Path,
    start_seconds: u32,
    session: &PlaybackSession,
) -> std::io::Result<tokio::process::Child> {
    build_mpv_command(player_path, file_path, start_seconds, session).spawn()
}

// =====================================================================
// Tâche 10.4 — branche External_Player ≠ mpv.net.
// =====================================================================

/// Nom de l'événement Tauri émis lorsqu'un `play_media` est routé vers
/// la branche [`LaunchOutcome::External`] (lecteur tiers, pas mpv.net).
///
/// Le frontend écoute cet événement pour activer, sur la fiche du
/// Media_Item courant, le badge **« reprise exacte non garantie »** :
/// puisqu'aucun pipe IPC n'a été créé pour cette session, Kyro n'a
/// aucun moyen de pousser une position de reprise précise au lecteur
/// tiers ni de récupérer la position de fin (cf. requirement 9.5,
/// property 21 : « préservation de Watch_Progress en l'absence de
/// message IPC »). Le badge est purement informatif côté UI ; aucun
/// payload n'est obligatoire dans l'event — l'appelant Tauri (commande
/// `play_media`, tâche 12.3) peut joindre l'identifiant du Media_Item
/// concerné si plusieurs sessions coexistent.
///
/// **Contrat avec la couche commands (tâche 12.3)** : sur réception de
/// [`LaunchOutcome::External`], l'appelant doit (a) émettre
/// `Window::emit(EVENT_EXTERNAL_PLAYER_NON_MPV, payload)` et (b)
/// signaler dans le DTO `MediaItem` retourné au frontend que la reprise
/// exacte ne peut pas être garantie pour cette session — le composant
/// `MediaCard.svelte` (tâche 14.x) s'appuiera sur ce drapeau pour
/// afficher le badge.
///
/// _Requirements: 10.5_
pub const EVENT_EXTERNAL_PLAYER_NON_MPV: &str = "external-player-non-mpv";

/// Type de lecteur résolu à partir d'un chemin External_Player.
///
/// Décision **purement déterministe** prise par [`branch`] sans
/// effectuer aucun IO ni spawn : c'est l'intermédiaire entre la
/// détection [`is_mpv_net`] (booléen brut) et l'enum [`LaunchOutcome`]
/// (qui embarque l'enfant `tokio::process::Child` et la session IPC).
///
/// Cette indirection existe pour deux raisons :
///
/// 1. **Testabilité** — les tests unitaires peuvent vérifier la branche
///    choisie sans créer de fichier exécutable réel ni spawner un
///    processus, ce qui resterait indispensable pour valider
///    [`launch_player`] sur des chemins arbitraires.
/// 2. **Lisibilité côté commands** — la couche `play_media` peut
///    journaliser, mesurer, ou afficher le mode choisi avant d'engager
///    le spawn (utile pour le diagnostic « pourquoi le badge s'affiche
///    sur cette fiche ? »).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerKind {
    /// Le chemin pointe vers `mpv.net.exe` (ou `mpvnet.exe`) :
    /// l'appelant doit emprunter le chemin IPC complet (named pipe +
    /// `--start=<seconds>` + `--script-opts=…`).
    MpvNet,
    /// Le chemin pointe vers un autre lecteur (VLC, PotPlayer, mpv
    /// stock, …) : l'appelant doit lancer le lecteur uniquement avec
    /// le chemin du fichier média, ne pas créer de pipe, et émettre
    /// l'événement [`EVENT_EXTERNAL_PLAYER_NON_MPV`].
    External,
}

/// Décide si `player_path` doit être lancé via la branche IPC mpv.net
/// ou la branche « lecteur externe générique ».
///
/// Pure-logic : pas d'IO, pas de spawn — délègue uniquement à
/// [`is_mpv_net`]. Cette fonction est l'API recommandée pour les
/// callers qui veulent tester ou journaliser le routage avant de
/// déclencher le lancement effectif via [`launch_player`].
///
/// _Requirements: 10.5_
pub fn branch(player_path: &Path) -> PlayerKind {
    if is_mpv_net(player_path) {
        PlayerKind::MpvNet
    } else {
        PlayerKind::External
    }
}

/// Construit un [`tokio::process::Command`] pour un lecteur tiers
/// (branche `External`), sans le spawn.
///
/// Contrairement à [`build_mpv_command`], **un seul argument** est
/// passé : le chemin du fichier média. Aucun `--start=<seconds>`, aucun
/// `--script-opts=…` ; ces options sont propres à mpv et risqueraient
/// d'être interprétées comme des arguments positionnels (donc des
/// fichiers à ouvrir) par d'autres lecteurs.
///
/// Exposé publiquement pour la testabilité : un test peut inspecter
/// les args produits via [`Command::as_std`] sans déclencher de spawn.
///
/// _Requirements: 10.5_
pub fn build_external_command(player_path: &Path, file_path: &Path) -> Command {
    let mut cmd = Command::new(player_path);
    cmd.arg(file_path);
    cmd
}

/// Lance un lecteur tiers (non-mpv.net) avec uniquement le chemin du
/// fichier média en argument, et retourne l'enfant asynchrone résultant.
///
/// **Aucun named pipe n'est créé** par cette fonction (property 23,
/// requirement 10.5) : la couche `commands::play_media` (tâche 12.3)
/// est responsable, sur retour de [`LaunchOutcome::External`], d'émettre
/// l'événement [`EVENT_EXTERNAL_PLAYER_NON_MPV`] vers le frontend afin
/// que le badge « reprise exacte non garantie » s'affiche sur la fiche
/// du Media_Item courant.
///
/// _Requirements: 10.5_
pub async fn launch_external_player(
    player_path: &Path,
    file_path: &Path,
) -> std::io::Result<Child> {
    build_external_command(player_path, file_path).spawn()
}

/// Issue d'un appel à [`launch_player`] : enveloppe l'enfant spawné et,
/// dans le cas mpv.net, la session IPC associée.
///
/// **Contrat avec la couche `commands::play_media` (tâche 12.3)** :
///
/// * `MpvNet { session, child }` — la session IPC est active, l'appelant
///   doit attacher la couche `ipc_pipe::listen(&session.pipe_name)`
///   (tâche 10.6) pour persister la progression au fil des messages
///   NDJSON. La position de reprise initiale (`start_seconds`) a déjà
///   été propagée à mpv.net via `--start=…`.
/// * `External { child }` — pas de session IPC, **pas de pipe**, pas de
///   `--start`. L'appelant doit émettre l'événement
///   [`EVENT_EXTERNAL_PLAYER_NON_MPV`] et signaler au frontend (via le
///   DTO `MediaItem`) que la reprise exacte ne peut pas être garantie
///   pour cette session : le badge correspondant sera rendu par la
///   tâche 14.x.
#[derive(Debug)]
pub enum LaunchOutcome {
    /// Lancement mpv.net réussi, avec session IPC à attacher au
    /// receiver `ipc_pipe::listen(&session.pipe_name)`.
    MpvNet {
        session: PlaybackSession,
        child: Child,
    },
    /// Lancement d'un lecteur tiers réussi, sans IPC. L'appelant doit
    /// émettre [`EVENT_EXTERNAL_PLAYER_NON_MPV`] et marquer la fiche
    /// Media_Item « reprise exacte non garantie ».
    External { child: Child },
}

/// Façade unifiée : route le lancement vers la branche mpv.net (avec
/// IPC) ou la branche lecteur tiers (sans IPC) selon le résultat de
/// [`branch`].
///
/// Cette fonction reste **Tauri-free** : elle ne reçoit pas de handle
/// `Window<R>` et n'émet aucun événement. L'appelant Tauri-aware
/// (commande `play_media`, tâche 12.3) inspecte la variante retournée
/// dans [`LaunchOutcome`] et compose l'effet UI correspondant
/// (`Window::emit(EVENT_EXTERNAL_PLAYER_NON_MPV, …)` sur la branche
/// `External`).
///
/// **Note pour 12.3** : `LaunchOutcome::External` doit déclencher dans
/// le DTO `MediaItem` un drapeau « reprise exacte non garantie » (côté
/// frontend, tâche 14.x, le composant `MediaCard.svelte` rend ce
/// drapeau sous forme de badge). `LaunchOutcome::MpvNet` n'a pas
/// besoin de ce drapeau : la position est garantie via `--start=…`.
///
/// _Requirements: 9.1, 10.5, 12.4_
pub async fn launch_player(
    player_path: &Path,
    file_path: &Path,
    start_seconds: u32,
) -> std::io::Result<LaunchOutcome> {
    match branch(player_path) {
        PlayerKind::MpvNet => {
            let session = new_session();
            let child =
                launch_mpv_net(player_path, file_path, start_seconds, &session).await?;
            Ok(LaunchOutcome::MpvNet { session, child })
        }
        PlayerKind::External => {
            let child = launch_external_player(player_path, file_path).await?;
            Ok(LaunchOutcome::External { child })
        }
    }
}

/// Nom de l'événement Tauri émis au lancement d'un `play_media` lorsque
/// le chemin External_Player enregistré ne pointe plus vers un fichier
/// `.exe` valide (lecteur déplacé, désinstallé, partition absente,
/// fichier corrompu).
///
/// L'émission est faite par la couche commands : si
/// [`validate_external_player_path`] retourne `Err(Validation)` au
/// moment de `play_media`, la commande émet cet événement (sans
/// payload obligatoire — le frontend connaît déjà le chemin configuré
/// via `get_external_player_path`) et abandonne le lancement.
///
/// _Requirements: 10.3, 10.4_
pub const EVENT_EXTERNAL_PLAYER_MISSING: &str = "external-player-missing";

/// Valide qu'un chemin External_Player est utilisable comme exécutable.
///
/// Effectue, dans l'ordre, deux vérifications :
///
/// 1. **Existence + fichier régulier** : `std::fs::metadata(path)?`
///    suivi de `is_file()`. Couvre les cas
///    « chemin inexistant », « dossier », « lien symbolique cassé »,
///    « partition non montée », « permissions insuffisantes ».
/// 2. **Extension `.exe` insensible à la casse** : l'extension du
///    fichier (composant après le dernier `.`) doit valoir
///    `exe` une fois normalisée en minuscules. `.EXE`, `.Exe`,
///    `.exe` sont tous acceptés ; `.bat`, `.cmd`, ou l'absence
///    d'extension sont refusés.
///
/// Retourne `Ok(())` si les deux conditions tiennent, sinon
/// [`KyroError::Validation`] avec un message FR citant le chemin
/// fautif (le message est déjà adapté au requirement 18.4 : il sera
/// exposé tel quel dans `KyroError::Validation` puis traduit en
/// `« Erreur de validation : … »` côté Tauri).
///
/// Cette fonction est appelée :
/// * par `set_external_player_path` (commande Tauri) — refuse
///   l'enregistrement et propage l'erreur au frontend ;
/// * par `play_media` (commande Tauri) — bascule sur l'émission de
///   l'événement [`EVENT_EXTERNAL_PLAYER_MISSING`] et abandonne le
///   lancement, sans modifier la configuration enregistrée.
///
/// _Requirements: 10.3, 10.4_
pub fn validate_external_player_path(path: &Path) -> Result<(), KyroError> {
    // 1. Existence + fichier régulier.
    let metadata = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(err) => {
            tracing::warn!(
                path = %path.display(),
                error = %err,
                "Kyro: chemin External_Player refusé — métadonnées FS inaccessibles"
            );
            return Err(KyroError::Validation(format!(
                "le chemin du lecteur externe {} est inaccessible ou n'existe pas",
                path.display()
            )));
        }
    };
    if !metadata.is_file() {
        tracing::warn!(
            path = %path.display(),
            "Kyro: chemin External_Player refusé — n'est pas un fichier régulier"
        );
        return Err(KyroError::Validation(format!(
            "le chemin du lecteur externe {} n'est pas un fichier",
            path.display()
        )));
    }

    // 2. Extension `.exe` insensible à la casse.
    //
    // `Path::extension` renvoie déjà uniquement le dernier segment après
    // le `.`, donc une comparaison `eq_ignore_ascii_case` suffit ici
    // (les extensions Windows sont ASCII pur en pratique). Si
    // l'extension est absente, `extension()` retourne `None` → refus.
    let ext_ok = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("exe"))
        .unwrap_or(false);
    if !ext_ok {
        tracing::warn!(
            path = %path.display(),
            "Kyro: chemin External_Player refusé — extension différente de .exe"
        );
        return Err(KyroError::Validation(format!(
            "le chemin du lecteur externe {} n'a pas l'extension .exe",
            path.display()
        )));
    }

    Ok(())
}

// =====================================================================
// Tâche 10.6 — persistance de la progression et de la position finale.
// =====================================================================

/// Nom canonique de l'événement Tauri émis lorsque la session IPC mpv
/// se termine (sur réception d'un message [`PipeMessage::Closed`] ou,
/// plus généralement, sur EOF du pipe). Le frontend s'abonne à cet
/// événement pour, par exemple, raffraîchir la fiche du Media_Item
/// courant et rendre le nouveau statut / la nouvelle progression.
///
/// Aucun payload structuré n'est obligatoire ; les call-sites
/// (`commands::play_media`, tâche 12.3) peuvent y joindre la position
/// finale écrite et la cible (`MediaItemId` / `EpisodeItemId`) si la
/// fenêtre observe simultanément plusieurs sessions.
///
/// _Requirements: 9.3, 9.4_
pub const EVENT_PLAYBACK_ENDED: &str = "playback-ended";

/// Nom canonique de l'événement Tauri émis **à chaque message de
/// progression** reçu pendant la session IPC mpv (`Progress`, `Paused`,
/// `Resumed`). Le payload est un [`PlaybackProgress`] qui contient la
/// cible (kind + id) et la position en secondes — suffisant pour que
/// le frontend mette à jour la chip « X min restantes » et la barre de
/// progression sur la fiche détail courante sans avoir à recharger.
///
/// L'émission est best-effort : un échec (fenêtre fermée, canal IPC
/// saturé) est journalisé mais n'interrompt pas la session, la base
/// restant la source de vérité.
///
/// _Requirements: 9.2, 9.4_
pub const EVENT_PLAYBACK_PROGRESS: &str = "playback-progress";

/// Payload de l'événement [`EVENT_PLAYBACK_PROGRESS`]. Sérialisable
/// JSON, consommé par le frontend pour mettre à jour la fiche détail
/// en direct pendant qu'mpv lit le fichier.
#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq)]
pub struct PlaybackProgress {
    /// `"media"` ou `"episode"` selon la cible (cohérent avec le
    /// discriminant accepté par la commande `play_media`).
    pub kind: &'static str,
    /// Identifiant de la cible (`media_item.id` ou `episode_item.id`).
    pub target_id: i64,
    /// Position courante en secondes (clampée comme côté DB).
    pub position_seconds: u32,
}

/// Consomme le receiver de [`crate::infra::ipc_pipe::listen`] et
/// persiste la progression au fil des messages.
///
/// Comportement détaillé :
///
/// * Pour chaque [`PipeMessage::Progress`], [`PipeMessage::Paused`] ou
///   [`PipeMessage::Resumed`] reçu, la position transmise (`f64`
///   secondes) est tronquée à `u32` (clamp à `0` pour une position
///   négative et à [`u32::MAX`] pour un débordement — pratique
///   uniquement pour des durées extrêmes), puis propagée vers
///   [`Repo::set_watch_progress`] qui recalcule le statut via
///   [`crate::domain::status::compute_status`] et commit (req. 9.3,
///   17.2 — persistance avant signal de succès).
/// * Sur [`PipeMessage::Closed`], la position finale est écrite **une
///   dernière fois** (afin de garantir que la valeur la plus récente
///   est toujours celle persistée, même si le client mpv n'a pas
///   poussé de `Progress` juste avant la fermeture), puis la boucle
///   sort. Le call-site refermera ensuite le pipe (drop du receiver →
///   fin de la tâche `ipc_pipe::listen`) et émettra
///   [`EVENT_PLAYBACK_ENDED`] (cf. [`run_ipc_session_emit`]).
/// * Si **aucun** message n'est reçu avant que le sender ne soit drop
///   (pipe vide jusqu'à fermeture du processus joueur ou erreur de
///   connexion), `set_watch_progress` n'est **pas** appelé : la valeur
///   `watch_progress_seconds` existante en base est préservée
///   (requirement 9.5, property 21).
///
/// La valeur de retour est la dernière position effectivement écrite
/// (en secondes, `u32`) ou `None` si aucun message n'a transité — le
/// call-site Tauri-aware peut s'en servir comme payload de
/// `playback-ended`.
///
/// Si une écriture DB échoue pour un message intermédiaire, l'erreur
/// est journalisée via `tracing::warn!` et la boucle **continue** :
/// perdre une mise à jour de progression intermédiaire est moins grave
/// que d'interrompre la session (le prochain `Progress` ou le `Closed`
/// final écrasera la valeur). Une erreur sur le `Closed` final, en
/// revanche, est propagée à l'appelant via `Err(KyroError::Db | _)`,
/// car c'est la garantie de cohérence finale du requirement 9.4.
///
/// _Requirements: 9.3, 9.4, 9.5, 17.2_
pub async fn run_ipc_session(
    repo: &Repo,
    target: ProgressTarget,
    mut rx: mpsc::Receiver<PipeMessage>,
) -> Result<Option<u32>, KyroError> {
    let mut current_target = target;
    let mut last_written: Option<u32> = None;
    let mut max_seen: u32 = 0;
    let mut last_duration: u32 = 0;

    while let Some(msg) = rx.recv().await {
        match msg {
            PipeMessage::Progress {
                position, duration, ..
            } => {
                let pos = clamp_position_to_u32(position);
                let dur = clamp_position_to_u32(duration);
                if dur > 0 {
                    last_duration = dur;
                }
                if pos > max_seen {
                    max_seen = pos;
                }
                match repo.set_watch_progress(current_target, pos).await {
                    Ok(_status) => {
                        last_written = Some(pos);
                    }
                    Err(err) => {
                        tracing::warn!(
                            position = pos,
                            error = %err,
                            "Kyro_Player_Bridge: échec persistance progression intermédiaire (ignoré)"
                        );
                    }
                }
            }
            PipeMessage::Paused { position, .. }
            | PipeMessage::Resumed { position, .. } => {
                let pos = clamp_position_to_u32(position);
                if pos > max_seen {
                    max_seen = pos;
                }
                match repo.set_watch_progress(current_target, pos).await {
                    Ok(_status) => {
                        last_written = Some(pos);
                    }
                    Err(err) => {
                        tracing::warn!(
                            position = pos,
                            error = %err,
                            "Kyro_Player_Bridge: échec persistance progression intermédiaire (ignoré)"
                        );
                    }
                }
            }
            PipeMessage::Completed {
                position,
                duration,
                path,
                ..
            } => {
                let dur = clamp_position_to_u32(duration);
                let pos = clamp_position_to_u32(position);
                let final_pos = if dur > 0 {
                    dur
                } else if last_duration > 0 {
                    last_duration
                } else {
                    pos.max(max_seen)
                };
                let completion_target = if let Some(ref p) = path {
                    match repo.find_progress_target_by_path(p).await {
                        Ok(Some(t)) => t,
                        _ => current_target,
                    }
                } else {
                    current_target
                };
                repo.set_watch_progress(completion_target, final_pos).await?;
                last_written = Some(final_pos);
                max_seen = 0;
                last_duration = 0;
            }
            PipeMessage::FileChanged { path, .. } => {
                if last_duration > 0 || max_seen > 0 {
                    let final_pos = if last_duration > 0 {
                        last_duration
                    } else {
                        max_seen
                    };
                    let _ = repo
                        .set_watch_progress(current_target, final_pos)
                        .await;
                }
                if let Ok(Some(new_target)) =
                    repo.find_progress_target_by_path(&path).await
                {
                    if new_target != current_target {
                        current_target = new_target;
                        max_seen = 0;
                        last_duration = 0;
                        last_written = None;
                    }
                }
            }
            PipeMessage::Closed { position, .. } => {
                let pos = clamp_position_to_u32(position);
                // Garde-fou principal : si on a déjà observé une
                // position non-nulle pendant cette session (max_seen)
                // et que le closed régresse de plus de 5 s, on
                // préserve max_seen. Tolérance pour le rewind
                // utilisateur juste avant la fermeture.
                let effective_pos = if max_seen > 0 && pos + 5 < max_seen {
                    max_seen
                } else if pos == 0 && last_written.is_some() && last_written != Some(0) {
                    // Garde-fou secondaire : si max_seen a été reset
                    // par un Completed qui a écrit une valeur non
                    // nulle (last_written = duration), un closed à 0
                    // ne doit pas écraser. Cas typique :
                    //   Completed (force à 600) → Closed (mpv libère
                    //   state, envoie 0)
                    last_written.unwrap()
                } else {
                    pos
                };
                repo.set_watch_progress(current_target, effective_pos).await?;
                last_written = Some(effective_pos);
                break;
            }
        }
    }

    Ok(last_written)
}

/// Wrapper Tauri-aware autour de [`run_ipc_session`] qui, à la
/// terminaison de la session, émet l'événement [`EVENT_PLAYBACK_ENDED`]
/// sur la fenêtre fournie.
///
/// L'émission est best-effort (cohérent avec le pattern utilisé par
/// `scan_folder_emit` pour `scan-progress`) : si la fenêtre est fermée
/// ou si le canal IPC est saturé, l'erreur est journalisée mais la
/// fonction retourne tout de même `Ok(last_written)` — la persistance
/// DB reste la source de vérité, l'événement n'est qu'une notification
/// UI.
///
/// La position finale écrite (`Option<u32>`) est jointe en payload de
/// l'événement, ce qui permet au frontend de mettre à jour la fiche
/// sans recharger.
///
/// _Requirements: 9.3, 9.4, 9.5_
pub async fn run_ipc_session_emit<R: tauri::Runtime>(
    window: &tauri::Window<R>,
    repo: &Repo,
    target: ProgressTarget,
    mut rx: mpsc::Receiver<PipeMessage>,
) -> Result<Option<u32>, KyroError> {
    use tauri::Emitter;

    // Cible courante de la session — peut être ré-assignée en cours
    // de session si mpv signale un changement de fichier (autoplay,
    // playlist-next). On garde aussi des miroirs `progress_kind` /
    // `target_id` pour les payloads `playback-progress` côté UI.
    let mut current_target = target;
    let (mut progress_kind, mut target_id) = describe_target(current_target);

    let mut last_written: Option<u32> = None;
    // Maximum de la position observée pendant la **session courante**
    // (réinitialisé à chaque `FileChanged`) — utilisé en garde-fou sur
    // le `Closed` final pour empêcher une régression pathologique.
    let mut max_seen: u32 = 0;
    // Dernière `duration` connue de la session courante.
    let mut last_duration: u32 = 0;

    while let Some(msg) = rx.recv().await {
        match msg {
            PipeMessage::Progress {
                position, duration, ..
            } => {
                let pos = clamp_position_to_u32(position);
                let dur = clamp_position_to_u32(duration);
                if dur > 0 {
                    last_duration = dur;
                }
                if pos > max_seen {
                    max_seen = pos;
                }
                match repo.set_watch_progress(current_target, pos).await {
                    Ok(_status) => {
                        last_written = Some(pos);
                        let payload = PlaybackProgress {
                            kind: progress_kind,
                            target_id,
                            position_seconds: pos,
                        };
                        if let Err(err) =
                            window.emit(EVENT_PLAYBACK_PROGRESS, payload)
                        {
                            tracing::warn!(
                                event = EVENT_PLAYBACK_PROGRESS,
                                target_id = target_id,
                                error = %err,
                                "Kyro_Player_Bridge: échec émission playback-progress (best-effort, ignoré)"
                            );
                        }
                    }
                    Err(err) => {
                        tracing::warn!(
                            position = pos,
                            error = %err,
                            "Kyro_Player_Bridge: échec persistance progression intermédiaire (ignoré)"
                        );
                    }
                }
            }
            PipeMessage::Paused { position, .. }
            | PipeMessage::Resumed { position, .. } => {
                let pos = clamp_position_to_u32(position);
                if pos > max_seen {
                    max_seen = pos;
                }
                match repo.set_watch_progress(current_target, pos).await {
                    Ok(_status) => {
                        last_written = Some(pos);
                        let payload = PlaybackProgress {
                            kind: progress_kind,
                            target_id,
                            position_seconds: pos,
                        };
                        if let Err(err) =
                            window.emit(EVENT_PLAYBACK_PROGRESS, payload)
                        {
                            tracing::warn!(
                                event = EVENT_PLAYBACK_PROGRESS,
                                target_id = target_id,
                                error = %err,
                                "Kyro_Player_Bridge: échec émission playback-progress (best-effort, ignoré)"
                            );
                        }
                    }
                    Err(err) => {
                        tracing::warn!(
                            position = pos,
                            error = %err,
                            "Kyro_Player_Bridge: échec persistance progression intermédiaire (ignoré)"
                        );
                    }
                }
            }
            PipeMessage::Completed {
                position,
                duration,
                path,
                ..
            } => {
                let dur = clamp_position_to_u32(duration);
                let pos = clamp_position_to_u32(position);
                let final_pos = if dur > 0 {
                    dur
                } else if last_duration > 0 {
                    last_duration
                } else {
                    pos.max(max_seen)
                };

                // Si le Lua nous indique le chemin qui se termine, on
                // résout la cible de complétion à partir de ce chemin
                // (robuste à un FileChanged arrivé en course juste
                // avant). Sinon on retombe sur la cible courante.
                let completion_target = if let Some(ref p) = path {
                    match repo.find_progress_target_by_path(p).await {
                        Ok(Some(t)) => t,
                        _ => current_target,
                    }
                } else {
                    current_target
                };

                let (comp_kind, comp_id) = describe_target(completion_target);

                repo.set_watch_progress(completion_target, final_pos).await?;
                last_written = Some(final_pos);
                tracing::info!(
                    target_id = comp_id,
                    final_pos = final_pos,
                    duration = dur,
                    "Kyro_Player_Bridge: end-of-file reçu — position forcée à la durée totale"
                );
                let payload = PlaybackProgress {
                    kind: comp_kind,
                    target_id: comp_id,
                    position_seconds: final_pos,
                };
                if let Err(err) = window.emit(EVENT_PLAYBACK_PROGRESS, payload) {
                    tracing::warn!(
                        event = EVENT_PLAYBACK_PROGRESS,
                        error = %err,
                        "Kyro_Player_Bridge: échec émission playback-progress (completed)"
                    );
                }
                // Ne PAS break : mpv peut enchaîner sur un fichier
                // suivant (autoplay) ; on attend `FileChanged` ou
                // `Closed` pour décider de la suite. Le `Completed`
                // marque la fin du fichier courant, pas de la session.
                max_seen = 0;
                last_duration = 0;
            }
            PipeMessage::FileChanged { path, .. } => {
                tracing::info!(
                    incoming_path = %path,
                    previous_target = ?current_target,
                    max_seen = max_seen,
                    last_duration = last_duration,
                    "Kyro_Player_Bridge: file-changed reçu"
                );
                // Avant de basculer : on marque la cible précédente
                // comme **vue**, peu importe la position observée.
                // Sémantique : un changement de fichier en cours de
                // session signifie que l'utilisateur en a fini avec
                // le précédent (skip volontaire, autoplay, etc.). Si
                // la lecture a réellement été abandonnée tôt, l'UI
                // permet de revenir au statut « En cours » via le
                // bouton de la fiche détail.
                //
                // Cas particulier : si on n'a observé aucune position
                // (max_seen == 0) ET aucune durée (last_duration ==
                // 0), c'est que le fichier précédent n'a jamais
                // vraiment démarré (mpv a planté au load, fichier
                // corrompu, etc.). On ne touche pas à la base dans
                // ce cas pour ne pas marquer vu un fichier jamais lu.
                if last_duration > 0 || max_seen > 0 {
                    let final_pos = if last_duration > 0 {
                        last_duration
                    } else {
                        max_seen
                    };
                    tracing::info!(
                        previous_target = ?current_target,
                        final_pos = final_pos,
                        "Kyro_Player_Bridge: file-changed — marque la cible précédente comme vue"
                    );
                    if let Err(err) = repo
                        .set_watch_progress(current_target, final_pos)
                        .await
                    {
                        tracing::warn!(
                            error = %err,
                            "Kyro_Player_Bridge: échec marquage Vu sur file-changed (ignoré)"
                        );
                    } else {
                        let payload = PlaybackProgress {
                            kind: progress_kind,
                            target_id,
                            position_seconds: final_pos,
                        };
                        let _ = window.emit(EVENT_PLAYBACK_PROGRESS, payload);
                    }
                }

                match repo.find_progress_target_by_path(&path).await {
                    Ok(Some(new_target)) if new_target != current_target => {
                        let (new_kind, new_id) = describe_target(new_target);
                        tracing::info!(
                            previous_target = ?current_target,
                            new_target = ?new_target,
                            new_path = %path,
                            "Kyro_Player_Bridge: file-changed — bascule de cible IPC"
                        );
                        current_target = new_target;
                        progress_kind = new_kind;
                        target_id = new_id;
                        max_seen = 0;
                        last_duration = 0;
                        last_written = None;
                    }
                    Ok(Some(_)) => {
                        tracing::info!(
                            new_path = %path,
                            "Kyro_Player_Bridge: file-changed vers la même cible — pas de switch"
                        );
                    }
                    Ok(None) => {
                        tracing::warn!(
                            new_path = %path,
                            "Kyro_Player_Bridge: file-changed vers chemin INTROUVABLE en base — IPC inchangée (path non canonicalisable ?)"
                        );
                    }
                    Err(err) => {
                        tracing::warn!(
                            new_path = %path,
                            error = %err,
                            "Kyro_Player_Bridge: échec résolution file-changed (ignoré)"
                        );
                    }
                }
            }
            PipeMessage::Closed { position, .. } => {
                let pos = clamp_position_to_u32(position);
                let effective_pos = if max_seen > 0 && pos + 5 < max_seen {
                    tracing::info!(
                        closed_pos = pos,
                        max_seen = max_seen,
                        "Kyro_Player_Bridge: closed régressif ignoré (préserve max observé)"
                    );
                    max_seen
                } else if pos == 0 && last_written.is_some() && last_written != Some(0) {
                    // Garde-fou secondaire : après un Completed qui
                    // a reset max_seen mais écrit une valeur non
                    // nulle (last_written = duration), un closed à 0
                    // ne doit pas écraser cette valeur.
                    tracing::info!(
                        closed_pos = pos,
                        last_written = ?last_written,
                        "Kyro_Player_Bridge: closed à 0 après écriture non-nulle ignoré (préserve last_written)"
                    );
                    last_written.unwrap()
                } else {
                    pos
                };
                repo.set_watch_progress(current_target, effective_pos).await?;
                last_written = Some(effective_pos);
                let payload = PlaybackProgress {
                    kind: progress_kind,
                    target_id,
                    position_seconds: effective_pos,
                };
                if let Err(err) = window.emit(EVENT_PLAYBACK_PROGRESS, payload) {
                    tracing::warn!(
                        event = EVENT_PLAYBACK_PROGRESS,
                        target_id = target_id,
                        error = %err,
                        "Kyro_Player_Bridge: échec émission playback-progress final (ignoré)"
                    );
                }
                break;
            }
        }
    }

    if let Err(err) = window.emit(EVENT_PLAYBACK_ENDED, last_written) {
        tracing::warn!(
            event = EVENT_PLAYBACK_ENDED,
            last_written = ?last_written,
            error = %err,
            "Kyro_Player_Bridge: échec émission playback-ended (best-effort, ignoré)"
        );
    }

    Ok(last_written)
}

/// Convertit une position en secondes (`f64` en provenance du script
/// Lua mpv) en `u32` adapté à `set_watch_progress`. Une valeur négative
/// (théoriquement impossible côté mpv, mais le script peut être
/// remplacé) est clamp à `0`. Une valeur supérieure à [`u32::MAX`]
/// (soit ≈ 136 ans en secondes — purement pathologique) est clamp à
/// [`u32::MAX`] plutôt que de panic via `as u32` saturant.
fn clamp_position_to_u32(position: f64) -> u32 {
    if position.is_nan() || position <= 0.0 {
        0
    } else if position >= u32::MAX as f64 {
        // Couvre aussi `f64::INFINITY` qui est `>= u32::MAX as f64`.
        u32::MAX
    } else {
        position as u32
    }
}

/// Projette un [`ProgressTarget`] en `(kind, id)` pour les payloads
/// `playback-progress` consommés par le frontend. Helper local utilisé
/// par [`run_ipc_session_emit`] qui peut ré-assigner sa cible courante
/// à chaque message `FileChanged`.
fn describe_target(target: ProgressTarget) -> (&'static str, i64) {
    match target {
        ProgressTarget::Media(id) => ("media", id.get()),
        ProgressTarget::Episode(id) => ("episode", id.get()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Sanity check du préfixe : il doit correspondre exactement à la
    /// forme attendue par `NamedPipeServer` côté Windows et par le
    /// script Lua côté mpv.net (`\\.\pipe\kyro-`).
    #[test]
    fn pipe_name_prefix_is_canonical_windows_path() {
        assert_eq!(PIPE_NAME_PREFIX, "\\\\.\\pipe\\kyro-");
    }

    /// Le nom de pipe d'une session commence toujours par le préfixe
    /// canonique — invariant utilisé par la couche `ipc_pipe` pour
    /// valider l'entrée.
    #[test]
    fn new_session_pipe_name_has_expected_prefix() {
        let session = new_session();
        assert!(
            session.pipe_name.starts_with(PIPE_NAME_PREFIX),
            "pipe_name `{}` ne commence pas par `{}`",
            session.pipe_name,
            PIPE_NAME_PREFIX
        );
    }

    /// Cohérence interne : `session_id`, `pipe_name` et `script_opts`
    /// doivent référer au même UUID. Cette propriété est essentielle
    /// car le script Lua (qui ne reçoit que `script_opts`) doit pouvoir
    /// reconstruire le pipe que le serveur Rust a réellement créé.
    #[test]
    fn new_session_fields_are_mutually_consistent() {
        let session = new_session();
        let id = session.session_id.to_string();

        assert_eq!(session.pipe_name, format!("{PIPE_NAME_PREFIX}{id}"));
        assert_eq!(session.script_opts, format!("kyro-pipe=kyro-{id}"));

        // Le suffixe du pipe (après le préfixe) doit correspondre au
        // nom logique embarqué dans script_opts (`kyro-<id>`).
        let logical_name = session
            .pipe_name
            .strip_prefix(r"\\.\pipe\")
            .expect("pipe_name doit commencer par `\\\\.\\pipe\\`");
        assert_eq!(session.script_opts, format!("kyro-pipe={logical_name}"));
    }

    /// Deux appels consécutifs à `new_session()` produisent des noms de
    /// pipe distincts grâce à l'unicité d'UUID v4 (requirement 9.6).
    /// La propriété complète sera revérifiée par fast-check / proptest
    /// dans la tâche 10.10 ; ce test sert de garde-fou immédiat.
    #[test]
    fn two_consecutive_sessions_have_distinct_pipe_names() {
        let a = new_session();
        let b = new_session();
        assert_ne!(a.session_id, b.session_id);
        assert_ne!(a.pipe_name, b.pipe_name);
        assert_ne!(a.script_opts, b.script_opts);
    }

    // -----------------------------------------------------------------
    // Tâche 10.3 — détection mpv.net + construction de la commande.
    // -----------------------------------------------------------------

    /// `is_mpv_net` reconnaît `mpv.net.exe` quel que soit le casing du
    /// nom de fichier — Windows traite les noms en insensible à la casse
    /// et l'utilisateur peut très bien tomber sur `MPV.NET.EXE` selon le
    /// sélecteur natif. Couvre aussi l'alias `mpvnet.exe` distribué par
    /// certaines builds packagées.
    #[test]
    fn is_mpv_net_accepts_canonical_executable_names() {
        let cases = [
            r"C:\path\mpv.net.exe",
            r"C:\path\MPV.NET.EXE",
            r"C:\Path\Mpv.Net.Exe",
            "mpvnet.exe",
            "MPVNET.EXE",
            r"D:\Tools\mpv\mpvnet.exe",
        ];
        for raw in cases {
            let path = PathBuf::from(raw);
            assert!(
                is_mpv_net(&path),
                "is_mpv_net devrait accepter `{raw}`"
            );
        }
    }

    /// `is_mpv_net` rejette tout nom d'exécutable qui n'est pas mpv.net,
    /// y compris `mpv.exe` (mpv « stock », pas la variante .NET) — c'est
    /// crucial pour ne pas activer l'IPC sur un lecteur incompatible
    /// (requirement 10.5 : « WHERE External_Player n'est pas mpv.net,
    /// THE Kyro_Player_Bridge SHALL [...] sans tenter d'utiliser
    /// Kyro_IPC_Pipe »).
    #[test]
    fn is_mpv_net_rejects_other_players() {
        let cases = [
            r"C:\Program Files\VideoLAN\VLC\vlc.exe",
            r"C:\path\mpv.exe",
            r"C:\path\MPV.EXE",
            r"C:\path\mpv-net.exe", // tiret au lieu d'un point
            r"C:\path\mpvnet",      // pas d'extension .exe
            r"C:\path\mpv.net",     // pas d'extension .exe
            r"C:\path\PotPlayerMini64.exe",
            "",
        ];
        for raw in cases {
            let path = PathBuf::from(raw);
            assert!(
                !is_mpv_net(&path),
                "is_mpv_net devrait rejeter `{raw}`"
            );
        }
    }

    /// `build_mpv_command` doit produire une commande dont le programme
    /// est l'exécutable mpv.net passé en argument et dont les args sont,
    /// dans l'ordre exact attendu par mpv.net :
    ///
    ///   1. le chemin du fichier média ;
    ///   2. `--start=<start_seconds>` ;
    ///   3. `--script-opts=kyro-pipe=kyro-<session_id>`.
    ///
    /// L'ordre est observable côté test via `Command::as_std().get_args()`.
    /// Cette propriété verrouille le contrat avec mpv.net et avec le
    /// script Lua (qui parse `--script-opts`).
    #[test]
    fn build_mpv_command_args_are_ordered_and_complete() {
        let player = PathBuf::from(r"C:\path\mpv.net.exe");
        let file = PathBuf::from(r"D:\Media\Films\Inception (2010).mkv");
        let start: u32 = 1234;
        let session = new_session();

        let cmd = build_mpv_command(&player, &file, start, &session);
        let std_cmd = cmd.as_std();

        // Programme : le binaire mpv.net configuré.
        assert_eq!(std_cmd.get_program(), player.as_os_str());

        // Args : ordre stable, valeurs exactes.
        let args: Vec<&std::ffi::OsStr> = std_cmd.get_args().collect();
        assert_eq!(args.len(), 3, "build_mpv_command doit produire exactement 3 args");
        assert_eq!(args[0], file.as_os_str());
        assert_eq!(args[1], std::ffi::OsStr::new(&format!("--start={start}")));
        assert_eq!(
            args[2],
            std::ffi::OsStr::new(&format!(
                "--script-opts=kyro-pipe=kyro-{}",
                session.session_id
            ))
        );

        // Garde-fou supplémentaire : le 3ᵉ argument est bien construit à
        // partir du `script_opts` du `PlaybackSession` (cohérence avec la
        // tâche 10.1) et embarque le préfixe `kyro-pipe=`.
        let script_opts_arg = args[2].to_string_lossy();
        assert_eq!(
            script_opts_arg,
            format!("--script-opts={}", session.script_opts)
        );
        assert!(script_opts_arg.starts_with("--script-opts=kyro-pipe=kyro-"));
    }

    /// `build_mpv_command` accepte `start_seconds = 0` (cas « pas encore
    /// commencé ») et produit un `--start=0` valide pour mpv.net.
    #[test]
    fn build_mpv_command_supports_zero_start() {
        let player = PathBuf::from(r"C:\path\mpv.net.exe");
        let file = PathBuf::from(r"D:\Media\episode.mkv");
        let session = new_session();

        let cmd = build_mpv_command(&player, &file, 0, &session);
        let args: Vec<String> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[1], "--start=0");
    }

    // -----------------------------------------------------------------
    // Tâche 10.4 — branche External_Player ≠ mpv.net.
    // -----------------------------------------------------------------

    /// La constante `EVENT_EXTERNAL_PLAYER_NON_MPV` est exportée et
    /// vaut exactement la chaîne `"external-player-non-mpv"` — c'est
    /// le contrat figé avec le frontend (tâche 14.x), qui s'abonne à
    /// cet événement par son nom littéral pour activer le badge UI
    /// « reprise exacte non garantie ».
    #[test]
    fn event_external_player_non_mpv_constant_value() {
        assert_eq!(EVENT_EXTERNAL_PLAYER_NON_MPV, "external-player-non-mpv");
    }

    /// `branch` retourne `MpvNet` ssi `is_mpv_net` retourne `true`. Ce
    /// test verrouille la cohérence entre les deux APIs : la couche
    /// `commands::play_media` peut se reposer indifféremment sur l'une
    /// ou l'autre, elles ne peuvent jamais diverger.
    #[test]
    fn branch_matches_is_mpv_net_decision() {
        let mpv_cases = [
            r"C:\path\mpv.net.exe",
            r"C:\path\MPV.NET.EXE",
            "mpvnet.exe",
        ];
        for raw in mpv_cases {
            let path = PathBuf::from(raw);
            assert_eq!(branch(&path), PlayerKind::MpvNet, "{raw}");
            assert!(is_mpv_net(&path));
        }

        let external_cases = [
            r"C:\Program Files\VideoLAN\VLC\vlc.exe",
            r"C:\path\mpv.exe",
            r"C:\path\PotPlayerMini64.exe",
            "",
        ];
        for raw in external_cases {
            let path = PathBuf::from(raw);
            assert_eq!(branch(&path), PlayerKind::External, "{raw}");
            assert!(!is_mpv_net(&path));
        }
    }

    /// `build_external_command` produit une commande dont :
    ///
    ///   * le programme est exactement `player_path` ;
    ///   * **un seul argument** est passé : le chemin du fichier
    ///     média ;
    ///   * **aucun** `--start=…` ni `--script-opts=…` n'apparaît dans
    ///     les arguments — invariant central de la tâche 10.4 (ces
    ///     options sont propres à mpv et casseraient un autre lecteur).
    #[test]
    fn build_external_command_passes_only_file_path_arg() {
        let player = PathBuf::from(r"C:\Program Files\VideoLAN\VLC\vlc.exe");
        let file = PathBuf::from(r"D:\Media\Films\Inception (2010).mkv");

        let cmd = build_external_command(&player, &file);
        let std_cmd = cmd.as_std();

        assert_eq!(std_cmd.get_program(), player.as_os_str());

        let args: Vec<&std::ffi::OsStr> = std_cmd.get_args().collect();
        assert_eq!(
            args.len(),
            1,
            "build_external_command doit produire exactement 1 arg, vu : {args:?}"
        );
        assert_eq!(args[0], file.as_os_str());

        for a in &args {
            let s = a.to_string_lossy();
            assert!(
                !s.starts_with("--start"),
                "argument inattendu `--start…` pour un lecteur tiers : {s}"
            );
            assert!(
                !s.starts_with("--script-opts"),
                "argument inattendu `--script-opts…` pour un lecteur tiers : {s}"
            );
        }
    }

    /// Sur un chemin de lecteur tiers connu, la décision pure
    /// [`branch`] retourne `External` — sans IO, sans spawn. C'est la
    /// brique testable qui permet de valider la logique de routage de
    /// [`launch_player`] sans dépendre d'un binaire réel ni de Tauri.
    #[test]
    fn branch_on_third_party_player_returns_external() {
        let vlc = PathBuf::from(r"C:\Program Files\VideoLAN\VLC\vlc.exe");
        assert_eq!(branch(&vlc), PlayerKind::External);

        let pot = PathBuf::from(r"C:\path\PotPlayerMini64.exe");
        assert_eq!(branch(&pot), PlayerKind::External);

        // Cas piège : mpv « stock » (pas la variante .NET) doit aussi
        // tomber en External — le script Lua n'est installé que pour
        // mpv.net, et `--script-opts=kyro-pipe=…` côté mpv stock ne
        // déclencherait jamais l'écriture sur le pipe.
        let mpv_stock = PathBuf::from(r"C:\path\mpv.exe");
        assert_eq!(branch(&mpv_stock), PlayerKind::External);
    }

    /// Sur un chemin mpv.net, `branch` retourne `MpvNet` — symétrique
    /// du test précédent, garantit que la branche IPC reste accessible.
    #[test]
    fn branch_on_mpv_net_path_returns_mpv_net() {
        let path = PathBuf::from(r"C:\Program Files\mpv.net\mpv.net.exe");
        assert_eq!(branch(&path), PlayerKind::MpvNet);
    }

    /// La sortie de `launch_player` sur la branche External n'embarque
    /// **pas** de [`PlaybackSession`] — propriété structurelle vérifiée
    /// statiquement par le pattern matching exhaustif. Si quelqu'un
    /// ajoute un champ `session` à `LaunchOutcome::External`, ce test
    /// deviendra rouge à la compilation et signalera le bris de
    /// contrat avec la tâche 14.x (badge UI).
    #[test]
    fn launch_outcome_external_has_no_ipc_session_field() {
        fn _shape_check(o: LaunchOutcome) {
            match o {
                LaunchOutcome::External { child: _ } => {
                    // OK : aucun champ `session` n'est destructurable.
                }
                LaunchOutcome::MpvNet {
                    session: _,
                    child: _,
                } => {
                    // OK : la branche MpvNet, elle, embarque la session.
                }
            }
        }
        // Le simple fait que cette fonction compile prouve la propriété.
        let _ = _shape_check;
    }

    /// Vérifie la chaîne complète `launch_player` sur la branche
    /// `External` en utilisant un binaire toujours présent sur Windows
    /// (`cmd.exe`) avec un argument inoffensif (`/C exit 0`) pour ne
    /// pas ouvrir de fenêtre persistante.
    ///
    /// Propriété testée : `launch_player(non_mpv, …)` retourne
    /// **toujours** `LaunchOutcome::External`, jamais `MpvNet`. C'est
    /// l'invariant central du requirement 10.5.
    #[cfg(windows)]
    #[tokio::test]
    async fn launch_player_external_path_uses_external_outcome() {
        // `cmd.exe` est toujours présent sur Windows ; on le résout via
        // %SystemRoot%, qui est garanti côté processus de test.
        let system_root = std::env::var_os("SystemRoot")
            .or_else(|| std::env::var_os("WINDIR"))
            .expect("SystemRoot/WINDIR doit être défini sur Windows");
        let cmd_exe = PathBuf::from(system_root).join("System32").join("cmd.exe");
        assert!(
            cmd_exe.is_file(),
            "cmd.exe introuvable à {} — environnement Windows inattendu",
            cmd_exe.display()
        );

        // Argument « média » qui transforme l'invocation en un no-op
        // immédiat : `cmd /C exit 0`.
        let dummy_file = PathBuf::from("/C exit 0");

        let outcome = launch_player(&cmd_exe, &dummy_file, 0)
            .await
            .expect("launch_player doit réussir avec cmd.exe");

        match outcome {
            LaunchOutcome::External { mut child } => {
                // Pas de session IPC à valider — c'est précisément
                // l'invariant : aucune `PlaybackSession` n'est exposée
                // sur cette branche.
                let _ = child.wait().await;
            }
            LaunchOutcome::MpvNet { .. } => {
                panic!(
                    "launch_player avec cmd.exe (≠ mpv.net) doit retourner \
                     LaunchOutcome::External, jamais MpvNet"
                );
            }
        }
    }

    // -----------------------------------------------------------------
    // Tâche 10.5 — validate_external_player_path.
    // -----------------------------------------------------------------

    /// Sanity check : la constante d'événement Tauri est bien la chaîne
    /// documentée dans le design (requirement 10.3 / 10.4). Toute
    /// modification du nom doit être synchronisée avec le frontend.
    #[test]
    fn event_external_player_missing_is_canonical() {
        assert_eq!(EVENT_EXTERNAL_PLAYER_MISSING, "external-player-missing");
    }

    /// Cas nominal : un fichier régulier avec extension `.exe` est
    /// accepté (requirement 10.3).
    #[test]
    fn validate_accepts_regular_exe_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path: PathBuf = dir.path().join("mpv.net.exe");
        std::fs::write(&path, b"MZ").expect("write fake exe");

        validate_external_player_path(&path).expect("doit accepter un .exe valide");
    }

    /// Extension `.bat` (ou autre que `.exe`) → refus avec
    /// `KyroError::Validation` qui cite le chemin (requirement 10.3).
    #[test]
    fn validate_rejects_non_exe_extension() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("player.bat");
        std::fs::write(&path, b"@echo off").expect("write");

        let err = validate_external_player_path(&path)
            .expect_err("doit refuser une extension différente de .exe");
        match err {
            KyroError::Validation(msg) => {
                assert!(
                    msg.contains("player.bat"),
                    "le message doit citer le chemin fautif, got: {msg}"
                );
                assert!(
                    msg.contains(".exe"),
                    "le message doit mentionner l'extension attendue, got: {msg}"
                );
            }
            other => panic!("variante d'erreur attendue: Validation, got: {other:?}"),
        }
    }

    /// Aucune extension du tout → refus avec `Validation`.
    #[test]
    fn validate_rejects_file_without_extension() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("player");
        std::fs::write(&path, b"x").expect("write");

        let err = validate_external_player_path(&path)
            .expect_err("doit refuser l'absence d'extension");
        assert!(matches!(err, KyroError::Validation(_)));
    }

    /// Chemin inexistant → refus avec `Validation` (requirement 10.4 :
    /// le binaire enregistré peut avoir été déplacé / désinstallé).
    #[test]
    fn validate_rejects_missing_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("does_not_exist.exe");
        // path n'est volontairement pas créé.

        let err = validate_external_player_path(&path)
            .expect_err("doit refuser un chemin inexistant");
        match err {
            KyroError::Validation(msg) => assert!(
                msg.contains("does_not_exist.exe"),
                "message doit citer le chemin: {msg}"
            ),
            other => panic!("variante attendue: Validation, got: {other:?}"),
        }
    }

    /// Un dossier (même avec un nom se terminant par `.exe`) → refus,
    /// car `is_file()` est faux. C'est la garantie que la validation
    /// n'est pas réductible à une simple vérification d'extension.
    #[test]
    fn validate_rejects_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Crée un dossier dont le nom ressemble à un exe pour piéger
        // une éventuelle implémentation qui ne vérifierait que
        // l'extension.
        let path = dir.path().join("looks_like.exe");
        std::fs::create_dir(&path).expect("create_dir");

        let err = validate_external_player_path(&path)
            .expect_err("doit refuser un dossier");
        assert!(matches!(err, KyroError::Validation(_)));
    }

    /// Insensibilité à la casse de l'extension : `.EXE`, `.Exe`, `.eXe`
    /// sont acceptés au même titre que `.exe` — Windows ne distingue pas
    /// la casse sur les noms de fichiers.
    #[test]
    fn validate_extension_check_is_case_insensitive() {
        let dir = tempfile::tempdir().expect("tempdir");

        for name in ["app.EXE", "app.Exe", "app.eXe"] {
            let path = dir.path().join(name);
            std::fs::write(&path, b"MZ").expect("write");
            validate_external_player_path(&path)
                .unwrap_or_else(|e| panic!("{name} doit être accepté, got: {e:?}"));
            std::fs::remove_file(&path).ok();
        }
    }

    // -----------------------------------------------------------------
    // Tâche 10.6 — run_ipc_session : persistance progression + position finale.
    // -----------------------------------------------------------------

    use crate::domain::types::{MediaItemId, MediaType};
    use crate::infra::repo::UpsertMediaInput;
    use sqlx::sqlite::{
        SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
    };
    use sqlx::SqlitePool;
    use std::path::Path as StdPath;
    use tempfile::TempDir;

    /// Helper synchrone : exécute un futur sur un runtime
    /// `current_thread` dédié — même pattern que `infra::repo::tests`.
    fn block_on<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime tokio dédié au test")
            .block_on(fut)
    }

    /// Reproduit `infra::repo::tests::fresh_pool` localement : pool
    /// SQLite sur fichier temporaire avec migrations appliquées et les
    /// pragmas de production (`foreign_keys = ON`, journal WAL).
    async fn fresh_pool(db_dir: &StdPath) -> SqlitePool {
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

        pool
    }

    /// Insère un Film (Media_Item) avec une durée connue pour que
    /// `compute_status` produise des transitions observables.
    async fn insert_film_with_duration(
        repo: &Repo,
        path: &str,
        duration_seconds: i64,
    ) -> MediaItemId {
        let input = UpsertMediaInput {
            folder_id: None,
            media_type: MediaType::Film,
            file_path: path.to_string(),
            size_bytes: 1_000_000,
            title: "Test".to_string(),
            year: Some(2020),
            quality: None,
            duration_seconds: Some(duration_seconds),
            video_codec: None,
            width: None,
            height: None,
            numbering_unknown: false,
        };
        repo.upsert_media_item(&input).await.unwrap()
    }

    /// Lit la valeur courante de `watch_progress_seconds` pour un
    /// Media_Item.
    async fn read_watch_progress(pool: &SqlitePool, id: MediaItemId) -> i64 {
        sqlx::query_scalar::<_, i64>(
            "SELECT watch_progress_seconds FROM media_item WHERE id = ?",
        )
        .bind(id.get())
        .fetch_one(pool)
        .await
        .expect("lecture watch_progress_seconds")
    }

    /// Forge un `Progress` pour un test, avec `duration` arbitraire et
    /// un identifiant de session statique (le champ n'est pas inspecté
    /// par `run_ipc_session`).
    fn progress(position: f64) -> PipeMessage {
        PipeMessage::Progress {
            position,
            duration: 1000.0,
            session: "sid".to_string(),
        }
    }

    fn paused(position: f64) -> PipeMessage {
        PipeMessage::Paused {
            position,
            session: "sid".to_string(),
        }
    }

    fn closed(position: f64) -> PipeMessage {
        PipeMessage::Closed {
            position,
            session: "sid".to_string(),
        }
    }

    /// Cas nominal : séquence `Progress → Paused → Closed`. Le receiver
    /// fournit les trois messages, puis le sender est drop. Chaque
    /// message produit un `set_watch_progress` ; la position finale
    /// écrite est celle du `Closed` (requirement 9.4, property 20).
    #[test]
    fn run_ipc_session_persists_each_message_and_final_close() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());
            let id = insert_film_with_duration(&repo, "C:/movies/A.mkv", 1000).await;

            let (tx, rx) = mpsc::channel::<PipeMessage>(8);
            tx.send(progress(100.0)).await.unwrap();
            tx.send(paused(250.0)).await.unwrap();
            tx.send(closed(420.0)).await.unwrap();
            drop(tx);

            let last_written = run_ipc_session(&repo, ProgressTarget::Media(id), rx)
                .await
                .expect("session IPC doit terminer sans erreur");

            assert_eq!(last_written, Some(420));
            assert_eq!(read_watch_progress(&pool, id).await, 420);
        });
    }

    /// Property 21 (requirement 9.5) : si **aucun** message n'est
    /// reçu pendant la session, `watch_progress_seconds` doit rester
    /// strictement inchangé.
    #[test]
    fn run_ipc_session_preserves_progress_when_no_message_received() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());
            let id = insert_film_with_duration(&repo, "C:/movies/B.mkv", 1000).await;

            // Pré-positionne `watch_progress_seconds` à 123 s.
            repo.set_watch_progress(ProgressTarget::Media(id), 123)
                .await
                .unwrap();
            assert_eq!(read_watch_progress(&pool, id).await, 123);

            // Canal créé, sender drop tout de suite : aucune ligne ne
            // transitera jamais.
            let (tx, rx) = mpsc::channel::<PipeMessage>(8);
            drop(tx);

            let last_written = run_ipc_session(&repo, ProgressTarget::Media(id), rx)
                .await
                .expect("session vide doit terminer Ok");

            assert_eq!(last_written, None);
            // Garantie centrale du requirement 9.5 : la valeur est
            // strictement préservée.
            assert_eq!(read_watch_progress(&pool, id).await, 123);
        });
    }

    /// Séquence sans `Closed` final (le client mpv a crash, la fenêtre
    /// a été tuée, …) : la dernière position écrite doit être celle du
    /// dernier message reçu.
    #[test]
    fn run_ipc_session_records_last_progress_when_sender_drops_without_close() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool.clone());
            let id = insert_film_with_duration(&repo, "C:/movies/C.mkv", 1000).await;

            let (tx, rx) = mpsc::channel::<PipeMessage>(8);
            tx.send(progress(50.0)).await.unwrap();
            tx.send(progress(180.0)).await.unwrap();
            // Pas de `Closed` : drop du sender = fermeture sale.
            drop(tx);

            let last_written = run_ipc_session(&repo, ProgressTarget::Media(id), rx)
                .await
                .expect("session sans close doit terminer Ok");

            assert_eq!(last_written, Some(180));
            assert_eq!(read_watch_progress(&pool, id).await, 180);
        });
    }

    /// Vérifie que `clamp_position_to_u32` neutralise les positions
    /// pathologiques (négative, NaN, infinity, débordement) sans
    /// jamais paniquer.
    #[test]
    fn clamp_position_to_u32_handles_pathological_floats() {
        assert_eq!(clamp_position_to_u32(0.0), 0);
        assert_eq!(clamp_position_to_u32(-1.0), 0);
        assert_eq!(clamp_position_to_u32(f64::NAN), 0);
        assert_eq!(clamp_position_to_u32(f64::NEG_INFINITY), 0);
        assert_eq!(clamp_position_to_u32(f64::INFINITY), u32::MAX);
        assert_eq!(clamp_position_to_u32(42.7), 42);
        assert_eq!(clamp_position_to_u32(f64::from(u32::MAX) + 100.0), u32::MAX);
    }

    /// La constante d'événement Tauri est figée : le frontend
    /// (`window.listen("playback-ended", ...)`) en dépend par son nom
    /// littéral.
    #[test]
    fn event_playback_ended_constant_value() {
        assert_eq!(EVENT_PLAYBACK_ENDED, "playback-ended");
    }

    /// Un `Closed` sur une cible inexistante propage l'erreur (garantie
    /// de cohérence finale, requirement 9.4).
    #[test]
    fn run_ipc_session_propagates_error_on_final_close() {
        block_on(async {
            let db_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db_dir.path()).await;
            let repo = Repo::new(pool);

            let bogus = ProgressTarget::Media(MediaItemId(99_999));

            let (tx, rx) = mpsc::channel::<PipeMessage>(8);
            tx.send(progress(10.0)).await.unwrap();
            tx.send(closed(20.0)).await.unwrap();
            drop(tx);

            let result = run_ipc_session(&repo, bogus, rx).await;
            assert!(
                result.is_err(),
                "le `Closed` final doit propager l'erreur (got: {result:?})"
            );
        });
    }

    // -----------------------------------------------------------------
    // Tâche 10.7 — Property 19 : Lancement avec position de reprise.
    //
    // Feature: kyro, Property 19
    // Validates: Requirements 9.1, 12.4
    //
    // Pour tout `start_seconds: u32` (généré dans `0..=86400`, soit
    // 24 h max — borne pratique correspondant à la durée réaliste
    // maximale d'un Media_Item dans Kyro), la commande produite par
    // [`build_mpv_command`] doit contenir **exactement un** argument
    // de la forme `--start=<start_seconds>`. La commande est inspectée
    // via `Command::as_std().get_args()` sans déclencher de spawn :
    // c'est le « Command mock » prescrit par la tâche.
    //
    // Cette propriété verrouille le contrat avec mpv.net pour la
    // reprise exacte (requirement 9.1) tel qu'il est propagé par la
    // commande Tauri `play_media` (requirement 12.4).
    // -----------------------------------------------------------------

    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 100,
            .. ProptestConfig::default()
        })]

        /// **Property 19 — Lancement avec position de reprise.**
        ///
        /// Pour tout `start_seconds` arbitraire (un `Media_Item` peut
        /// porter n'importe quelle valeur de `watch_progress_seconds`,
        /// bornée ici à 24 h pour rester dans le domaine réaliste),
        /// la `Command` construite pour mpv.net doit contenir
        /// **un et un seul** argument `--start=<start_seconds>`.
        ///
        /// **Validates: Requirements 9.1, 12.4**
        #[test]
        fn property_19_build_mpv_command_contains_start_arg(
            start_seconds in 0u32..=86_400u32,
        ) {
            let player = PathBuf::from(r"C:\path\mpv.net.exe");
            let file = PathBuf::from(r"D:\Media\Films\sample.mkv");
            let session = new_session();

            let cmd = build_mpv_command(&player, &file, start_seconds, &session);
            let std_cmd = cmd.as_std();

            let expected = format!("--start={start_seconds}");
            let expected_os: &std::ffi::OsStr = std::ffi::OsStr::new(&expected);

            // Compte exact d'occurrences du `--start=…` attendu : doit
            // être strictement égal à 1, jamais 0 (l'argument doit être
            // présent) ni > 1 (jamais dupliqué — mpv.net se plaindrait).
            let matches: usize = std_cmd
                .get_args()
                .filter(|a| *a == expected_os)
                .count();
            prop_assert_eq!(
                matches, 1,
                "la commande doit contenir exactement un `{}`, vu {} occurrence(s)",
                expected, matches
            );

            // Garde-fou : aucun autre argument ne doit commencer par
            // `--start=` avec une valeur différente — interdit toute
            // forme de doublon « silencieux » (`--start=0 --start=42`).
            let stray: Vec<String> = std_cmd
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .filter(|s| s.starts_with("--start=") && s != &expected)
                .collect();
            prop_assert!(
                stray.is_empty(),
                "argument(s) `--start=…` parasite(s) détecté(s) : {:?}",
                stray
            );
        }
    }

    // -----------------------------------------------------------------
    // Tâche 10.8 — Property 20 : Mise à jour de Watch_Progress sur
    // message IPC.
    //
    // Feature: kyro, Property 20
    // Validates: Requirements 9.3, 9.4
    //
    // *For any* séquence arbitraire de messages NDJSON `{type,
    // position}` envoyée sur le canal IPC d'une session associée à un
    // `Media_Item i`, après la fermeture de la session
    // [`run_ipc_session`] :
    //
    //   * `i.watch_progress_seconds` est égal à la **dernière position
    //     effectivement consommée** par la boucle, c'est-à-dire la
    //     position du dernier message de la séquence tronquée au
    //     premier [`PipeMessage::Closed`] inclus (la boucle sort sur
    //     `Closed` et n'aspire pas les messages suivants — propriété
    //     architecturale du design tâche 10.6) ;
    //   * la valeur retournée par [`run_ipc_session`] (`Option<u32>`)
    //     coïncide avec la valeur lue en base — write-then-read
    //     cohérent (requirement 9.4 + 17.2).
    //
    // Le « mock pipe » est ici une [`tokio::sync::mpsc::channel`] :
    // c'est exactement la primitive que [`crate::infra::ipc_pipe::listen`]
    // expose à [`run_ipc_session`] en production (le serveur named
    // pipe parse le NDJSON et pousse les `PipeMessage` typés sur ce
    // channel). Tester contre le channel directement isole la
    // propriété de l'IO Windows et garde le run-time de proptest dans
    // le budget (50 cas × création d'un pool SQLite).
    // -----------------------------------------------------------------

    /// Énumération des types de messages NDJSON tirés par le générateur
    /// de la propriété 20. Mappée vers une variante [`PipeMessage`] via
    /// [`p20_make_message`] : `Resumed` est inliné ici car il n'a pas
    /// de helper `resumed()` dans le module de tests.
    #[derive(Debug, Clone, Copy)]
    enum P20MessageType {
        Progress,
        Paused,
        Resumed,
        Closed,
    }

    /// Convertit un couple `(kind, position)` tiré par le générateur en
    /// un [`PipeMessage`] concret. La `position` est élargie en `f64`
    /// (le format NDJSON envoie des floats) ; le champ `session` est
    /// statique car [`run_ipc_session`] ne l'inspecte pas.
    fn p20_make_message(kind: P20MessageType, position: u32) -> PipeMessage {
        let pos = position as f64;
        match kind {
            P20MessageType::Progress => progress(pos),
            P20MessageType::Paused => paused(pos),
            P20MessageType::Resumed => PipeMessage::Resumed {
                position: pos,
                session: "sid".to_string(),
            },
            P20MessageType::Closed => closed(pos),
        }
    }

    /// Stratégie proptest émettant un `P20MessageType` parmi les quatre
    /// variantes du protocole, équiprobables.
    fn p20_message_kind() -> impl Strategy<Value = P20MessageType> {
        prop_oneof![
            Just(P20MessageType::Progress),
            Just(P20MessageType::Paused),
            Just(P20MessageType::Resumed),
            Just(P20MessageType::Closed),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            // 50 cas comme prescrit par la tâche 10.8 : chaque cas
            // ouvre un pool SQLite (TempDir + migrations) puis pousse
            // jusqu'à 20 messages dans le channel — coût non
            // négligeable, mais 50 cas suffisent largement à explorer
            // l'espace combinatoire (4 types × 20 positions)^len.
            cases: 50,
            .. ProptestConfig::default()
        })]

        /// **Property 20 — Mise à jour de Watch_Progress sur message
        /// IPC.**
        ///
        /// Pour toute séquence non vide de messages
        /// `(MessageType, position)` poussée sur le channel IPC d'une
        /// session liée à un `Media_Item` de durée 24 h :
        ///
        ///   * la boucle [`run_ipc_session`] s'arrête au premier
        ///     `Closed` (s'il existe) ou consomme la séquence entière
        ///     sinon ; la **dernière position consommée** est la valeur
        ///     observable dans `media_item.watch_progress_seconds`
        ///     après terminaison ;
        ///   * la valeur retournée par `run_ipc_session` (`Some(pos)`)
        ///     est strictement égale à la valeur persistée en base.
        ///
        /// Tag: Feature: kyro, Property 20.
        ///
        /// **Validates: Requirements 9.3, 9.4**
        #[test]
        fn property_20_watch_progress_matches_last_consumed_position(
            messages in proptest::collection::vec(
                (p20_message_kind(), 0u32..=86_400u32),
                1..=20,
            ),
        ) {
            // Tout le travail asynchrone (création DB, insertion,
            // exécution de la session IPC, lecture de la position
            // persistée) est confiné dans `block_on` ; les
            // `prop_assert_*` sont effectués en dehors du futur pour
            // respecter le `Result<(), TestCaseError>` synthétisé par
            // `proptest!`.
            let (expected, last_written, persisted) = block_on(async {
                let db_dir = TempDir::new().expect("tempdir DB");
                let pool = fresh_pool(db_dir.path()).await;
                let repo = Repo::new(pool.clone());
                // Durée = 86 400 s (24 h) pour que `compute_status`
                // puisse classer toute position 0..=86400 sans
                // saturer.
                let id = insert_film_with_duration(
                    &repo,
                    "C:/movies/p20.mkv",
                    86_400,
                )
                .await;

                // Calcul de l'oracle : la boucle `run_ipc_session`
                // s'arrête au premier `Closed` (inclus). On reproduit
                // ici la sémantique exacte de la boucle, y compris le
                // garde-fou anti-régression sur le `Closed` : si le
                // closed.position est plus de 5 s en dessous du max
                // observé pendant la session, c'est le max qui est
                // persisté à la place de la valeur du closed (cf.
                // commentaire dans `run_ipc_session`).
                let expected_position: u32 = {
                    let mut last: u32 = 0;
                    let mut max_seen: u32 = 0;
                    let mut closed_seen = false;
                    for (kind, pos) in messages.iter() {
                        match kind {
                            P20MessageType::Closed => {
                                closed_seen = true;
                                let effective = if max_seen > 0 && *pos + 5 < max_seen {
                                    max_seen
                                } else if *pos == 0 && last > 0 {
                                    // Garde-fou Closed à 0 après non-nul.
                                    last
                                } else {
                                    *pos
                                };
                                last = effective;
                                break;
                            }
                            _ => {
                                last = *pos;
                                if *pos > max_seen {
                                    max_seen = *pos;
                                }
                            }
                        }
                    }
                    let _ = closed_seen;
                    last
                };

                // Channel dimensionné exactement à la taille de la
                // séquence pour que tous les `tx.send().await`
                // réussissent immédiatement, sans blocage côté
                // producteur.
                let (tx, rx) = mpsc::channel::<PipeMessage>(messages.len());
                for (kind, pos) in &messages {
                    tx.send(p20_make_message(*kind, *pos))
                        .await
                        .expect("envoi sur le channel IPC mock");
                }
                // Drop du sender → le receiver verra `None` après
                // épuisement du buffer (équivalent EOF du pipe réel).
                drop(tx);

                let last_written = run_ipc_session(
                    &repo,
                    ProgressTarget::Media(id),
                    rx,
                )
                .await
                .expect("run_ipc_session doit terminer Ok");

                let persisted = read_watch_progress(&pool, id).await as u32;
                (expected_position, last_written, persisted)
            });

            // Cœur de la propriété : la valeur retournée par
            // `run_ipc_session` doit être exactement la dernière
            // position consommée par la boucle, et la valeur
            // persistée doit y être strictement égale (cohérence
            // write-then-read, requirement 9.4 + 17.2).
            prop_assert_eq!(
                last_written,
                Some(expected),
                "run_ipc_session doit retourner la dernière position consommée"
            );
            prop_assert_eq!(
                persisted,
                expected,
                "watch_progress_seconds en base doit valoir la dernière position consommée"
            );
        }
    }

    // -----------------------------------------------------------------
    // Property 23 — Pas d'IPC pour lecteur autre que mpv.net.
    //
    // Feature: kyro, Property 23
    //
    // *For any* chemin External_Player aléatoire ne se résolvant pas en
    // `mpv.net.exe` (ni en l'alias `mpvnet.exe`), la couche
    // `Kyro_Player_Bridge` doit :
    //
    //   1. router le lancement vers la branche [`PlayerKind::External`]
    //      (jamais `MpvNet`) — c'est l'invariant testé via [`branch`] ;
    //   2. construire une `Command` minimaliste avec **un seul argument**
    //      (le chemin du fichier média), sans `--start=…` ni
    //      `--script-opts=…` — c'est l'invariant testé via
    //      [`build_external_command`].
    //
    // L'absence de `--script-opts=kyro-pipe=…` dans les arguments est la
    // garantie programmatique qu'aucun [`PlaybackSession`] (donc aucun
    // `NamedPipeServer`) ne pourra être attaché à cette session : la
    // façade [`launch_player`] ne crée le pipe qu'à l'intérieur de la
    // branche `MpvNet` (cf. corps de la fonction). Vérifier la commande
    // par inspection (« Command mock » sans spawn) suffit donc à
    // attester l'absence d'IPC, et l'item courant doit être annoté
    // « reprise non garantie » côté UI via l'événement
    // [`EVENT_EXTERNAL_PLAYER_NON_MPV`] (contrat lié à la variante
    // `LaunchOutcome::External`).
    //
    // Validates: Requirements 10.5
    // -----------------------------------------------------------------

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 100,
            .. ProptestConfig::default()
        })]

        /// **Property 23 — Pas d'IPC pour lecteur autre que mpv.net.**
        ///
        /// Pour tout chemin `<dir>\<stem>.exe` où `<stem>` est composé
        /// de 1 à 10 lettres ASCII minuscules **et** ne vaut pas
        /// `mpvnet` (alias mpv.net) — la chaîne `mpv.net` n'est jamais
        /// générée puisque le générateur n'émet pas de `.` dans le
        /// stem :
        ///
        ///   * [`branch`] retourne `PlayerKind::External` (jamais
        ///     `MpvNet`) ;
        ///   * [`build_external_command`] produit une `Command` dont
        ///     le programme est `player_path` et dont **l'unique
        ///     argument** est `file_path` — pas de `--start=…`, pas
        ///     de `--script-opts=…`, donc **aucun** named pipe ne peut
        ///     être attaché à cette session.
        ///
        /// **Validates: Requirements 10.5**
        #[test]
        fn property_23_no_ipc_for_non_mpv_player(
            stem in "[a-z]{1,10}",
            file_stem in "[a-z]{1,15}",
        ) {
            // Exclut explicitement l'alias `mpvnet.exe` reconnu par
            // [`is_mpv_net`]. (Le nom canonique `mpv.net.exe` est déjà
            // hors du codomaine du générateur — `[a-z]{1,10}` n'émet
            // jamais de `.`.)
            prop_assume!(stem != "mpvnet");

            let player = PathBuf::from(format!(r"C:\Players\{stem}.exe"));
            let file = PathBuf::from(format!(r"D:\Media\{file_stem}.mkv"));

            // Invariant 1 : routage vers la branche External, jamais
            // MpvNet — la garantie « pas d'IPC » repose sur ce choix
            // de branche.
            let kind = branch(&player);
            prop_assert_eq!(
                kind,
                PlayerKind::External,
                "branch({}) doit retourner External, vu {:?}",
                player.display(),
                kind
            );

            // Invariant 2 : la commande externe contient exactement 1
            // argument — le chemin du fichier média — et aucun
            // argument propre à mpv (`--start=…`, `--script-opts=…`).
            // L'absence de `--script-opts=kyro-pipe=…` est la preuve
            // structurelle qu'aucun NamedPipeServer ne sera attaché à
            // cette session : `launch_player` ne crée la session IPC
            // qu'à l'intérieur de la branche MpvNet.
            let cmd = build_external_command(&player, &file);
            let std_cmd = cmd.as_std();

            prop_assert_eq!(
                std_cmd.get_program(),
                player.as_os_str(),
                "le programme de la Command doit être le chemin du lecteur tiers"
            );

            let args: Vec<String> = std_cmd
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            prop_assert_eq!(
                args.len(),
                1,
                "la Command externe doit contenir exactement 1 argument (file_path), vu {:?}",
                args
            );
            prop_assert_eq!(
                std_cmd.get_args().next().expect("1 arg"),
                file.as_os_str(),
                "l'unique argument doit être le chemin du fichier média"
            );

            // Garde-fous explicites : aucun argument propre à mpv ne
            // doit transiter — leur présence ouvrirait la porte à un
            // attachement IPC ou à un mauvais routage côté lecteur
            // tiers (qui interpréterait `--start=…` comme un fichier).
            for a in &args {
                prop_assert!(
                    !a.starts_with("--start"),
                    "argument `--start…` interdit pour un lecteur non-mpv : {}",
                    a
                );
                prop_assert!(
                    !a.starts_with("--script-opts"),
                    "argument `--script-opts…` interdit pour un lecteur non-mpv : {}",
                    a
                );
                prop_assert!(
                    !a.contains("kyro-pipe"),
                    "fragment `kyro-pipe` interdit (signerait une fuite d'IPC) : {}",
                    a
                );
            }
        }
    }

    // -----------------------------------------------------------------
    // Tâche 10.9 — Property 21 : Préservation en l'absence de message IPC.
    //
    // Feature: kyro, Property 21
    // Validates: Requirements 9.5
    //
    // Pour tout `initial_progress: u32` (borné à 24 h, soit 86_400 s,
    // pour rester dans le domaine réaliste d'un Media_Item Kyro), si
    // une session IPC s'ouvre et se ferme sans qu'aucun message ne
    // transite (sender drop immédiat → receiver vide jusqu'à EOF),
    // alors `watch_progress_seconds` doit rester strictement égal à
    // `initial_progress` après l'appel à [`run_ipc_session`].
    //
    // C'est l'invariant central du requirement 9.5 : « si aucun message
    // n'est reçu pendant la session, ne pas modifier
    // `watch_progress_seconds` ». Un test unitaire le couvre déjà
    // (`run_ipc_session_preserves_progress_when_no_message_received`)
    // sur la valeur arbitraire 123 ; cette propriété généralise la
    // garantie sur l'ensemble du domaine.
    //
    // Chaque cas crée son propre pool SQLite + applique les migrations
    // dans un `TempDir`. C'est volontairement coûteux (cf. tâches 7.x
    // qui utilisent le même pattern) mais reste tractable à 50 cas.
    // -----------------------------------------------------------------

    proptest! {
        #![proptest_config(ProptestConfig {
            // 50 cas comme prescrit par la tâche 10.9 : compromis entre
            // couverture du domaine `0..=86_400` et coût du bootstrap
            // SQLite (création fichier + migrations) répété par cas.
            cases: 50,
            .. ProptestConfig::default()
        })]

        /// **Property 21 — Préservation de `watch_progress_seconds` en
        /// l'absence de message IPC.**
        ///
        /// Une session ouverte sans qu'aucun message NDJSON ne soit
        /// reçu avant la fermeture du pipe (sender drop immédiat) doit
        /// laisser `watch_progress_seconds` strictement inchangé en
        /// base, quelle que soit la valeur initiale.
        ///
        /// Tag: Feature: kyro, Property 21.
        ///
        /// **Validates: Requirements 9.5**
        #[test]
        fn property_21_preserves_progress_when_no_ipc_message(
            initial_progress in 0u32..=86_400u32,
        ) {
            // Le futur retourne le `watch_progress_seconds` final lu en
            // base (en `i64` comme stocké par SQLite) ainsi que la
            // valeur retournée par `run_ipc_session` (qui doit être
            // `None` puisque aucune écriture n'a eu lieu). Les
            // `prop_assert_*` sont effectués hors du futur pour
            // respecter le `Result<(), TestCaseError>` synthétisé par
            // `proptest!`.
            let (final_progress, last_written) = block_on(async {
                let db_dir = TempDir::new().expect("tempdir DB");
                let pool = fresh_pool(db_dir.path()).await;
                let repo = Repo::new(pool.clone());

                // Insère un Media_Item avec une durée connue (les
                // valeurs précises n'ont pas d'importance ici : le
                // test valide la non-écriture).
                let id = insert_film_with_duration(
                    &repo,
                    "C:/movies/p21_no_ipc.mkv",
                    1_000,
                )
                .await;

                // Pré-positionne `watch_progress_seconds = initial_progress`
                // via le chemin officiel `set_watch_progress` (qui
                // recalcule le statut et commit). C'est l'état de
                // départ que la propriété s'engage à préserver.
                repo.set_watch_progress(
                    ProgressTarget::Media(id),
                    initial_progress,
                )
                .await
                .expect("pré-positionnement watch_progress_seconds");

                // Canal créé puis sender drop immédiatement : aucun
                // `PipeMessage` ne sera jamais reçu, le receiver verra
                // `None` au premier `recv().await`.
                let (tx, rx) = mpsc::channel::<PipeMessage>(8);
                drop(tx);

                let last_written =
                    run_ipc_session(&repo, ProgressTarget::Media(id), rx)
                        .await
                        .expect("session IPC vide doit terminer Ok");

                let final_progress = read_watch_progress(&pool, id).await;
                (final_progress, last_written)
            });

            // La valeur en base est strictement préservée
            // (requirement 9.5).
            prop_assert_eq!(
                final_progress,
                initial_progress as i64,
                "P-21 : sans message IPC, watch_progress_seconds doit \
                 rester égal à la valeur initiale, mais a été modifié"
            );

            // Aucune écriture n'a eu lieu pendant la session : la
            // valeur retournée par `run_ipc_session` reflète
            // l'absence d'activité.
            prop_assert_eq!(
                last_written,
                None,
                "P-21 : `run_ipc_session` ne doit retourner aucune \
                 position écrite quand aucun message n'a transité"
            );
        }
    }

    // -----------------------------------------------------------------
    // Tâche 10.10 — Property 22 : Unicité du nom de pipe par session.
    //
    // Feature: kyro, Property 22
    // Validates: Requirements 9.6
    //
    // Pour tout `n` dans `2..=20`, `n` appels successifs à
    // [`new_session()`] produisent `n` `pipe_name` deux à deux
    // distincts. La garantie repose sur l'unicité statistique d'UUID
    // v4 (122 bits aléatoires) — aucune table d'identifiants n'est
    // partagée entre sessions, l'unicité est donc une propriété pure
    // du générateur. Cette propriété verrouille le requirement 9.6 :
    // chaque `play_media` ouvre un named pipe Windows dont le nom est
    // suffisamment unique pour qu'aucune collision avec une session
    // précédente (encore en cours de fermeture, par exemple) ne
    // déclenche un `ERROR_PIPE_BUSY` côté `NamedPipeServer`.
    // -----------------------------------------------------------------

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 100,
            .. ProptestConfig::default()
        })]

        /// **Property 22 — Unicité du nom de pipe par session.**
        ///
        /// Pour tout `n` dans `2..=20`, `n` appels consécutifs à
        /// [`new_session()`] produisent `n` `pipe_name` distincts.
        ///
        /// Tag: Feature: kyro, Property 22.
        ///
        /// **Validates: Requirements 9.6**
        #[test]
        fn property_22_pipe_names_are_unique_per_session(
            n in 2usize..=20usize,
        ) {
            use std::collections::HashSet;

            let pipe_names: Vec<String> = (0..n)
                .map(|_| new_session().pipe_name)
                .collect();

            // Sanity : on a bien généré `n` noms.
            prop_assert_eq!(
                pipe_names.len(),
                n,
                "P-22 : `new_session()` doit produire un `pipe_name` \
                 par appel"
            );

            // Cœur de la propriété : tous les noms sont deux à deux
            // distincts. On collecte dans un `HashSet` — sa taille
            // doit valoir exactement `n`, sinon au moins une collision
            // a eu lieu (cas catastrophique côté NamedPipeServer
            // Windows : `ERROR_PIPE_BUSY` au moment du `bind`).
            let unique: HashSet<&String> = pipe_names.iter().collect();
            prop_assert_eq!(
                unique.len(),
                n,
                "P-22 : `n` appels à `new_session()` doivent produire \
                 `n` `pipe_name` distincts (UUID v4), mais une \
                 collision a été détectée"
            );
        }
    }

    // -----------------------------------------------------------------
    // Tâche 10.12 — Property 24 : Validation du chemin External_Player.
    //
    // Feature: kyro, Property 24
    // Validates: Requirements 10.3
    //
    // *For any* chemin sélectionné comme External_Player, la
    // validation [`validate_external_player_path`] retourne `Ok(())`
    // si et seulement si le fichier référencé **existe sur disque**
    // **ET** son extension (insensible à la casse) vaut `exe`.
    //
    // Le générateur mélange via `prop_oneof!` trois catégories de
    // chemins, chacune matérialisée dans un `TempDir` éphémère pour
    // contrôler entièrement l'état du système de fichiers observé par
    // la fonction sous test :
    //
    //   1. **Valide** — un vrai fichier régulier dont l'extension est
    //      tirée parmi les variantes de casse acceptées
    //      (`.exe`, `.EXE`, `.Exe`, `.eXe`). Doit produire `Ok(())`.
    //   2. **Extension invalide** — un vrai fichier régulier dont
    //      l'extension est tirée parmi `.bat`, `.cmd`, `.txt`, `.dll`.
    //      Doit produire `Err(Validation)`.
    //   3. **Inexistant** — un chemin qui ne pointe sur rien (avec
    //      ou sans extension). Doit produire `Err(Validation)` quoi
    //      qu'il arrive (la condition « existe sur disque » est
    //      fausse).
    //
    // L'oracle est strictement la conjonction `exists_on_disk &&
    // ext_lc == "exe"` recalculée à partir de l'état réel du
    // `TempDir` : la propriété est auto-vérifiante et reste alignée
    // sur la sémantique de `validate_external_player_path` même pour
    // les cas exotiques (par exemple un chemin inexistant avec
    // extension `.exe` reste refusé car le fichier n'existe pas).
    // -----------------------------------------------------------------

    /// Catégorie de chemin générée par la propriété 24. Conserve les
    /// composants nécessaires à la matérialisation du fichier dans le
    /// `TempDir` du test sans figer le chemin absolu (qui dépend du
    /// `TempDir` créé à chaque cas).
    #[derive(Debug, Clone)]
    enum P24Scenario {
        /// Fichier régulier avec extension `.exe` (variantes de casse).
        ValidExe { stem: String, ext: String },
        /// Fichier régulier avec extension différente de `.exe`.
        InvalidExt { stem: String, ext: String },
        /// Chemin qui ne sera pas matérialisé sur disque.
        Nonexistent {
            stem: String,
            ext: Option<String>,
        },
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 100,
            .. ProptestConfig::default()
        })]

        /// **Property 24 — Validation du chemin External_Player.**
        ///
        /// `validate_external_player_path(p).is_ok()` ⇔
        /// (`p` existe sur disque comme fichier régulier ET son
        /// extension lowercase vaut `"exe"`).
        ///
        /// Tag: Feature: kyro, Property 24.
        ///
        /// **Validates: Requirements 10.3**
        #[test]
        fn property_24_external_player_path_validation(
            scenario in prop_oneof![
                // Catégorie 1 : fichier .exe valide (toutes variantes
                // de casse acceptées par Windows / la fonction sous
                // test).
                ("[a-z]{1,10}", prop::sample::select(vec![".exe", ".EXE", ".Exe", ".eXe"]))
                    .prop_map(|(stem, ext)| P24Scenario::ValidExe {
                        stem,
                        ext: ext.to_string(),
                    }),
                // Catégorie 2 : fichier réel mais extension non-.exe.
                ("[a-z]{1,10}", prop::sample::select(vec![".bat", ".cmd", ".txt", ".dll"]))
                    .prop_map(|(stem, ext)| P24Scenario::InvalidExt {
                        stem,
                        ext: ext.to_string(),
                    }),
                // Catégorie 3 : chemin inexistant (avec ou sans
                // extension). Couvre le cas « lecteur déplacé /
                // désinstallé » prévu par requirement 10.4.
                ("[a-z]{1,10}", prop::option::of(prop::sample::select(vec![".exe", ".bat", ".txt"])))
                    .prop_map(|(stem, ext)| P24Scenario::Nonexistent {
                        stem,
                        ext: ext.map(|s| s.to_string()),
                    }),
            ],
        ) {
            // TempDir éphémère par cas : isole entièrement l'effet
            // de bord (création/non-création de fichier) du reste de
            // la suite de tests.
            let dir = tempfile::tempdir().expect("tempdir P-24");

            let path: PathBuf = match &scenario {
                P24Scenario::ValidExe { stem, ext } => {
                    let p = dir.path().join(format!("{stem}{ext}"));
                    // En-tête MZ (executable PE) — le contenu n'a
                    // aucune importance pour la validation, qui ne
                    // lit que les métadonnées et l'extension.
                    std::fs::write(&p, b"MZ").expect("write valid exe P-24");
                    p
                }
                P24Scenario::InvalidExt { stem, ext } => {
                    let p = dir.path().join(format!("{stem}{ext}"));
                    std::fs::write(&p, b"x").expect("write non-exe P-24");
                    p
                }
                P24Scenario::Nonexistent { stem, ext } => {
                    let name = match ext {
                        Some(e) => format!("{stem}{e}"),
                        None => stem.clone(),
                    };
                    // Ne PAS créer le fichier : on veut un chemin
                    // qui n'existe pas pour cette catégorie.
                    dir.path().join(name)
                }
            };

            // Oracle recalculé à partir de l'état réel du système
            // de fichiers : c'est exactement la spécification du
            // requirement 10.3 (« persistance ssi fichier existe et
            // est .exe »), traduite sans dépendance à
            // `validate_external_player_path`.
            let exists_on_disk = path.is_file();
            let ext_lc_is_exe = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("exe"))
                .unwrap_or(false);
            let oracle = exists_on_disk && ext_lc_is_exe;

            let actual_ok = validate_external_player_path(&path).is_ok();

            // Cœur de la propriété : équivalence stricte entre la
            // décision de la fonction et la conjonction
            // (existence × extension .exe).
            prop_assert_eq!(
                actual_ok,
                oracle,
                "P-24 : validate_external_player_path doit être Ok ⇔ \
                 (existe sur disque ∧ extension == .exe). \
                 path={}, actual_ok={}, exists_on_disk={}, ext_lc_is_exe={}",
                path.display(),
                actual_ok,
                exists_on_disk,
                ext_lc_is_exe
            );

            // Sanity check de cohérence avec la catégorie générée :
            // - ValidExe : doit toujours être Ok ;
            // - InvalidExt et Nonexistent : doivent toujours être Err.
            match scenario {
                P24Scenario::ValidExe { .. } => prop_assert!(
                    actual_ok,
                    "P-24 : un .exe régulier matérialisé doit être accepté"
                ),
                P24Scenario::InvalidExt { .. } | P24Scenario::Nonexistent { .. } => {
                    prop_assert!(
                        !actual_ok,
                        "P-24 : extension non-.exe ou chemin inexistant doit être refusé"
                    )
                }
            }
        }
    }
}
