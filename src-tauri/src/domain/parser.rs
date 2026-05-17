//! Kyro_Parser — extraction de titre, année, qualité et numérotation
//! d'épisode depuis un nom de fichier (requirement 4.x).
//!
//! Module pur, sans IO ni accès aux modules d'infrastructure. Toutes les
//! regex sont compilées une seule fois via `OnceLock` afin de garantir
//! une performance prévisible quand le scanner appelle `parse` sur des
//! milliers de chemins.
//!
//! Pipeline (cf. design § Kyro_Parser) :
//!  1. `strip_extension` — retire l'extension finale (`.mkv`, `.mp4`…).
//!  2. `extract_quality` — token `480p|720p|1080p|1440p|2160p|4k`.
//!  3. `extract_year` — entier ∈ [1900, 2099].
//!  4. `extract_sxxexx` — motif `S<saison>E<épisode>`.
//!  5. `extract_n_x_n` — motif `<saison>x<épisode>` (fallback).
//!  6. `extract_loose_episode` — motif `ép./ep./episode/-` + numéro
//!     (fallback ; saison forcée à 1).
//!  7. `clean_title` — retire les segments reconnus puis remplace les
//!     délimiteurs `.`, `_`, `-` par des espaces.

use std::sync::OnceLock;

use regex::Regex;

use super::types::{Category, NumberingSource, Quality};

/// Résultat de parsing d'un nom de fichier vidéo.
///
/// Tous les champs hormis `title` sont optionnels : si la regex
/// correspondante n'a rien trouvé, la valeur reste `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedFilename {
    pub title: String,
    pub year: Option<u16>,
    pub quality: Option<Quality>,
    pub episode: Option<EpisodeNumbering>,
}

/// Numérotation `(saison, épisode)` extraite et l'origine du motif qui
/// l'a produite (utile pour distinguer `S01E02` d'un fallback `- 12`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EpisodeNumbering {
    pub season: u16,
    pub episode: u16,
    pub source: NumberingSource,
}

// ---------------------------------------------------------------------------
// Regex compilées paresseusement.
// ---------------------------------------------------------------------------

fn quality_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b(480p|720p|1080p|1440p|2160p|4k)\b").expect("regex qualité valide")
    })
}

fn year_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b(19[0-9]{2}|20[0-9]{2})\b").expect("regex année valide"))
}

fn sxxexx_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\bs(?P<s>\d{1,2})[\s._-]?e(?P<e>\d{1,3})\b")
            .expect("regex SxxExx valide")
    })
}

fn n_x_n_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b(?P<s>\d{1,2})x(?P<e>\d{1,3})\b").expect("regex NxN valide")
    })
}

fn loose_episode_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(?:ép\.?|ep\.?|episode|\-)\s*(?P<e>\d{1,3})\b")
            .expect("regex épisode souple valide")
    })
}

/// Numéro nu en fin de « stem » utile, après les segments reconnus
/// (qualité, année, SxxExx…) déjà retirés. On capture explicitement la
/// fin de chaîne (ou un crochet/parenthèse de tag releaser) pour ne
/// pas confondre avec un numéro situé au milieu (ex. « V2 », « N° 5 »
/// dans le titre lui-même). Saison forcée à 1.
///
/// Exemples détectés :
///   - `Bleach Kai 042`
///   - `Naruto Shippuden 12`
///   - `One Piece 1099 [final]`
///   - `Attack On Titan 87 (1080p)`
fn trailing_number_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?P<e>\d{1,4})\s*(?:[\[\(].*)?$").expect("regex épisode trailing valide")
    })
}

// ---------------------------------------------------------------------------
// Pipeline public.
// ---------------------------------------------------------------------------

/// Point d'entrée : extrait toutes les informations reconnues du nom de
/// fichier brut (extension comprise).
pub fn parse(filename: &str) -> ParsedFilename {
    let stem = strip_extension(filename);

    let quality = extract_quality(stem);
    let year = extract_year(stem);
    let episode = extract_sxxexx(stem)
        .or_else(|| extract_n_x_n(stem))
        .or_else(|| extract_loose_episode(stem))
        .or_else(|| extract_trailing_number(stem));

    let title = clean_title(stem);

    ParsedFilename {
        title,
        year,
        quality,
        episode,
    }
}

