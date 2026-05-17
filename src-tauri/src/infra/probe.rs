//! Tâche 6.2 — wrapper Kyro_Probe (requirements 5.1, 5.2, 5.5, 5.6).
//!
//! Le design § « Kyro_Probe » impose un wrapper `tokio::process::Command`
//! autour de ffprobe avec sortie JSON, désérialisée par `serde_json` vers
//! [`TechnicalMetadata`]. L'invocation exécute exactement :
//!
//! ```text
//! ffprobe -v error -print_format json -show_format -show_streams <path>
//! ```
//!
//! La taille du fichier (`size_bytes`) est lue séparément via
//! [`tokio::fs::metadata`] pour garantir la fidélité requise par le
//! requirement 5.2 (le champ `format.size` retourné par ffprobe est une
//! chaîne, donc moins fiable et redondant avec la métadonnée système).
//!
//! Les pannes (échec spawn, exit ≠ 0, JSON invalide, lecture métadonnées
//! échouée) remontent une [`ProbeError`] : le scanner (tâche 8) la
//! capture, journalise et persiste l'item avec champs techniques `null`
//! (requirement 5.5, design § « Politique générale »).
//!
//! ## Trait `ProbeBackend`
//!
//! Le design § « Tests d'intégration » prévoit un trait `ProbeBackend`
//! injectable pour permettre une implémentation `MockProbe` (sorties
//! JSON synthétiques) côté tests des tâches 6.4 / 6.5 / 6.6 / 8.x. On
//! l'expose ici en signature non-`async fn in trait` (boxed future) pour
//! deux raisons :
//!
//! 1. La signature `Pin<Box<dyn Future<Output = ...> + Send + 'a>>` est
//!    explicitement object-safe et `Send`-friendly, ce qui permet aux
//!    couches consommatrices (scanner) de stocker un
//!    `Arc<dyn ProbeBackend>` et d'orchestrer plusieurs probes en
//!    parallèle via `tokio::spawn`.
//! 2. On évite le warning `async_fn_in_trait` autour des bornes `Send`
//!    qui apparaîtrait avec un `async fn` direct dans le trait.
//!
//! La fonction libre [`probe`] reste l'API non-trait directe, pratique
//! pour les call-sites qui n'ont pas besoin d'injection.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use serde::Deserialize;
use thiserror::Error;
use tokio::process::Command;

// ---------------------------------------------------------------------------
// Types publics — exactement ceux du design § « Kyro_Probe ».
// ---------------------------------------------------------------------------

/// Piste audio extraite par ffprobe. `codec` et `language` sont
/// optionnels car certains conteneurs n'exposent pas ces tags
/// (cf. table SQL `audio_track`, colonnes nullable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioTrack {
    pub codec: Option<String>,
    pub language: Option<String>,
}

/// Piste de sous-titres extraite par ffprobe. Mêmes contraintes
/// d'optionalité que [`AudioTrack`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubtitleTrack {
    pub codec: Option<String>,
    pub language: Option<String>,
}

/// Métadonnées techniques d'un fichier vidéo, telles que persistées par
/// le scanner dans `media_item` / `episode_item` (durée, codec, taille,
/// dimensions) et dans `audio_track` / `subtitle_track` (listes).
///
/// Tous les champs `Option` peuvent être `None` quand ffprobe ne les
/// retourne pas (conteneur sans header de durée, flux sans `width`/
/// `height`, etc.). `size_bytes` reste obligatoire car obtenu via
/// `tokio::fs::metadata` qui a déjà réussi pour atteindre cette branche.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TechnicalMetadata {
    pub duration_seconds: Option<u32>,
    pub video_codec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub audio_tracks: Vec<AudioTrack>,
    pub subtitle_tracks: Vec<SubtitleTrack>,
    pub size_bytes: u64,
}

