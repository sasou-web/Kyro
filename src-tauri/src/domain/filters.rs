//! Filtrage de la bibliothèque par genre, année, type et statut.
//!
//! Module pur (aucune IO) couvrant Requirement 14.
//!
//! Sémantique formelle (Requirement 14.2, 14.3) : un Media_Item passe
//! le filtre si et seulement si toutes les dimensions ci-dessous sont
//! satisfaites simultanément. Une dimension dont la liste est vide ne
//! contraint rien — c'est l'identité (Requirement 14.4).
//!
//! ```text
//! keep(item) ⇔
//!     (genres   = ∅ ∨ item.genres ∩ genres ≠ ∅)   // union sur les genres
//!   ∧ (years    = ∅ ∨ item.year ∈ years)          // intersection sur l'année
//!   ∧ (types    = ∅ ∨ item.media_type ∈ types)    // intersection sur le type
//!   ∧ (statuses = ∅ ∨ item.status ∈ statuses)     // intersection sur le statut
//! ```
//!
//! Les tests de propriété (5.5, 5.6) valident cette formule sur des
//! bibliothèques arbitraires. Cette tâche fournit uniquement la
//! fonction et des tests unitaires d'exemples ciblés (un par clause).

use crate::domain::status::Status;
use crate::domain::types::{MediaItem, MediaType};

/// Critères de filtrage applicables à une bibliothèque.
///
/// Conventions :
/// - `genres` est une union (au moins un genre commun suffit) — 14.3.
/// - `years`, `types`, `statuses` sont des appartenances ensemblistes
///   (chaque dimension restreint indépendamment).
/// - Une liste vide signifie « pas de contrainte sur cette dimension »
///   (14.4).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filters {
    /// Genres sélectionnés (union — Requirement 14.3).
    pub genres: Vec<String>,
    /// Années sélectionnées (intersection ensembliste — Requirement 14.1).
    pub years: Vec<u16>,
    /// Types sélectionnés (intersection ensembliste — Requirement 14.1).
    pub types: Vec<MediaType>,
    /// Statuts sélectionnés (intersection ensembliste — Requirement 14.1).
    pub statuses: Vec<Status>,
}

/// Applique le quadruplet de filtres à une bibliothèque arbitraire et
/// retourne la liste filtrée dans l'ordre d'origine.
///
/// La fonction est pure : aucune IO, complexité O(|items| · k) où k est
/// la taille agrégée des listes de filtres. Pour des bibliothèques
/// jusqu'à 5 000 items (Requirement 13.3) cette implémentation reste
/// largement sous le budget de re-filtrage cible.
///
/// Couvre Requirements 14.1, 14.2, 14.3, 14.4. Le compteur visible
/// associé (Requirement 14.5) est trivialement `result.len()` pour
/// l'appelant — il n'y a donc rien à exposer de plus côté domaine.
pub fn apply_filters(items: &[MediaItem], filters: &Filters) -> Vec<MediaItem> {
    items
        .iter()
        .filter(|item| matches(item, filters))
        .cloned()
        .collect()
}

/// Prédicat unitaire utilisé par `apply_filters`. Exposé `pub(crate)`
/// pour que les tests de propriété (5.5) puissent l'utiliser
/// directement, ce qui garde la PBT focalisée sur la formule sans
/// recopier la sémantique.
pub(crate) fn matches(item: &MediaItem, filters: &Filters) -> bool {
    matches_genres(item, &filters.genres)
        && matches_years(item, &filters.years)
        && matches_types(item, &filters.types)
        && matches_statuses(item, &filters.statuses)
}

/// Clause genre — union (intersection non vide). Une liste vide est
/// l'identité (Requirement 14.4) ; sinon, l'item passe ssi au moins un
/// de ses genres figure parmi ceux sélectionnés (Requirement 14.3).
fn matches_genres(item: &MediaItem, selected: &[String]) -> bool {
    if selected.is_empty() {
        return true;
    }
    item.genres.iter().any(|g| selected.iter().any(|s| s == g))
}

/// Clause année — intersection ensembliste. Une liste vide est
/// l'identité ; sinon l'année doit être connue et appartenir à la
/// sélection.
fn matches_years(item: &MediaItem, selected: &[u16]) -> bool {
    if selected.is_empty() {
        return true;
    }
    match item.year {
        Some(y) => selected.contains(&y),
        None => false,
    }
}