/// Retourne `true` si l'item doit être marqué `numbering_unknown`
/// (requirement 4.7) : aucun motif d'épisode reconnu pour un fichier
/// rangé en catégorie `Séries` ou `Animes`. Pour les `Films`, la
/// numérotation n'a pas de sens et la fonction renvoie `false`.
pub fn is_numbering_unknown(category: Category, parsed: &ParsedFilename) -> bool {
    matches!(category, Category::Series | Category::Animes) && parsed.episode.is_none()
}

// ---------------------------------------------------------------------------
// Étapes du pipeline (publiques pour permettre des tests unitaires fins).
// ---------------------------------------------------------------------------

/// Liste des extensions qu'on consent à retirer au début du pipeline.
/// Reproduit `Supported_Video_Extensions` (requirements/Glossary).
const STRIPPABLE_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "avi", "mov", "webm", "m4v", "ts", "wmv",
];

/// Retire la dernière extension d'un nom de fichier si et seulement si
/// elle figure dans `STRIPPABLE_EXTENSIONS`. Cette restriction évite de
/// tronquer un suffixe légitime comme `1080p` ou un numéro `9.99`.
pub fn strip_extension(filename: &str) -> &str {
    match filename.rfind('.') {
        Some(idx) => {
            let ext = filename[idx + 1..].to_ascii_lowercase();
            if STRIPPABLE_EXTENSIONS.iter().any(|known| *known == ext) {
                &filename[..idx]
            } else {
                filename
            }
        }
        None => filename,
    }
}

/// Extrait la qualité (requirement 4.3). Insensible à la casse.
pub fn extract_quality(stem: &str) -> Option<Quality> {
    let token = quality_re().captures(stem)?.get(1)?.as_str();
    token.parse::<Quality>().ok()
}

/// Extrait l'année (requirement 4.2). Première occurrence ∈ [1900, 2099].
pub fn extract_year(stem: &str) -> Option<u16> {
    let captured = year_re().captures(stem)?.get(1)?.as_str();
    captured.parse::<u16>().ok()
}

/// Extrait un motif `S<saison>E<épisode>` (requirement 4.4).
pub fn extract_sxxexx(stem: &str) -> Option<EpisodeNumbering> {
    let caps = sxxexx_re().captures(stem)?;
    let season = caps.name("s")?.as_str().parse::<u16>().ok()?;
    let episode = caps.name("e")?.as_str().parse::<u16>().ok()?;
    Some(EpisodeNumbering {
        season,
        episode,
        source: NumberingSource::SxxExx,
    })
}

/// Extrait un motif `<saison>x<épisode>` (requirement 4.5).
pub fn extract_n_x_n(stem: &str) -> Option<EpisodeNumbering> {
    let caps = n_x_n_re().captures(stem)?;
    let season = caps.name("s")?.as_str().parse::<u16>().ok()?;
    let episode = caps.name("e")?.as_str().parse::<u16>().ok()?;
    Some(EpisodeNumbering {
        season,
        episode,
        source: NumberingSource::NxN,
    })
}

/// Extrait un motif souple `ép./ep./episode/-` suivi d'un numéro
/// (requirement 4.6). La saison est forcée à 1 par convention.
pub fn extract_loose_episode(stem: &str) -> Option<EpisodeNumbering> {
    let caps = loose_episode_re().captures(stem)?;
    let episode = caps.name("e")?.as_str().parse::<u16>().ok()?;
    Some(EpisodeNumbering {
        season: 1,
        episode,
        source: NumberingSource::EpOnly,
    })
}