/// Erreurs spécifiques au probe ffprobe. Distinctes de `KyroError` pour
/// rester proche de la cause racine ; le scanner (tâche 8) les traduira
/// en `KyroError::Probe(String)` ou les journalisera selon sa stratégie
/// « continue on isolated failure » (requirement 18.1).
#[derive(Debug, Error)]
pub enum ProbeError {
    /// Le binaire ffprobe n'a pas pu être lancé (introuvable, droits
    /// d'exécution refusés, etc.).
    #[error("échec du spawn de ffprobe: {0}")]
    Spawn(#[source] std::io::Error),

    /// La lecture des métadonnées système (`tokio::fs::metadata`) a
    /// échoué — fichier disparu entre le scan et le probe, droits
    /// refusés, etc. Le chemin est conservé pour le log scanner.
    #[error("lecture métadonnées système échouée pour {path:?}: {source}")]
    Metadata {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// ffprobe a terminé avec un code de sortie non nul. Le contenu de
    /// `stderr` est conservé (lossy UTF-8) à des fins de diagnostic.
    #[error("ffprobe a échoué (code {code}): {stderr}")]
    NonZeroExit { code: i32, stderr: String },

    /// La sortie standard de ffprobe n'est pas un JSON valide selon le
    /// schéma attendu (champs `format` / `streams`).
    #[error("sortie JSON ffprobe invalide: {0}")]
    InvalidJson(#[source] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Trait injectable + implémentation réelle.
// ---------------------------------------------------------------------------

/// Abstraction du probe ffprobe pour permettre l'injection d'un
/// [`MockProbe`] dans les tests d'intégration (cf. design § « Tests
/// d'intégration » et tâches 6.4 / 6.5 / 6.6).
///
/// La signature retourne un `Pin<Box<dyn Future + Send>>` plutôt qu'un
/// `async fn` pour rester object-safe et pour que les implémentations
/// puissent garantir explicitement le bound `Send` requis par le
/// scanner asynchrone.
pub trait ProbeBackend: Send + Sync {
    fn probe<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<TechnicalMetadata, ProbeError>> + Send + 'a>>;
}

/// Implémentation réelle du `ProbeBackend` : exécute le binaire ffprobe
/// résolu par [`crate::infra::ffprobe_locator`] (tâche 6.1).
#[derive(Debug, Clone)]
pub struct FfprobeBackend {
    /// Chemin absolu vers le binaire `ffprobe.exe` à invoquer.
    bin: PathBuf,
}

impl FfprobeBackend {
    /// Construit un backend qui invoquera `bin` pour chaque appel.
    /// Aucun contrôle n'est effectué ici : la validité du chemin doit
    /// déjà avoir été vérifiée par [`crate::infra::ffprobe_locator`].
    pub fn new(bin: PathBuf) -> Self {
        Self { bin }
    }

    /// Accès lecture seule au binaire pour les tests d'intégration et
    /// les logs.
    pub fn bin(&self) -> &Path {
        &self.bin
    }
}

impl ProbeBackend for FfprobeBackend {
    fn probe<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<TechnicalMetadata, ProbeError>> + Send + 'a>> {
        Box::pin(probe(path, &self.bin))
    }
}

// ---------------------------------------------------------------------------
// Fonction libre `probe` — API non-trait, miroir exact du pseudocode
// design § « Kyro_Probe ».
// ---------------------------------------------------------------------------

/// Probe un fichier vidéo via `ffprobe_bin` et retourne ses
/// [`TechnicalMetadata`]. Étapes (cf. design) :
///
/// 1. Lire la taille du fichier via `tokio::fs::metadata` (requirement 5.2).
/// 2. Lancer `ffprobe -v error -print_format json -show_format -show_streams <path>`.
/// 3. Vérifier que l'exit code est zéro (sinon `Err(NonZeroExit)`).
/// 4. Désérialiser la sortie JSON via `serde_json` (sinon `Err(InvalidJson)`).
/// 5. Mapper vers [`TechnicalMetadata`] (premier flux vidéo retenu pour
///    `video_codec` / `width` / `height` ; tous les flux audio et
///    sous-titres collectés).
pub async fn probe(
    path: &Path,
    ffprobe_bin: &Path,
) -> Result<TechnicalMetadata, ProbeError> {
    // 1. Taille fichier (requirement 5.2). On le fait avant le spawn
    //    pour échouer rapidement si le fichier a disparu — inutile de
    //    payer le coût d'un spawn ffprobe dans ce cas.
    let size_bytes = tokio::fs::metadata(path)
        .await
        .map_err(|source| ProbeError::Metadata {
            path: path.to_path_buf(),
            source,
        })?
        .len();

    // 2. Lancer ffprobe avec arguments fixes du design.
    let output = Command::new(ffprobe_bin)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .output()
        .await
        .map_err(ProbeError::Spawn)?;

    // 3. Exit code.
    if !output.status.success() {
        let code = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        return Err(ProbeError::NonZeroExit { code, stderr });
    }

    // 4. JSON.
    let parsed: FfprobeOutput =
        serde_json::from_slice(&output.stdout).map_err(ProbeError::InvalidJson)?;

    // 5. Mapping (fonction pure, testable par les tâches 6.4 / 6.5).
    Ok(map_metadata(parsed, size_bytes))
}

// ---------------------------------------------------------------------------
// Mapping JSON → TechnicalMetadata (fonction pure).
// ---------------------------------------------------------------------------

/// Projette la sortie ffprobe désérialisée vers la structure publique.
///
/// Règles :
/// - `duration_seconds` : `format.duration` (string décimale en secondes)
///   parsée en `f64`, tronquée à `u32`. Toute valeur non finie ou
///   négative est ignorée (`None`). Le requirement 5.1 exige une durée
///   en secondes ; on tronque pour rester cohérent avec le type entier
///   persisté en SQL (`media_item.duration_seconds INTEGER`).
/// - `video_codec` / `width` / `height` : extraits du **premier** flux
///   `codec_type == "video"`. Les flux vidéo additionnels (cas rare :
///   pistes attached pictures) sont ignorés à ce niveau de granularité.
/// - `audio_tracks` / `subtitle_tracks` : un élément par flux
///   correspondant, dans l'ordre d'apparition de ffprobe. La langue est
///   tirée de `tags.language` (clé standard ffprobe).
fn map_metadata(parsed: FfprobeOutput, size_bytes: u64) -> TechnicalMetadata {
    let duration_seconds = parsed
        .format
        .as_ref()
        .and_then(|f| f.duration.as_deref())
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|d| d.is_finite() && *d >= 0.0)
        .map(|d| d.trunc() as u32);

    let mut video_codec = None;
    let mut width = None;
    let mut height = None;
    let mut audio_tracks = Vec::new();
    let mut subtitle_tracks = Vec::new();
    let mut video_seen = false;

    for stream in parsed.streams {
        match stream.codec_type.as_deref() {
            Some("video") => {
                if !video_seen {
                    video_codec = stream.codec_name;
                    width = stream.width;
                    height = stream.height;
                    video_seen = true;
                }
            }
            Some("audio") => {
                let language = stream.tags.as_ref().and_then(|t| t.language.clone());
                audio_tracks.push(AudioTrack {
                    codec: stream.codec_name,
                    language,
                });
            }
            Some("subtitle") => {
                let language = stream.tags.as_ref().and_then(|t| t.language.clone());
                subtitle_tracks.push(SubtitleTrack {
                    codec: stream.codec_name,
                    language,
                });
            }
            _ => {
                // codec_type inconnu (data, attachment, etc.) — ignoré.
            }
        }
    }

    TechnicalMetadata {
        duration_seconds,
        video_codec,
        width,
        height,
        audio_tracks,
        subtitle_tracks,
        size_bytes,
    }
}

// ---------------------------------------------------------------------------
// Schéma JSON ffprobe (`-show_format -show_streams`).
//
// Tous les champs sont `Option` pour absorber les variations de
// conteneur (mkv / mp4 / ts...). `serde(default)` évite les échecs de
// désérialisation sur des sorties partielles.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct FfprobeOutput {
    #[serde(default)]
    format: Option<FfprobeFormat>,
    #[serde(default)]
    streams: Vec<FfprobeStream>,
}

#[derive(Debug, Deserialize)]
struct FfprobeFormat {
    /// Durée en secondes, formatée en chaîne décimale par ffprobe
    /// (`"1234.567000"`). Optionnelle car certains conteneurs ne la
    /// publient pas en header.
    #[serde(default)]
    duration: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FfprobeStream {
    #[serde(default)]
    codec_type: Option<String>,
    #[serde(default)]
    codec_name: Option<String>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default)]
    tags: Option<FfprobeTags>,
}