/// Clause type — intersection ensembliste.
fn matches_types(item: &MediaItem, selected: &[MediaType]) -> bool {
    if selected.is_empty() {
        return true;
    }
    selected.contains(&item.media_type)
}

/// Clause statut — intersection ensembliste.
fn matches_statuses(item: &MediaItem, selected: &[Status]) -> bool {
    if selected.is_empty() {
        return true;
    }
    selected.contains(&item.status)
}

#[cfg(test)]
mod tests {
    //! Tests unitaires d'exemples : une assertion par clause de la
    //! formule afin d'illustrer chaque branche de la sémantique.
    //! Les tests de propriété correspondants sont 5.5 (conjonction et
    //! union) et 5.6 (filtres vides = identité).

    use super::*;

    fn make(
        id: i64,
        media_type: MediaType,
        year: Option<u16>,
        status: Status,
        genres: &[&str],
    ) -> MediaItem {
        MediaItem {
            id: crate::domain::types::MediaItemId(id),
            title: format!("Item {id}"),
            original_title: None,
            year,
            media_type,
            status,
            genres: genres.iter().map(|s| (*s).to_string()).collect(),
            poster_path: None,
        }
    }

    fn fixture() -> Vec<MediaItem> {
        vec![
            make(
                1,
                MediaType::Film,
                Some(1999),
                Status::Vu,
                &["Action", "Sci-Fi"],
            ),
            make(2, MediaType::Series, Some(2008), Status::EnCours, &["Drame"]),
            make(
                3,
                MediaType::Anime,
                Some(2019),
                Status::NonVu,
                &["Action", "Aventure"],
            ),
            make(4, MediaType::Film, None, Status::NonVu, &["Comédie"]),
        ]
    }

    /// Filtres tous vides ⇒ identité (Requirement 14.4).
    #[test]
    fn empty_filters_return_full_library() {
        let items = fixture();
        let result = apply_filters(&items, &Filters::default());
        assert_eq!(result, items);
    }

    /// Filtre genre — union (Requirement 14.3) : un item passe dès
    /// qu'au moins un de ses genres correspond.
    #[test]
    fn genres_apply_as_union() {
        let items = fixture();
        let filters = Filters {
            genres: vec!["Action".into()],
            ..Default::default()
        };
        let ids: Vec<i64> = apply_filters(&items, &filters)
            .iter()
            .map(|i| i.id.0)
            .collect();
        // Items 1 et 3 ont le genre "Action".
        assert_eq!(ids, vec![1, 3]);
    }

    /// Plusieurs genres sélectionnés ⇒ union (intersection non vide
    /// suffit), pas intersection.
    #[test]
    fn multiple_genres_widen_the_selection() {
        let items = fixture();
        let filters = Filters {
            genres: vec!["Drame".into(), "Comédie".into()],
            ..Default::default()
        };
        let ids: Vec<i64> = apply_filters(&items, &filters)
            .iter()
            .map(|i| i.id.0)
            .collect();
        assert_eq!(ids, vec![2, 4]);
    }

    /// Filtre année — intersection ensembliste.
    #[test]
    fn years_apply_as_set_membership() {
        let items = fixture();
        let filters = Filters {
            years: vec![1999, 2019],
            ..Default::default()
        };
        let ids: Vec<i64> = apply_filters(&items, &filters)
            .iter()
            .map(|i| i.id.0)
            .collect();
        assert_eq!(ids, vec![1, 3]);
    }

    /// Item sans année ⇒ exclu dès qu'un filtre année est actif.
    #[test]
    fn unknown_year_is_excluded_when_year_filter_active() {
        let items = fixture();
        let filters = Filters {
            years: vec![2024],
            ..Default::default()
        };
        assert!(apply_filters(&items, &filters).is_empty());
    }

    /// Filtre type — intersection ensembliste (Requirement 14.1).
    #[test]
    fn types_apply_as_set_membership() {
        let items = fixture();
        let filters = Filters {
            types: vec![MediaType::Film],
            ..Default::default()
        };
        let ids: Vec<i64> = apply_filters(&items, &filters)
            .iter()
            .map(|i| i.id.0)
            .collect();
        assert_eq!(ids, vec![1, 4]);
    }

    /// Filtre statut — intersection ensembliste (Requirement 14.1).
    #[test]
    fn statuses_apply_as_set_membership() {
        let items = fixture();
        let filters = Filters {
            statuses: vec![Status::NonVu],
            ..Default::default()
        };
        let ids: Vec<i64> = apply_filters(&items, &filters)
            .iter()
            .map(|i| i.id.0)
            .collect();
        assert_eq!(ids, vec![3, 4]);
    }