/// Extrait un numéro nu en fin de stem, après suppression des autres
/// motifs reconnus. Fallback de dernière chance pour des conventions
/// type `Bleach Kai 042`, `Naruto 12 [Mixouille]`. Saison forcée à 1.
///
/// Pour limiter les faux positifs sur les titres contenant des chiffres
/// (« 2001 A Space Odyssey »), on n'applique cette regex que **sur le
/// stem dont les autres patterns ont déjà été retirés**. C'est le rôle
/// de `clean_title` qui le pré-nettoie avant cette extraction.
pub fn extract_trailing_number(stem: &str) -> Option<EpisodeNumbering> {
    // On nettoie d'abord le stem comme dans `clean_title` afin que les
    // tokens qualité / année / SxxExx déjà reconnus ne polluent pas la
    // détection. On ne réutilise pas `clean_title()` directement car
    // il transforme aussi les délimiteurs en espaces (utile pour
    // l'affichage, sans incidence ici).
    let mut work: String = quality_re().replace_all(stem, " ").into_owned();
    work = year_re().replace_all(&work, " ").into_owned();
    work = sxxexx_re().replace_all(&work, " ").into_owned();
    work = n_x_n_re().replace_all(&work, " ").into_owned();
    work = loose_episode_re().replace_all(&work, " ").into_owned();
    let work = work.trim();

    let caps = trailing_number_re().captures(work)?;
    let episode = caps.name("e")?.as_str().parse::<u16>().ok()?;
    // On rejette les numéros à 4 chiffres dans la plage 1900-2099
    // (probable année oubliée par `extract_year`) et les vraiment
    // énormes (au-delà de ~2000 épisodes c'est plus crédible que le
    // numéro fasse partie du titre).
    if (1900..=2099).contains(&episode) {
        return None;
    }
    if episode > 2000 {
        return None;
    }
    Some(EpisodeNumbering {
        season: 1,
        episode,
        source: NumberingSource::TrailingNumber,
    })
}

/// Construit le titre en retirant tous les segments reconnus puis en
/// remplaçant les délimiteurs `.`, `_`, `-` par des espaces (requirement
/// 4.1). Les espaces consécutifs sont collapsés et le résultat est trimé.
pub fn clean_title(stem: &str) -> String {
    // 1) Effacer chaque famille de segments reconnus en les remplaçant
    //    par un espace afin de ne jamais coller deux mots restants.
    let mut work: String = quality_re().replace_all(stem, " ").into_owned();
    work = year_re().replace_all(&work, " ").into_owned();
    work = sxxexx_re().replace_all(&work, " ").into_owned();
    work = n_x_n_re().replace_all(&work, " ").into_owned();
    work = loose_episode_re().replace_all(&work, " ").into_owned();

    // 2) Remplacer les délimiteurs explicitement listés par des espaces.
    let work: String = work
        .chars()
        .map(|c| match c {
            '.' | '_' | '-' => ' ',
            other => other,
        })
        .collect();

    // 3) Collapser la séquence d'espaces et trimer.
    let mut out = String::with_capacity(work.len());
    let mut last_was_space = true;
    for c in work.chars() {
        if c.is_whitespace() {
            if !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
        } else {
            out.push(c);
            last_was_space = false;
        }
    }

    out.trim().to_string()
}