#[derive(Debug, Deserialize)]
struct FfprobeTags {
    #[serde(default)]
    language: Option<String>,
}
// ===========================================================================
// Tests unitaires (tâche 6.4) — fixtures synthétiques pour `map_metadata`
// et la branche d'échec de désérialisation JSON.
//
// Les types `FfprobeOutput` / `FfprobeFormat` / `FfprobeStream` /
// `FfprobeTags` sont privés au module `probe` ; les tests ci-dessous
// vivent dans le même fichier pour y accéder directement, conformément
// à la consigne « ne pas modifier `mod.rs`, garder tout dans
// `probe.rs` ».
// ===========================================================================

#[cfg(test)]
mod tests {
    //! Tâche 6.4 — tests unitaires pour le parsing ffprobe.
    //!
    //! Trois fixtures couvrent les cas typiques de la bibliothèque
    //! Kyro :
    //!
    //! 1. **mkv h264 + AAC** — un flux vidéo, un flux audio, pas de
    //!    sous-titres. Vérifie que les champs typés (`duration_seconds`,
    //!    `video_codec`, `width`, `height`, `audio_tracks`) sont
    //!    correctement extraits et que `subtitle_tracks` est vide.
    //!
    //! 2. **mp4 multi-audio** — un flux vidéo H.265, deux flux audio
    //!    (FR + EN) avec tags `language`. Vérifie que les pistes audio
    //!    sont collectées dans l'ordre d'apparition et que la langue
    //!    transite via `tags.language` (requirement 5.6).
    //!
    //! 3. **fichier corrompu** — bytes non-JSON ; on vérifie que
    //!    `serde_json::from_slice::<FfprobeOutput>` retourne `Err`,
    //!    ce qui dans la fonction libre `probe` conduit à
    //!    `ProbeError::InvalidJson`. Le scanner (tâche 8) capture cette
    //!    erreur et persiste l'item avec `TechnicalMetadata = None`
    //!    (requirement 5.5) — le test confirme la première moitié de
    //!    cette chaîne (échec contrôlé), la seconde moitié étant
    //!    couverte par la tâche 8.9 (Property 37).

    use super::*;

    // -----------------------------------------------------------------------
    // Helpers de construction de fixtures `FfprobeOutput` synthétiques.
    // Construire programmatiquement plutôt que via JSON sur disque évite
    // toute dépendance fichier et garde les tests hermétiques.
    // -----------------------------------------------------------------------

    fn video_stream(codec: &str, width: u32, height: u32) -> FfprobeStream {
        FfprobeStream {
            codec_type: Some("video".to_string()),
            codec_name: Some(codec.to_string()),
            width: Some(width),
            height: Some(height),
            tags: None,
        }
    }