    /// Conjonction des dimensions (Requirement 14.2) : un item ne
    /// passe que s'il satisfait simultanément toutes les contraintes.
    #[test]
    fn dimensions_compose_as_conjunction() {
        let items = fixture();
        let filters = Filters {
            genres: vec!["Action".into()],
            years: vec![1999, 2019],
            types: vec![MediaType::Anime],
            statuses: vec![Status::NonVu],
        };
        let ids: Vec<i64> = apply_filters(&items, &filters)
            .iter()
            .map(|i| i.id.0)
            .collect();
        // Seul l'item 3 (Anime, 2019, Action, NonVu) satisfait
        // simultanément les quatre dimensions.
        assert_eq!(ids, vec![3]);
    }

    /// Une dimension vide n'introduit aucune contrainte même si
    /// d'autres sont actives (Requirement 14.4).
    #[test]
    fn empty_dimension_does_not_constrain_other_active_dimensions() {
        let items = fixture();
        let filters = Filters {
            // genres = ∅ ⇒ aucune contrainte sur les genres
            types: vec![MediaType::Film],
            ..Default::default()
        };
        let ids: Vec<i64> = apply_filters(&items, &filters)
            .iter()
            .map(|i| i.id.0)
            .collect();
        assert_eq!(ids, vec![1, 4]);
    }

    /// Sanity check du compteur (Requirement 14.5) : la longueur du
    /// résultat correspond au nombre d'items visibles.
    #[test]
    fn visible_count_matches_result_length() {
        let items = fixture();
        let filters = Filters {
            statuses: vec![Status::Vu, Status::EnCours],
            ..Default::default()
        };
        let result = apply_filters(&items, &filters);
        assert_eq!(result.len(), 2);
    }
}

#[cfg(test)]
mod proptests {
    //! Tests de propriété pour `domain::filters`.
    //!
    //! Couvre :
    //! - **Property 30 (tâche 5.5)** — la sortie de `apply_filters`
    //!   coïncide avec la formule à quatre clauses (conjonction +
    //!   union des genres) du Requirement 14.2/14.3.
    //! - **Property 31 (tâche 5.6)** — tous filtres vides ⇒ identité
    //!   sur la bibliothèque, et le compteur visible vaut `card(L)`
    //!   (Requirement 14.4/14.5).
    //!
    //! Les générateurs sont délibérément bornés (genres dans un petit
    //! alphabet, années dans `[1950, 2030]`, listes ≤ 5 / 20) afin
    //! que la PBT explore des intersections non triviales tout en
    //! restant rapide à exécuter sous `cargo test`.

    use super::*;
    use crate::domain::status::Status;
    use crate::domain::types::{MediaItem, MediaItemId, MediaType};
    use proptest::collection::vec;
    use proptest::option;
    use proptest::prelude::*;

    /// Alphabet restreint de genres. La probabilité que deux items
    /// partagent un genre — et donc que la clause d'union soit
    /// réellement testée — reste élevée à cette taille.
    const GENRE_ALPHABET: &[&str] = &[
        "Action", "Aventure", "Comédie", "Drame", "Sci-Fi", "Horreur",
    ];

    /// Type de média uniformément choisi parmi les trois variantes.
    fn arb_media_type() -> impl Strategy<Value = MediaType> {
        prop_oneof![
            Just(MediaType::Film),
            Just(MediaType::Series),
            Just(MediaType::Anime),
        ]
    }

    /// Statut uniformément choisi parmi les trois variantes.
    fn arb_status() -> impl Strategy<Value = Status> {
        prop_oneof![
            Just(Status::NonVu),
            Just(Status::EnCours),
            Just(Status::Vu),
        ]
    }

    /// Genre tiré dans `GENRE_ALPHABET`.
    fn arb_genre() -> impl Strategy<Value = String> {
        proptest::sample::select(GENRE_ALPHABET).prop_map(|s| s.to_string())
    }

    /// Année dans une plage plausible côté domaine. Couvre les bornes
    /// de validation côté frontend (`1900..=2099`) sans s'aventurer
    /// dans des extrémités peu utiles pour la PBT.
    fn arb_year() -> impl Strategy<Value = u16> {
        1950u16..=2030u16
    }