// ---------------------------------------------------------------------------
// Tests unitaires de sanité (les tests de propriété arrivent en 3.2 / 3.3).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_extension_drops_known_extension() {
        assert_eq!(strip_extension("Movie.2010.1080p.mkv"), "Movie.2010.1080p");
        assert_eq!(strip_extension("Movie"), "Movie");
        // Extension non listée : on ne touche pas (pas une extension vidéo).
        assert_eq!(strip_extension("Movie.with.dots"), "Movie.with.dots");
    }

    #[test]
    fn strip_extension_is_case_insensitive() {
        assert_eq!(strip_extension("Show.S01E02.MKV"), "Show.S01E02");
        assert_eq!(strip_extension("Show.MP4"), "Show");
    }

    #[test]
    fn strip_extension_keeps_versions_without_extension() {
        // Pas d'extension réelle : "9.99" doit être préservé tel quel.
        assert_eq!(strip_extension("Title 9.99"), "Title 9.99");
    }

    #[test]
    fn parses_movie_with_year_and_quality() {
        let p = parse("Inception.2010.1080p.mkv");
        assert_eq!(p.title, "Inception");
        assert_eq!(p.year, Some(2010));
        assert_eq!(p.quality, Some(Quality::Q1080p));
        assert_eq!(p.episode, None);
    }

    #[test]
    fn parses_quality_4k_token() {
        let p = parse("Dune.2021.4K.mkv");
        assert_eq!(p.year, Some(2021));
        assert_eq!(p.quality, Some(Quality::Q4K));
    }

    #[test]
    fn parses_sxxexx_pattern_case_insensitive() {
        let p = parse("Breaking.Bad.s01e02.720p.mkv");
        assert_eq!(p.title, "Breaking Bad");
        assert_eq!(p.quality, Some(Quality::Q720p));
        assert_eq!(
            p.episode,
            Some(EpisodeNumbering {
                season: 1,
                episode: 2,
                source: NumberingSource::SxxExx,
            })
        );
    }

    #[test]
    fn parses_sxxexx_with_separator() {
        let p = parse("Show.S02.E08.mkv");
        assert_eq!(
            p.episode,
            Some(EpisodeNumbering {
                season: 2,
                episode: 8,
                source: NumberingSource::SxxExx,
            })
        );
    }

    #[test]
    fn parses_n_x_n_pattern() {
        let p = parse("Friends.1x05.avi");
        assert_eq!(p.title, "Friends");
        assert_eq!(
            p.episode,
            Some(EpisodeNumbering {
                season: 1,
                episode: 5,
                source: NumberingSource::NxN,
            })
        );
    }

    #[test]
    fn parses_loose_episode_with_dash() {
        let p = parse("Bleach - 250.mkv");
        assert_eq!(p.title, "Bleach");
        assert_eq!(
            p.episode,
            Some(EpisodeNumbering {
                season: 1,
                episode: 250,
                source: NumberingSource::EpOnly,
            })
        );
    }

    #[test]
    fn loose_episode_rejects_four_digit_numbers() {
        // La regex souple `loose_episode_re` borne `\d{1,3}` (cf. design).
        // Avec la regex de fallback `trailing_number_re` introduite
        // pour gérer les conventions release-group de longues séries
        // (« One Piece 1050 »), `parse` retourne désormais un
        // `EpisodeNumbering { season: 1, episode: 1050, source:
        // TrailingNumber }`. Ce test documente la transition : la
        // détection ne vient pas de la regex souple historique mais
        // du fallback trailing-number, qui rejette quand même les
        // années 1900-2099 et les nombres > 2000 pour limiter les
        // faux positifs.
        let p = parse("OnePiece - 1050.mkv");
        assert_eq!(
            p.episode,
            Some(EpisodeNumbering {
                season: 1,
                episode: 1050,
                source: NumberingSource::TrailingNumber,
            })
        );

        // Année 2010 reste rejetée car interceptée par la garde
        // « plage [1900, 2099] » du fallback trailing-number — c'est
        // probablement un titre / une année oubliée par `extract_year`.
        let p = parse("Inception 2010.mkv");
        assert_eq!(p.episode, None);
    }

    #[test]
    fn parses_loose_episode_with_french_marker() {
        let p = parse("Naruto.ép.12.mkv");
        assert_eq!(
            p.episode,
            Some(EpisodeNumbering {
                season: 1,
                episode: 12,
                source: NumberingSource::EpOnly,
            })
        );
    }

    #[test]
    fn parses_loose_episode_with_episode_word() {
        let p = parse("Naruto episode 42.mkv");
        assert_eq!(
            p.episode,
            Some(EpisodeNumbering {
                season: 1,
                episode: 42,
                source: NumberingSource::EpOnly,
            })
        );
    }

    #[test]
    fn sxxexx_takes_precedence_over_loose() {
        let p = parse("Show.S03E04 - 99.mkv");
        assert_eq!(
            p.episode,
            Some(EpisodeNumbering {
                season: 3,
                episode: 4,
                source: NumberingSource::SxxExx,
            })
        );
    }

    #[test]
    fn clean_title_strips_separators_and_segments() {
        let cleaned = clean_title("Inception.2010.1080p");
        assert_eq!(cleaned, "Inception");
    }

    #[test]
    fn clean_title_collapses_multiple_spaces() {
        let cleaned = clean_title("Some___Title-with--dots..2020");
        assert_eq!(cleaned, "Some Title with dots");
    }

    #[test]
    fn movie_without_numbering_has_no_episode() {
        let p = parse("Inception.2010.1080p.mkv");
        assert!(p.episode.is_none());
    }

    #[test]
    fn is_numbering_unknown_true_for_series_without_pattern() {
        let p = parse("MysteryShow.1080p.mkv");
        assert!(is_numbering_unknown(Category::Series, &p));
        assert!(is_numbering_unknown(Category::Animes, &p));
    }

    #[test]
    fn is_numbering_unknown_false_for_films() {
        let p = parse("MysteryShow.1080p.mkv");
        assert!(!is_numbering_unknown(Category::Films, &p));
    }

    #[test]
    fn is_numbering_unknown_false_when_pattern_detected() {
        let p = parse("Show.S01E02.mkv");
        assert!(!is_numbering_unknown(Category::Series, &p));
        assert!(!is_numbering_unknown(Category::Animes, &p));
    }
}