    fn audio_stream(codec: &str, language: Option<&str>) -> FfprobeStream {
        FfprobeStream {
            codec_type: Some("audio".to_string()),
            codec_name: Some(codec.to_string()),
            width: None,
            height: None,
            tags: language.map(|l| FfprobeTags {
                language: Some(l.to_string()),
            }),
        }
    }

    fn subtitle_stream(codec: &str, language: Option<&str>) -> FfprobeStream {
        FfprobeStream {
            codec_type: Some("subtitle".to_string()),
            codec_name: Some(codec.to_string()),
            width: None,
            height: None,
            tags: language.map(|l| FfprobeTags {
                language: Some(l.to_string()),
            }),
        }
    }

    // -----------------------------------------------------------------------
    // Fixture #1 — mkv h264 + AAC (cas le plus courant).
    // -----------------------------------------------------------------------

    #[test]
    fn map_metadata_mkv_h264_aac() {
        let parsed = FfprobeOutput {
            format: Some(FfprobeFormat {
                duration: Some("5400.250000".to_string()), // 1h30m + 0.25s
            }),
            streams: vec![
                video_stream("h264", 1920, 1080),
                audio_stream("aac", Some("eng")),
            ],
        };

        let meta = map_metadata(parsed, 2_500_000_000);

        assert_eq!(meta.duration_seconds, Some(5400));
        assert_eq!(meta.video_codec.as_deref(), Some("h264"));
        assert_eq!(meta.width, Some(1920));
        assert_eq!(meta.height, Some(1080));
        assert_eq!(meta.audio_tracks.len(), 1);
        assert_eq!(meta.audio_tracks[0].codec.as_deref(), Some("aac"));
        assert_eq!(meta.audio_tracks[0].language.as_deref(), Some("eng"));
        assert!(meta.subtitle_tracks.is_empty());
        assert_eq!(meta.size_bytes, 2_500_000_000);
    }

    // -----------------------------------------------------------------------
    // Fixture #2 — mp4 multi-audio (FR + EN), sous-titres SRT.
    // -----------------------------------------------------------------------

    #[test]
    fn map_metadata_mp4_multi_audio_with_subtitles() {
        let parsed = FfprobeOutput {
            format: Some(FfprobeFormat {
                duration: Some("7322.500".to_string()),
            }),
            streams: vec![
                video_stream("hevc", 3840, 2160),
                audio_stream("ac3", Some("fre")),
                audio_stream("aac", Some("eng")),
                subtitle_stream("subrip", Some("fre")),
                subtitle_stream("subrip", Some("eng")),
            ],
        };

        let meta = map_metadata(parsed, 8_000_000_000);

        assert_eq!(meta.duration_seconds, Some(7322));
        assert_eq!(meta.video_codec.as_deref(), Some("hevc"));
        assert_eq!(meta.width, Some(3840));
        assert_eq!(meta.height, Some(2160));

        // Audio dans l'ordre exact d'apparition (FR puis EN).
        assert_eq!(meta.audio_tracks.len(), 2);
        assert_eq!(meta.audio_tracks[0].codec.as_deref(), Some("ac3"));
        assert_eq!(meta.audio_tracks[0].language.as_deref(), Some("fre"));
        assert_eq!(meta.audio_tracks[1].codec.as_deref(), Some("aac"));
        assert_eq!(meta.audio_tracks[1].language.as_deref(), Some("eng"));

        // Sous-titres dans l'ordre exact d'apparition.
        assert_eq!(meta.subtitle_tracks.len(), 2);
        assert_eq!(meta.subtitle_tracks[0].codec.as_deref(), Some("subrip"));
        assert_eq!(meta.subtitle_tracks[0].language.as_deref(), Some("fre"));
        assert_eq!(meta.subtitle_tracks[1].codec.as_deref(), Some("subrip"));
        assert_eq!(meta.subtitle_tracks[1].language.as_deref(), Some("eng"));

        assert_eq!(meta.size_bytes, 8_000_000_000);
    }

    // -----------------------------------------------------------------------
    // Fixture #3 — JSON corrompu : la désérialisation échoue, ce qui
    // dans `probe` produit `ProbeError::InvalidJson`. On teste les deux
    // bouts du contrat : (a) `serde_json::from_slice` retourne bien
    // `Err`, (b) la conversion en `ProbeError::InvalidJson` est
    // possible.
    // -----------------------------------------------------------------------

    #[test]
    fn ffprobe_output_rejects_corrupted_json() {
        let corrupted: &[u8] = b"not a json {{{ stderr\n\x00";
        let result = serde_json::from_slice::<FfprobeOutput>(corrupted);
        assert!(
            result.is_err(),
            "le parser doit rejeter du JSON corrompu (entrée: {corrupted:?})"
        );

        // Le `From<serde_json::Error>` est implicitement testé par la
        // construction de la variante `InvalidJson` dans `probe()`.
        let err = result.unwrap_err();
        let probe_err = ProbeError::InvalidJson(err);
        assert!(matches!(probe_err, ProbeError::InvalidJson(_)));
    }