    /// `MediaItem` arbitraire — projection minimale utilisée par
    /// `apply_filters` (id, year, media_type, status, genres). Les
    /// champs `title` / `original_title` ne participent pas au
    /// filtrage et restent constants / `None` afin de focaliser la
    /// PBT sur les quatre dimensions du Requirement 14.
    fn arb_media_item() -> impl Strategy<Value = MediaItem> {
        (
            any::<i64>(),
            option::of(arb_year()),
            arb_media_type(),
            arb_status(),
            vec(arb_genre(), 0..=4),
        )
            .prop_map(|(id, year, media_type, status, genres)| MediaItem {
                id: MediaItemId(id),
                title: format!("Item {id}"),
                original_title: None,
                year,
                media_type,
                status,
                genres,
                poster_path: None,
            })
    }

    /// Bibliothèque arbitraire (≤ 20 items) — taille suffisante pour
    /// stresser la composition des filtres sans pénaliser le run-time.
    fn arb_library() -> impl Strategy<Value = Vec<MediaItem>> {
        vec(arb_media_item(), 0..=20)
    }

    /// Quadruplet de filtres arbitraires — chaque dimension est bornée
    /// à 5 valeurs (Requirement 14.1 : ensembles finis sélectionnés
    /// côté UI) afin de garder un espace de recherche raisonnable.
    fn arb_filters() -> impl Strategy<Value = Filters> {
        (
            vec(arb_genre(), 0..=5),
            vec(arb_year(), 0..=5),
            vec(arb_media_type(), 0..=5),
            vec(arb_status(), 0..=5),
        )
            .prop_map(|(genres, years, types, statuses)| Filters {
                genres,
                years,
                types,
                statuses,
            })
    }

    /// Implémentation de référence, dérivée littéralement de la
    /// formule du design (P-30) :
    ///
    /// ```text
    /// keep ⇔ (genres = ∅ ∨ item.genres ∩ genres ≠ ∅)
    ///     ∧ (years   = ∅ ∨ item.year ∈ years)
    ///     ∧ (types   = ∅ ∨ item.media_type ∈ types)
    ///     ∧ (statuses = ∅ ∨ item.status ∈ statuses)
    /// ```
    ///
    /// Volontairement réécrite ici sans appeler `super::matches`,
    /// pour que le test compare deux implémentations indépendantes
    /// de la même spécification.
    fn reference_predicate(item: &MediaItem, filters: &Filters) -> bool {
        let g_clause = filters.genres.is_empty()
            || item.genres.iter().any(|g| filters.genres.contains(g));
        let y_clause = filters.years.is_empty()
            || item
                .year
                .map(|y| filters.years.contains(&y))
                .unwrap_or(false);
        let t_clause =
            filters.types.is_empty() || filters.types.contains(&item.media_type);
        let s_clause =
            filters.statuses.is_empty() || filters.statuses.contains(&item.status);
        g_clause && y_clause && t_clause && s_clause
    }

    proptest! {
        /// **Property 30 (P-30) — Conjonction et union des filtres.**
        ///
        /// Pour toute bibliothèque `L` et tout quadruplet de filtres
        /// `F = (genres, years, types, statuses)`, le résultat de
        /// `apply_filters(L, F)` est exactement la sous-séquence de
        /// `L`, dans l'ordre, des items satisfaisant la conjonction
        /// des quatre clauses (avec union ensembliste sur les genres).
        ///
        /// **Validates: Requirements 14.2, 14.3**
        #[test]
        fn p30_apply_filters_matches_reference_formula(
            items in arb_library(),
            filters in arb_filters(),
        ) {
            let actual = apply_filters(&items, &filters);
            let expected: Vec<MediaItem> = items
                .iter()
                .filter(|item| reference_predicate(item, &filters))
                .cloned()
                .collect();
            prop_assert_eq!(actual, expected);
        }

        /// **Property 31 (P-31) — Filtres vides = identité.**
        ///
        /// Pour toute bibliothèque `L`, `apply_filters(L, ∅⁴)` rend
        /// `L` complète (Requirement 14.4) et son cardinal est égal
        /// au nombre d'items visibles annoncé par le compteur de
        /// l'UI (Requirement 14.5 : `compteur = result.len()`).
        ///
        /// **Validates: Requirements 14.4, 14.5**
        #[test]
        fn p31_empty_filters_return_full_library_and_full_count(
            items in arb_library(),
        ) {
            let result = apply_filters(&items, &Filters::default());
            // Requirement 14.5 — compteur visible = card(L).
            prop_assert_eq!(result.len(), items.len());
            // Requirement 14.4 — identité ensembliste et ordonnée.
            prop_assert_eq!(result, items);
        }
    }
}