// ---------------------------------------------------------------------------
// Tests de propriété (tâches 3.2 et 3.3).
//
// Les générateurs sont conçus pour produire des noms de fichier sans
// ambiguïté regex :
//   * Le titre est restreint à des lettres ASCII et espaces simples (pas
//     de chiffres, pas de séparateurs `.` / `_` / `-`). Cela garantit
//     que ni `year_re`, ni `quality_re`, ni `sxxexx_re`, ni `n_x_n_re`,
//     ni `loose_episode_re` ne peuvent matcher à l'intérieur du titre
//     (toutes ces regex exigent au moins un chiffre, ou un marqueur
//     `ép./ep./episode/-` immédiatement suivi de chiffres).
//   * L'année est tirée dans [1900, 2099], ce qui correspond exactement à
//     l'intervalle reconnu par `extract_year`.
//   * La numérotation utilise les formats canoniques `S{ss}E{ee}`,
//     `{s}x{e}` ou `ep.{e}` ; ces formats sont reconstructibles à
//     l'identique après extraction.
//
// Les segments sont joints par `.` ce qui correspond au délimiteur
// nettoyé par `clean_title`. Aucune extension autre que `.mkv` n'est
// utilisée afin que `strip_extension` produise toujours le même `stem`.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    /// Titre arbitraire : 1 à 4 mots de 3 à 8 lettres ASCII séparés par
    /// un espace simple. Pas de chiffres, pas de séparateurs réservés.
    fn arb_title() -> impl Strategy<Value = String> {
        prop::collection::vec("[A-Z][a-z]{2,7}", 1usize..=4)
            .prop_map(|words| words.join(" "))
    }

    /// Année arbitraire dans la plage reconnue par `extract_year`.
    fn arb_year() -> impl Strategy<Value = u16> {
        1900u16..=2099u16
    }

    /// Qualité arbitraire couvrant les six variantes de l'enum.
    fn arb_quality() -> impl Strategy<Value = Quality> {
        prop_oneof![
            Just(Quality::Q480p),
            Just(Quality::Q720p),
            Just(Quality::Q1080p),
            Just(Quality::Q1440p),
            Just(Quality::Q2160p),
            Just(Quality::Q4K),
        ]
    }

    /// Numérotation arbitraire. Retourne le couple
    /// `(EpisodeNumbering attendu, segment textuel canonique)` afin que
    /// le test puisse construire le nom de fichier puis vérifier la
    /// reconstruction.
    ///
    /// Bornes : saison ∈ [1, 99] (regex `\d{1,2}`), épisode ∈ [1, 999]
    /// (regex `\d{1,3}`). Pour `EpOnly`, la saison attendue est forcée à
    /// 1 par convention (cf. `extract_loose_episode`).
    fn arb_numbering() -> impl Strategy<Value = (EpisodeNumbering, String)> {
        prop_oneof![
            (1u16..=99u16, 1u16..=999u16).prop_map(|(s, e)| (
                EpisodeNumbering {
                    season: s,
                    episode: e,
                    source: NumberingSource::SxxExx,
                },
                format!("S{:02}E{:02}", s, e),
            )),
            (1u16..=99u16, 1u16..=999u16).prop_map(|(s, e)| (
                EpisodeNumbering {
                    season: s,
                    episode: e,
                    source: NumberingSource::NxN,
                },
                format!("{}x{}", s, e),
            )),
            (1u16..=999u16).prop_map(|e| (
                EpisodeNumbering {
                    season: 1,
                    episode: e,
                    source: NumberingSource::EpOnly,
                },
                format!("ep.{}", e),
            )),
        ]
    }

    /// Catégorie arbitraire restreinte aux variantes pertinentes pour la
    /// règle `numbering_unknown` (Series, Animes).
    fn arb_series_or_anime() -> impl Strategy<Value = Category> {
        prop_oneof![Just(Category::Series), Just(Category::Animes)]
    }

    proptest! {
        /// **Property 6 : Round-trip parse / format des noms de fichier.**
        ///
        /// Pour un titre, une année optionnelle, une qualité optionnelle
        /// et une numérotation optionnelle générés selon les contraintes
        /// décrites en tête de module, le nom canonique
        /// `"{title}.{year}.{quality}.{numbering}.mkv"` (segments
        /// optionnels omis) doit être reparsé en `(title, year, quality,
        /// episode)` identique aux valeurs d'origine. Pour les motifs
        /// souples (`EpOnly`), la saison reconstruite est toujours 1.
        ///
        /// **Validates: Requirements 4.1, 4.2, 4.3, 4.4, 4.5, 4.6**
        #[test]
        fn property_6_round_trip_parse_format(
            title in arb_title(),
            year in prop::option::of(arb_year()),
            quality in prop::option::of(arb_quality()),
            numbering in prop::option::of(arb_numbering()),
        ) {
            let mut parts: Vec<String> = vec![title.clone()];
            if let Some(y) = year {
                parts.push(y.to_string());
            }
            if let Some(q) = quality {
                parts.push(q.as_str().to_string());
            }
            if let Some((_, ref segment)) = numbering {
                parts.push(segment.clone());
            }

            let filename = format!("{}.mkv", parts.join("."));
            let parsed = parse(&filename);

            prop_assert_eq!(&parsed.title, &title);
            prop_assert_eq!(parsed.year, year);
            prop_assert_eq!(parsed.quality, quality);
            let expected_episode = numbering.map(|(n, _)| n);
            prop_assert_eq!(parsed.episode, expected_episode);
        }

        /// **Property 7 : Marquage numérotation inconnue pour Series/Animes.**
        ///
        /// Pour un nom de fichier ne contenant aucun motif d'épisode
        /// reconnu (titre lettres + espaces, pas de chiffres susceptibles
        /// d'être interprétés comme un numéro d'épisode, plus
        /// éventuellement année ou qualité qui n'introduisent pas de
        /// motif d'épisode), `parse` doit renvoyer `episode = None` et
        /// `is_numbering_unknown` doit être vrai pour les catégories
        /// `Series` et `Animes`.
        ///
        /// **Validates: Requirements 4.7**
        #[test]
        fn property_7_numbering_unknown_for_series_and_animes(
            title in arb_title(),
            year in prop::option::of(arb_year()),
            quality in prop::option::of(arb_quality()),
            category in arb_series_or_anime(),
        ) {
            let mut parts: Vec<String> = vec![title];
            if let Some(y) = year {
                parts.push(y.to_string());
            }
            if let Some(q) = quality {
                parts.push(q.as_str().to_string());
            }
            let filename = format!("{}.mkv", parts.join("."));

            let parsed = parse(&filename);
            prop_assert!(parsed.episode.is_none());
            prop_assert!(is_numbering_unknown(category, &parsed));
        }
    }
}