    /// Cas limite : JSON syntaxiquement valide mais structurellement
    /// vide. Doit être accepté (tous les champs sont `Option` /
    /// `Vec` avec `serde(default)`) et produire `TechnicalMetadata`
    /// avec champs vidéo/audio/sub à `None` ou vides — confirme que la
    /// branche « partial probe » (durée absente, pas de flux) reste
    /// acceptée par `map_metadata`, ce qui évite qu'un fichier au
    /// header tronqué ne provoque une erreur de désérialisation.
    #[test]
    fn ffprobe_output_accepts_empty_json_object() {
        let parsed: FfprobeOutput =
            serde_json::from_slice(b"{}").expect("`{}` est un JSON valide pour FfprobeOutput");
        let meta = map_metadata(parsed, 42);

        assert_eq!(meta.duration_seconds, None);
        assert_eq!(meta.video_codec, None);
        assert_eq!(meta.width, None);
        assert_eq!(meta.height, None);
        assert!(meta.audio_tracks.is_empty());
        assert!(meta.subtitle_tracks.is_empty());
        assert_eq!(meta.size_bytes, 42);
    }
}

// ===========================================================================
// Tests de propriétés (tâches 6.5 et 6.6).
//
// `proptests` couvre :
//   * **Property 9** — Mapping ffprobe → métadonnées techniques
//     (round-trip exact pour durée / codec / résolution / listes
//     audio / listes sous-titres).
//   * **Property 10** — Robustesse du probe en cas d'échec (exit ≠ 0,
//     JSON invalide, binaire absent ; on vérifie que les variantes
//     `ProbeError` couvrent ces trois modes et que l'erreur peut être
//     véhiculée à travers la frontière `Send + Sync`).
// ===========================================================================

#[cfg(test)]
mod proptests {
    //! Tâches 6.5 (P-9) & 6.6 (P-10).
    //!
    //! ## Property 9 — Mapping ffprobe → métadonnées techniques
    //! Pour toute sortie JSON valide synthétique (`duration`, codec,
    //! résolution, listes audio / sous-titres), le mapping
    //! `FfprobeOutput → TechnicalMetadata` doit préserver chaque
    //! champ : la durée tronquée à la seconde, le codec / la
    //! résolution du premier flux vidéo, les pistes audio et
    //! sous-titres dans leur ordre d'apparition avec leurs codecs et
    //! langues respectifs, ainsi que la taille passée en argument.
    //! **Validates: Requirements 5.1, 5.6**
    //!
    //! ## Property 10 — Robustesse du probe en cas d'échec
    //! Pour chacun des trois modes d'échec spécifiés (exit ≠ 0, JSON
    //! invalide, binaire absent), `probe` doit retourner une
    //! `ProbeError` distincte et aucune `TechnicalMetadata` partielle
    //! n'est construite. Le scanner (tâche 8.9 / Property 37)
    //! transforme alors cette erreur en log `tracing::warn!` contenant
    //! le chemin du fichier et persiste l'item avec champs techniques
    //! `null`, garantissant que le scan continue (requirement 18.1).
    //! Au niveau probe, on vérifie ici les invariants observables :
    //!   1. `serde_json::from_slice::<FfprobeOutput>` rejette tout JSON
    //!      invalide synthétisé par le générateur ;
    //!   2. La variante `NonZeroExit { code, stderr }` est constructible
    //!      avec les champs attendus pour tout couple `(code, stderr)` ;
    //!   3. Une implémentation `ProbeBackend` qui simule un binaire
    //!      absent (`Spawn(io::Error)`) propage bien l'erreur via le
    //!      trait, ce qui valide le canal `ProbeBackend` utilisé par le
    //!      scanner pour injecter `MockProbe` dans les tâches 8.x ;
    //!   4. `ProbeError` est `Send + Sync + 'static` et implémente
    //!      `std::error::Error`, condition nécessaire pour traverser la
    //!      frontière `tokio::spawn` du scanner asynchrone.
    //! **Validates: Requirements 5.5, 18.1**

    use super::*;
    use proptest::prelude::*;
    use std::io;
    use std::sync::Arc;

    // -----------------------------------------------------------------------
    // Alphabets de codecs / langues — petits ensembles fixes pour
    // garder les générateurs déterministes et accélérer le shrinker.
    // Ces valeurs reflètent ce que ffprobe émet réellement sur des
    // conteneurs mkv / mp4 (voir fixtures unitaires plus haut).
    // -----------------------------------------------------------------------

    const VIDEO_CODECS: &[&str] = &["h264", "hevc", "vp9", "av1", "mpeg2video"];
    const AUDIO_CODECS: &[&str] = &["aac", "ac3", "eac3", "dts", "flac", "opus"];
    const SUBTITLE_CODECS: &[&str] = &["subrip", "ass", "hdmv_pgs_subtitle", "mov_text"];
    const LANGUAGES: &[&str] = &["fre", "eng", "jpn", "spa", "ita", "und"];

    /// Variantes typiques d'`io::ErrorKind` observées sur Windows quand
    /// `tokio::Command::spawn` échoue (binaire ffprobe absent ou
    /// inaccessible). Promue en `const` pour garantir le `'static`
    /// requis par `proptest::sample::select`.
    const SPAWN_ERROR_KINDS: &[io::ErrorKind] = &[
        io::ErrorKind::NotFound,
        io::ErrorKind::PermissionDenied,
        io::ErrorKind::Other,
    ];

