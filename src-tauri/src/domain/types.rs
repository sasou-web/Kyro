//! Types partagés du domaine pur Kyro.
//!
//! Ces types sont volontairement minimaux et indépendants de toute couche
//! d'infrastructure (DB, FS, Tauri). Ils sont consommés par le parser
//! (tâche 3.1), le scanner (tâche 8) et la couche repository (tâche 7).

use std::str::FromStr;

/// Catégorie d'un dossier de bibliothèque (requirement 2.5).
///
/// Les variantes correspondent exactement aux valeurs persistées en base
/// par la migration `0001_init.sql` (CHECK `category IN ('Films','Series','Animes')`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Films,
    Series,
    Animes,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Films => "Films",
            Category::Series => "Series",
            Category::Animes => "Animes",
        }
    }
}

/// Qualité vidéo détectée à partir du nom de fichier (requirement 4.3).
///
/// L'énumération couvre exactement les six valeurs reconnues par le
/// parser : `480p`, `720p`, `1080p`, `1440p`, `2160p`, `4K`. Le token
/// `4k` étant un alias usuel de `2160p`, le parser le distingue toutefois
/// en variante `Q4K` afin de préserver la valeur littérale extraite et
/// permettre un éventuel affichage fidèle au nom de fichier d'origine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Quality {
    Q480p,
    Q720p,
    Q1080p,
    Q1440p,
    Q2160p,
    Q4K,
}

impl Quality {
    /// Représentation canonique en minuscules, telle qu'elle peut être
    /// persistée dans `media_item.quality TEXT` (cf. design DDL).
    pub fn as_str(self) -> &'static str {
        match self {
            Quality::Q480p => "480p",
            Quality::Q720p => "720p",
            Quality::Q1080p => "1080p",
            Quality::Q1440p => "1440p",
            Quality::Q2160p => "2160p",
            Quality::Q4K => "4k",
        }
    }
}

impl FromStr for Quality {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "480p" => Ok(Quality::Q480p),
            "720p" => Ok(Quality::Q720p),
            "1080p" => Ok(Quality::Q1080p),
            "1440p" => Ok(Quality::Q1440p),
            "2160p" => Ok(Quality::Q2160p),
            "4k" => Ok(Quality::Q4K),
            _ => Err(()),
        }
    }
}

/// Source du motif de numérotation d'épisode reconnu par le parser.
///
/// - `SxxExx` : motif `S<saison>E<épisode>` (requirement 4.4).
/// - `NxN`    : motif `<saison>x<épisode>` (requirement 4.5).
/// - `EpOnly` : motifs souples `ép. <n>`, `ep <n>`, `episode <n>`,
///   `- <n>` ; la saison est forcée à 1 par défaut (requirement 4.6).
/// - `TrailingNumber` : numéro nu en fin de stem (`Bleach Kai 042`,
///   `Naruto 12`, `One Piece 1099 [final]`) ; saison forcée à 1.
///   Fallback de dernière chance utilisé avant de marquer
///   `numbering_unknown = true`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NumberingSource {
    SxxExx,
    NxN,
    EpOnly,
    TrailingNumber,
}


// ---------------------------------------------------------------------------
// Types Media_Item / MediaType / MediaItemId.
//
// Ces types sont utilisés par les modules de domaine `filters` (tâche 5.1)
// et `search` (tâche 5.2). Ils correspondent à la projection minimale du
// `media_item` SQL nécessaire pour ces deux modules ; la couche repo
// (tâche 7.x) matérialisera la fiche complète depuis SQLite et la
// projettera vers cette vue avant les appels.
// ---------------------------------------------------------------------------

/// Type d'un Media_Item, miroir du CHECK SQL
/// `type IN ('Film','Series','Anime')` (cf. migration 0001_init.sql).
///
/// Distinct de [`Category`] qui s'applique aux dossiers de bibliothèque
/// et utilise les variantes pluriels (`Films`, `Series`, `Animes`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaType {
    Film,
    Series,
    Anime,
}

impl MediaType {
    /// Représentation textuelle stable, identique aux littéraux acceptés
    /// par le `CHECK (type IN ('Film','Series','Anime'))` du DDL
    /// `media_item` (migration `0001_init.sql`).
    pub fn as_str(self) -> &'static str {
        match self {
            MediaType::Film => "Film",
            MediaType::Series => "Series",
            MediaType::Anime => "Anime",
        }
    }
}

/// Identifiant d'un Media_Item (clé primaire SQLite, `INTEGER`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MediaItemId(pub i64);

impl MediaItemId {
    /// Accès à l'`i64` sous-jacent pour les requêtes paramétrées.
    pub fn get(self) -> i64 {
        self.0
    }
}

/// Identifiant d'un Episode_Item (clé primaire SQLite, `INTEGER`).
///
/// Wrappe le `INTEGER PRIMARY KEY AUTOINCREMENT` de la table
/// `episode_item` afin d'éviter toute confusion avec [`MediaItemId`]
/// ou [`crate::infra::repo::FolderId`] dans les signatures publiques
/// de la couche `infra::repo` (tâche 7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EpisodeItemId(pub i64);

impl EpisodeItemId {
    /// Accès à l'`i64` sous-jacent pour les requêtes paramétrées.
    pub fn get(self) -> i64 {
        self.0
    }
}

/// Vue minimale d'un Media_Item utilisée par les fonctions pures de
/// `domain::filters` et `domain::search`. La couche repo construit cette
/// projection à partir de la ligne `media_item` complète et la passe aux
/// fonctions du domaine, ce qui garde ce dernier indépendant de `sqlx`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaItem {
    pub id: MediaItemId,
    pub title: String,
    pub original_title: Option<String>,
    pub year: Option<u16>,
    pub media_type: MediaType,
    pub status: crate::domain::status::Status,
    pub genres: Vec<String>,
    /// Chemin absolu local de l'affiche importée (cf. tâche 14.7).
    /// `None` si aucune affiche n'a été associée — le frontend rend
    /// alors un placeholder SVG inline (cf. `MediaCard.svelte`).
    pub poster_path: Option<String>,
}

/// Branchement de la vue `MediaItem` sur le module `domain::search` :
/// permet à `search::matches(item, query)` de filtrer une bibliothèque
/// chargée depuis SQLite (cf. `infra::repo::Repo::search`, tâche 7.4)
/// sans que le repo n'ait à ré-implémenter la sémantique de
/// normalisation Unicode (déjà couverte par les propriétés P-28 / P-29).
impl crate::domain::search::Searchable for MediaItem {
    fn title(&self) -> &str {
        &self.title
    }

    fn original_title(&self) -> Option<&str> {
        self.original_title.as_deref()
    }
}