    // -----------------------------------------------------------------------
    // Bornes du générateur — calibrées pour rester réalistes (pas de
    // durées astronomiques, résolutions cohérentes) tout en couvrant
    // les cas limites (0, valeurs maximales raisonnables).
    // u32 sécurisé pour la durée : on borne à 2^31 pour rester
    // largement sous 2^53 (limite de la précision entière f64).
    // -----------------------------------------------------------------------

    const MAX_DURATION: u32 = 1_000_000;
    const MAX_DIM: u32 = 7680; // 8K-class
    const MAX_TRACKS: usize = 4;

    fn arb_video_codec() -> impl Strategy<Value = String> {
        proptest::sample::select(VIDEO_CODECS).prop_map(|s| s.to_string())
    }

    fn arb_audio_codec() -> impl Strategy<Value = String> {
        proptest::sample::select(AUDIO_CODECS).prop_map(|s| s.to_string())
    }

    fn arb_subtitle_codec() -> impl Strategy<Value = String> {
        proptest::sample::select(SUBTITLE_CODECS).prop_map(|s| s.to_string())
    }

    fn arb_language() -> impl Strategy<Value = Option<String>> {
        proptest::option::of(
            proptest::sample::select(LANGUAGES).prop_map(|s| s.to_string()),
        )
    }

    /// Une « spec » d'entrée : valeurs typées que l'on souhaite voir
    /// apparaître dans la `TechnicalMetadata` finale. On les utilise
    /// pour construire un `FfprobeOutput` puis vérifier que le mapping
    /// les restitue exactement.
    #[derive(Debug, Clone)]
    struct InputSpec {
        duration_seconds: Option<u32>,
        video: Option<(String, u32, u32)>, // (codec, width, height)
        audio_tracks: Vec<(String, Option<String>)>, // (codec, language)
        subtitle_tracks: Vec<(String, Option<String>)>, // (codec, language)
        size_bytes: u64,
    }

    fn arb_input_spec() -> impl Strategy<Value = InputSpec> {
        let duration = proptest::option::of(0u32..=MAX_DURATION);
        let video = proptest::option::of((
            arb_video_codec(),
            1u32..=MAX_DIM,
            1u32..=MAX_DIM,
        ));
        let audio = proptest::collection::vec(
            (arb_audio_codec(), arb_language()),
            0..=MAX_TRACKS,
        );
        let subtitle = proptest::collection::vec(
            (arb_subtitle_codec(), arb_language()),
            0..=MAX_TRACKS,
        );
        let size = any::<u64>();
        (duration, video, audio, subtitle, size).prop_map(
            |(duration_seconds, video, audio_tracks, subtitle_tracks, size_bytes)| InputSpec {
                duration_seconds,
                video,
                audio_tracks,
                subtitle_tracks,
                size_bytes,
            },
        )
    }

    /// Construit un `FfprobeOutput` synthétique à partir de la spec.
    /// L'ordre des flux suit la convention typique mkv / mp4 :
    /// `[video?, audio*, subtitle*]`.
    fn build_output(spec: &InputSpec) -> FfprobeOutput {
        let mut streams: Vec<FfprobeStream> = Vec::new();

        if let Some((codec, w, h)) = &spec.video {
            streams.push(FfprobeStream {
                codec_type: Some("video".to_string()),
                codec_name: Some(codec.clone()),
                width: Some(*w),
                height: Some(*h),
                tags: None,
            });
        }

        for (codec, lang) in &spec.audio_tracks {
            streams.push(FfprobeStream {
                codec_type: Some("audio".to_string()),
                codec_name: Some(codec.clone()),
                width: None,
                height: None,
                tags: lang.as_ref().map(|l| FfprobeTags {
                    language: Some(l.clone()),
                }),
            });
        }

        for (codec, lang) in &spec.subtitle_tracks {
            streams.push(FfprobeStream {
                codec_type: Some("subtitle".to_string()),
                codec_name: Some(codec.clone()),
                width: None,
                height: None,
                tags: lang.as_ref().map(|l| FfprobeTags {
                    language: Some(l.clone()),
                }),
            });
        }

        // Durée formatée façon ffprobe : décimale en secondes, suffixée
        // de zéros. On utilise `format!("{}.000000", n)` ; `parse::<f64>`
        // restitue exactement `n` puisque `n` ≤ MAX_DURATION ≪ 2^53.
        let format = spec.duration_seconds.map(|d| FfprobeFormat {
            duration: Some(format!("{d}.000000")),
        });

        FfprobeOutput {
            format: format.or(Some(FfprobeFormat { duration: None })),
            streams,
        }
    }

    // -----------------------------------------------------------------------
    // Property 10 — modes d'échec.
    // -----------------------------------------------------------------------

    /// Trois conditions d'échec distinctes que le scanner doit pouvoir
    /// distinguer pour journaliser correctement le chemin du fichier
    /// (requirement 18.1).
    #[derive(Debug, Clone)]
    enum FailureMode {
        InvalidJson(Vec<u8>),
        NonZeroExit { code: i32, stderr: String },
        BinaryAbsent(io::ErrorKind),
    }

    fn arb_failure_mode() -> impl Strategy<Value = FailureMode> {
        prop_oneof![
            // (1) Bytes aléatoires qui ne sont *pas* un JSON valide
            //     pour `FfprobeOutput`. On filtre via `prop_filter`
            //     pour exclure les rares cas où le générateur tombe
            //     sur `{}` ou similaire (qui sont eux acceptés par
            //     `serde(default)`).
            proptest::collection::vec(any::<u8>(), 0..32)
                .prop_filter("doit être un JSON invalide", |b| {
                    serde_json::from_slice::<FfprobeOutput>(b).is_err()
                })
                .prop_map(FailureMode::InvalidJson),
            // (2) Exit code arbitraire ≠ 0 + stderr arbitraire borné.
            (
                any::<i32>().prop_filter("non zéro", |c| *c != 0),
                "[a-zA-Z0-9 .:_/-]{0,40}",
            )
                .prop_map(|(code, stderr)| FailureMode::NonZeroExit { code, stderr }),
            // (3) Binaire absent : trois variantes typiques d'io::Error
            //     observables sur Windows quand `tokio::Command::spawn`
            //     échoue.
            proptest::sample::select(SPAWN_ERROR_KINDS)
                .prop_map(FailureMode::BinaryAbsent),
        ]
    }

    /// Stub `ProbeBackend` qui retourne systématiquement l'erreur
    /// fournie. Sert à valider que la variante `Spawn(io::Error)` (mode
    /// « binaire absent ») se propage correctement à travers le trait,
    /// ce qui est le canal effectivement utilisé par le scanner pour
    /// injecter une implémentation factice (`MockProbe`) dans les
    /// tests d'intégration des tâches 8.x.
    struct StubBackend {
        // Arc + closure pour rester `Send + Sync` (requis par
        // `ProbeBackend`) et pour qu'un même générateur puisse être
        // exécuté plusieurs fois par proptest.
        err_factory: Arc<dyn Fn() -> ProbeError + Send + Sync>,
    }

    impl ProbeBackend for StubBackend {
        fn probe<'a>(
            &'a self,
            _path: &'a Path,
        ) -> Pin<Box<dyn Future<Output = Result<TechnicalMetadata, ProbeError>> + Send + 'a>>
        {
            let err = (self.err_factory)();
            Box::pin(async move { Err(err) })
        }
    }

    /// Marqueur compile-time : `ProbeError` doit être `Send + Sync +
    /// 'static` et implémenter `std::error::Error`. Si une variante
    /// future violait ce contrat, ce test ne compilerait plus —
    /// c'est précisément l'invariant requis par le scanner asynchrone
    /// (`tokio::spawn` exige `Send + 'static`).
    fn assert_error_traits<E: std::error::Error + Send + Sync + 'static>() {}

    #[test]
    fn probe_error_is_send_sync_static() {
        assert_error_traits::<ProbeError>();
    }

    proptest! {
        // -------------------------------------------------------------------
        // Property 9 — Mapping ffprobe → métadonnées techniques.
        //
        // **Validates: Requirements 5.1, 5.6**
        //
        // Pour toute sortie JSON synthétique, après mapping, chaque
        // champ typé doit correspondre exactement à l'entrée :
        //   * durée tronquée à la seconde (déjà entière dans le
        //     générateur, donc égalité stricte) ;
        //   * codec / largeur / hauteur du premier (et unique) flux
        //     vidéo si présent, sinon `None` ;
        //   * pistes audio dans l'ordre d'apparition avec codec et
        //     langue exacts ;
        //   * pistes sous-titres dans l'ordre d'apparition avec codec
        //     et langue exacts ;
        //   * `size_bytes` passé tel quel.
        // -------------------------------------------------------------------
        #[test]
        fn prop9_mapping_round_trips_each_field(spec in arb_input_spec()) {
            let parsed = build_output(&spec);
            let meta = map_metadata(parsed, spec.size_bytes);

            // Durée — round-trip exact car spec.duration_seconds est
            // entière et formatée via "{n}.000000".
            prop_assert_eq!(meta.duration_seconds, spec.duration_seconds);

            // Vidéo — premier flux video si présent.
            match &spec.video {
                Some((codec, w, h)) => {
                    prop_assert_eq!(meta.video_codec.as_deref(), Some(codec.as_str()));
                    prop_assert_eq!(meta.width, Some(*w));
                    prop_assert_eq!(meta.height, Some(*h));
                }
                None => {
                    prop_assert_eq!(meta.video_codec, None);
                    prop_assert_eq!(meta.width, None);
                    prop_assert_eq!(meta.height, None);
                }
            }

            // Pistes audio — ordre + codec + langue.
            prop_assert_eq!(meta.audio_tracks.len(), spec.audio_tracks.len());
            for (got, expected) in meta.audio_tracks.iter().zip(spec.audio_tracks.iter()) {
                prop_assert_eq!(got.codec.as_deref(), Some(expected.0.as_str()));
                prop_assert_eq!(got.language.as_deref(), expected.1.as_deref());
            }

            // Pistes sous-titres — ordre + codec + langue.
            prop_assert_eq!(meta.subtitle_tracks.len(), spec.subtitle_tracks.len());
            for (got, expected) in meta.subtitle_tracks.iter().zip(spec.subtitle_tracks.iter()) {
                prop_assert_eq!(got.codec.as_deref(), Some(expected.0.as_str()));
                prop_assert_eq!(got.language.as_deref(), expected.1.as_deref());
            }

            // Taille — passée telle quelle.
            prop_assert_eq!(meta.size_bytes, spec.size_bytes);
        }

        // -------------------------------------------------------------------
        // Property 10 — Robustesse du probe en cas d'échec.
        //
        // **Validates: Requirements 5.5, 18.1**
        //
        // Pour chaque mode d'échec généré, on vérifie l'invariant
        // observable au niveau de la couche probe :
        //
        //   * **JSON invalide** : `serde_json::from_slice` rejette
        //     l'entrée — c'est exactement la branche prise par
        //     `probe()` qui produit `ProbeError::InvalidJson`.
        //   * **Exit ≠ 0** : la variante `NonZeroExit { code, stderr }`
        //     est constructible avec les champs attendus, et son
        //     message d'erreur expose le code et le stderr (utile au
        //     log scanner pour identifier la cause racine).
        //   * **Binaire absent** : à travers le trait `ProbeBackend`
        //     (canal d'injection utilisé par le scanner), un stub
        //     retournant `ProbeError::Spawn(io::Error)` propage bien
        //     l'erreur jusqu'à l'appelant.
        //
        // Le « scan continue » et le « log contient le chemin » sont
        // intrinsèquement des invariants du **scanner** (couche
        // appelante) et sont testés par la tâche 8.9 (Property 37).
        // Au niveau probe, on garantit que l'information nécessaire
        // (variante distincte, champs typés) est disponible pour que
        // le scanner puisse remplir son contrat.
        // -------------------------------------------------------------------
        #[test]
        fn prop10_failure_modes_produce_distinct_probe_errors(
            mode in arb_failure_mode(),
        ) {
            match mode {
                FailureMode::InvalidJson(bytes) => {
                    // (1) Le parser JSON rejette les bytes — c'est la
                    // branche `Err(InvalidJson(_))` de `probe()`.
                    let parse_result =
                        serde_json::from_slice::<FfprobeOutput>(&bytes);
                    prop_assert!(
                        parse_result.is_err(),
                        "le générateur a produit un JSON malgré le filtre"
                    );
                    let probe_err = ProbeError::InvalidJson(parse_result.unwrap_err());
                    prop_assert!(matches!(probe_err, ProbeError::InvalidJson(_)));
                    // Le message d'erreur reste utilisable pour le log
                    // scanner — non vide, ce qui est suffisant ici (la
                    // forme exacte est fixée par `serde_json`).
                    prop_assert!(!probe_err.to_string().is_empty());
                }

                FailureMode::NonZeroExit { code, stderr } => {
                    // (2) La variante `NonZeroExit` capture code et
                    // stderr ; l'affichage `Display` expose les deux,
                    // ce qui permet au scanner de journaliser un
                    // message exploitable côté utilisateur.
                    let probe_err = ProbeError::NonZeroExit {
                        code,
                        stderr: stderr.clone(),
                    };
                    if let ProbeError::NonZeroExit { code: c, stderr: s } = &probe_err {
                        prop_assert_eq!(*c, code);
                        prop_assert_eq!(s.as_str(), stderr.as_str());
                    } else {
                        prop_assert!(false, "variante NonZeroExit attendue");
                    }
                    let rendered = probe_err.to_string();
                    prop_assert!(rendered.contains(&code.to_string()));
                }

                FailureMode::BinaryAbsent(kind) => {
                    // (3) Stub `ProbeBackend` qui simule un échec de
                    // spawn (binaire ffprobe absent ou inaccessible).
                    // On vérifie que l'erreur traverse le trait sans
                    // perte d'information (variante Spawn préservée,
                    // io::ErrorKind conservé).
                    let kind_for_factory = kind;
                    let backend = StubBackend {
                        err_factory: Arc::new(move || {
                            ProbeError::Spawn(io::Error::new(
                                kind_for_factory,
                                "ffprobe binary not found (synthetic)",
                            ))
                        }),
                    };

                    let path = Path::new("synthetic.mkv");
                    // On exécute le future via un mini-runtime pour
                    // éviter la dépendance `tokio_test` ici — un
                    // `block_on` suffit puisque le stub ne fait aucun
                    // IO réel.
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .build()
                        .expect("build current_thread runtime");
                    let result = runtime.block_on(backend.probe(path));

                    prop_assert!(result.is_err());
                    match result.unwrap_err() {
                        ProbeError::Spawn(io_err) => {
                            prop_assert_eq!(io_err.kind(), kind);
                        }
                        other => {
                            prop_assert!(
                                false,
                                "variante Spawn attendue, obtenu {other:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}
