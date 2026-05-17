//! Calcul du prochain Episode_Item à reprendre pour une série / un anime.
//!
//! Module pur (aucune IO, aucune dépendance Tauri ou sqlx) — destiné à
//! être appelé par `repo::next_resume_episode` (tâche 7.4) après avoir
//! chargé les épisodes d'une série depuis SQLite.
//!
//! Couvre les exigences EARS suivantes :
//!
//! * **Requirement 12.1** — priorité 1 : premier épisode `en_cours`
//!   selon l'ordre `(season_number, episode_number)` ;
//!   priorité 2 : premier épisode `non_vu` selon le même ordre.
//! * **Requirement 12.2** — l'épisode mémorisé est exposé via le bouton
//!   `Reprendre` côté frontend (couvert ici par le retour
//!   `ResumeAction::Resume`).
//! * **Requirement 12.3** — si tous les épisodes disponibles sont `vu`,
//!   le bouton devient `Revoir depuis le début` (retour
//!   `ResumeAction::RewatchFromStart(first)`).
//!
//! Voir le pseudocode du design § « Calcul prochain épisode ».

use crate::domain::status::Status;

/// Vue minimale d'un Episode_Item utilisée par `next_resume`.
///
/// Cette structure est volontairement réduite aux seuls champs requis
/// par l'algorithme : la couche repo (tâche 7.x) projette son
/// `EpisodeItem` complet (titres, codecs, durée, watch_progress…) vers
/// cette vue avant l'appel. Cela garde le domaine pur testable sans
/// matérialiser toute la fiche, et évite un cycle entre `domain` et
/// `infra::repo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpisodeItem {
    /// Identifiant SQLite (`episode_item.id`). Permet à l'appelant de
    /// retrouver la ligne complète après réception du résultat.
    pub id: i64,
    /// Numéro de saison (1-based, `episode_item.season_number`).
    pub season_number: u16,
    /// Numéro d'épisode dans la saison (1-based,
    /// `episode_item.episode_number`).
    pub episode_number: u16,
    /// Statut de visionnage courant — déterminé par
    /// `domain::status::compute_status` (tâche 4.1).
    pub status: Status,
    /// `true` si le fichier sous-jacent existe encore (booléen
    /// `episode_item.available`). Les épisodes indisponibles sont
    /// ignorés par `next_resume` (cf. requirement 3.5 + 12.1).
    pub available: bool,
}

/// Action proposée à l'utilisateur sur la fiche d'une série / d'un
/// anime, calculée à partir de la liste de ses épisodes.
///
/// Voir requirement 12.2 / 12.3 pour les libellés frontend
/// correspondants (« Reprendre » / « Revoir depuis le début »).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeAction {
    /// Reprendre la lecture sur cet épisode (priorité `en_cours` puis
    /// `non_vu`, requirement 12.1).
    Resume(EpisodeItem),
    /// Tous les épisodes disponibles sont `vu` — proposer de revoir
    /// depuis le premier épisode disponible (requirement 12.3). `None`
    /// uniquement si la série n'a aucun épisode disponible (cas
    /// dégénéré ; en pratique `next_resume` retourne alors la variante
    /// `None` ci-dessous).
    RewatchFromStart(Option<EpisodeItem>),
    /// Aucune action proposable : la série n'a aucun épisode
    /// disponible (tous marqués `available = false`, ou liste vide).
    None,
}

/// Calcule l'action `Reprendre` / `Revoir depuis le début` pour la liste
/// des épisodes d'une série ou d'un anime.
///
/// Algorithme (cf. design § « Calcul prochain épisode ») :
///
/// 1. Filtrer les épisodes `available = true`.
/// 2. Trier par `(season_number, episode_number)` croissant.
/// 3. Si la liste filtrée est vide → `ResumeAction::None`.
/// 4. Si **tous** les épisodes restants sont `Status::Vu` →
///    `RewatchFromStart(first)` (requirement 12.3).
/// 5. Sinon, retourner `Resume(e)` où `e` est :
///    a. le premier épisode `Status::EnCours` selon l'ordre, ou à défaut
///    b. le premier épisode `Status::NonVu` selon l'ordre
///    (requirement 12.1).
pub fn next_resume(episodes: &[EpisodeItem]) -> ResumeAction {
    // 1 + 2. Filtrage des disponibles puis tri stable par
    // (saison, épisode). On manipule des références pour éviter de
    // cloner des EpisodeItem au cours du tri.
    let mut sorted: Vec<&EpisodeItem> =
        episodes.iter().filter(|e| e.available).collect();
    sorted.sort_by_key(|e| (e.season_number, e.episode_number));

    // 3. Aucune entrée disponible : pas de bouton à proposer.
    if sorted.is_empty() {
        return ResumeAction::None;
    }

    // 4. Tous vus → revoir depuis le début (requirement 12.3).
    if sorted.iter().all(|e| e.status == Status::Vu) {
        return ResumeAction::RewatchFromStart(sorted.first().map(|e| (*e).clone()));
    }

    // 5a. Premier `en_cours` selon l'ordre (requirement 12.1, priorité 1).
    if let Some(in_progress) = sorted.iter().find(|e| e.status == Status::EnCours) {
        return ResumeAction::Resume((*in_progress).clone());
    }

    // 5b. Sinon, premier `non_vu` selon l'ordre (requirement 12.1, priorité 2).
    if let Some(first_unseen) = sorted.iter().find(|e| e.status == Status::NonVu) {
        return ResumeAction::Resume((*first_unseen).clone());
    }

    // Branche inatteignable : si la liste filtrée n'est pas vide et
    // n'est pas entièrement `Vu`, alors elle contient au moins un
    // `EnCours` ou un `NonVu`. On la conserve par défensivité plutôt
    // que de paniquer.
    ResumeAction::None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper de fabrique : construit un `EpisodeItem` avec uniquement
    /// les champs pertinents pour le tri / la sélection.
    fn ep(id: i64, season: u16, episode: u16, status: Status, available: bool) -> EpisodeItem {
        EpisodeItem {
            id,
            season_number: season,
            episode_number: episode,
            status,
            available,
        }
    }

    // ---------------------------------------------------------------
    // Branche 1 : priorité au premier `EnCours` selon l'ordre saison /
    // épisode (requirement 12.1, priorité 1).
    // ---------------------------------------------------------------

    #[test]
    fn returns_resume_on_first_in_progress_in_season_episode_order() {
        // Liste volontairement désordonnée pour valider le tri
        // (season_number, episode_number) avant la sélection.
        let episodes = vec![
            ep(10, 1, 3, Status::Vu, true),
            ep(11, 2, 1, Status::EnCours, true), // viendra en 4ᵉ position
            ep(12, 1, 1, Status::Vu, true),
            ep(13, 1, 2, Status::EnCours, true), // <-- attendu : premier EnCours
            ep(14, 1, 4, Status::NonVu, true),
        ];

        match next_resume(&episodes) {
            ResumeAction::Resume(e) => {
                assert_eq!(e.id, 13, "doit cibler S01E02 (premier EnCours dans l'ordre)");
                assert_eq!(e.season_number, 1);
                assert_eq!(e.episode_number, 2);
                assert_eq!(e.status, Status::EnCours);
            }
            other => panic!("attendu Resume(EnCours), reçu {other:?}"),
        }
    }

    // ---------------------------------------------------------------
    // Branche 2 : priorité au premier `NonVu` lorsqu'aucun épisode
    // n'est `EnCours` (requirement 12.1, priorité 2).
    // ---------------------------------------------------------------

    #[test]
    fn returns_resume_on_first_unseen_when_no_in_progress() {
        let episodes = vec![
            ep(20, 1, 2, Status::NonVu, true), // <-- attendu : premier NonVu
            ep(21, 1, 1, Status::Vu, true),
            ep(22, 1, 3, Status::NonVu, true),
            ep(23, 2, 1, Status::NonVu, true),
        ];

        match next_resume(&episodes) {
            ResumeAction::Resume(e) => {
                assert_eq!(e.id, 20, "doit cibler S01E02 (premier NonVu après tri)");
                assert_eq!((e.season_number, e.episode_number), (1, 2));
                assert_eq!(e.status, Status::NonVu);
            }
            other => panic!("attendu Resume(NonVu), reçu {other:?}"),
        }
    }

    // ---------------------------------------------------------------
    // Branche 3 : tous les disponibles sont `Vu` → RewatchFromStart
    // (requirement 12.3).
    // ---------------------------------------------------------------

    #[test]
    fn returns_rewatch_from_start_when_all_available_episodes_are_seen() {
        let episodes = vec![
            ep(30, 2, 1, Status::Vu, true),
            ep(31, 1, 1, Status::Vu, true), // <-- attendu : premier après tri
            ep(32, 1, 2, Status::Vu, true),
        ];

        match next_resume(&episodes) {
            ResumeAction::RewatchFromStart(Some(first)) => {
                assert_eq!(first.id, 31, "doit pointer le tout premier S01E01");
                assert_eq!((first.season_number, first.episode_number), (1, 1));
            }
            other => panic!("attendu RewatchFromStart(Some), reçu {other:?}"),
        }
    }

    // ---------------------------------------------------------------
    // Cas de bordure : seuls les épisodes `available = true` comptent
    // (un épisode indisponible `EnCours` ne doit pas être proposé).
    // ---------------------------------------------------------------

    #[test]
    fn unavailable_episodes_are_ignored() {
        let episodes = vec![
            // Indisponible — doit être totalement ignoré, même s'il
            // serait sinon le candidat naturel.
            ep(40, 1, 1, Status::EnCours, false),
            ep(41, 1, 2, Status::NonVu, true), // <-- attendu
            ep(42, 1, 3, Status::NonVu, true),
        ];

        match next_resume(&episodes) {
            ResumeAction::Resume(e) => assert_eq!(e.id, 41),
            other => panic!("attendu Resume sur l'épisode disponible, reçu {other:?}"),
        }
    }

    #[test]
    fn empty_input_returns_none() {
        let episodes: Vec<EpisodeItem> = Vec::new();
        assert_eq!(next_resume(&episodes), ResumeAction::None);
    }

    #[test]
    fn all_unavailable_returns_none() {
        let episodes = vec![
            ep(50, 1, 1, Status::Vu, false),
            ep(51, 1, 2, Status::EnCours, false),
        ];
        assert_eq!(next_resume(&episodes), ResumeAction::None);
    }

    #[test]
    fn in_progress_takes_priority_over_unseen_even_if_unseen_comes_first() {
        // S01E01 est NonVu et précède S01E02 (EnCours) dans l'ordre.
        // Le requirement 12.1 priorise pourtant `EnCours` sur `NonVu`,
        // indépendamment de l'ordre.
        let episodes = vec![
            ep(60, 1, 1, Status::NonVu, true),
            ep(61, 1, 2, Status::EnCours, true),
        ];

        match next_resume(&episodes) {
            ResumeAction::Resume(e) => {
                assert_eq!(e.id, 61, "EnCours doit primer sur NonVu (12.1)");
                assert_eq!(e.status, Status::EnCours);
            }
            other => panic!("attendu Resume(EnCours), reçu {other:?}"),
        }
    }
}

#[cfg(test)]
mod proptests {
    //! Tests de propriétés couvrant `next_resume`.
    //!
    //! Ces tests utilisent `proptest` (cf. `dev-dependencies` du
    //! `Cargo.toml`) et sont isolés dans un sous-module dédié pour ne
    //! pas alourdir le module `tests` exemple-par-exemple ci-dessus.
    //!
    //! Chaque propriété est numérotée selon le tableau du design
    //! (« 37 propriétés ») et annonce explicitement les exigences
    //! qu'elle valide.

    use super::*;
    use proptest::collection::hash_set;
    use proptest::prelude::*;
    use std::collections::HashSet;

    /// Bornes du générateur — calibrées sur des séries / animes
    /// réalistes (5 saisons × 20 épisodes = 100 cellules) tout en
    /// laissant suffisamment d'« espace » à proptest pour produire
    /// des combinaisons variées de statuts et de disponibilités.
    /// La taille de la liste est plafonnée à 15 pour garder un
    /// coût raisonnable sur les 256 cas par défaut de `proptest`.
    const MAX_SEASON: u16 = 5;
    const MAX_EPISODE: u16 = 20;
    const MAX_LIST_LEN: usize = 15;

    /// Générateur de `Status` uniforme sur les trois variantes —
    /// nécessaire pour exercer chacune des trois branches de
    /// `next_resume` (RewatchFromStart, Resume(EnCours), Resume(NonVu)).
    fn arb_status() -> impl Strategy<Value = Status> {
        prop_oneof![
            Just(Status::NonVu),
            Just(Status::EnCours),
            Just(Status::Vu),
        ]
    }

    /// Générateur de `Vec<EpisodeItem>` de taille `0..=MAX_LIST_LEN`
    /// avec des paires `(season_number, episode_number)` **uniques**.
    ///
    /// L'unicité est obtenue en générant d'abord un `HashSet` de
    /// paires (les valeurs en double sont éliminées par construction
    /// — cf. consigne « générer des clés puis dédupliquer »), puis
    /// en attachant à chaque paire un `(status, available)` tiré
    /// indépendamment via `prop_flat_map`.
    ///
    /// L'absence de doublons `(season, episode)` empêche les
    /// situations dégénérées où deux épisodes partageraient la même
    /// clé de tri : `next_resume` reposant sur `sort_by_key`, l'ordre
    /// relatif d'éléments à clé égale dépendrait alors de l'ordre
    /// d'itération du `HashSet` et rendrait le test non déterministe.
    fn arb_episode_list() -> impl Strategy<Value = Vec<EpisodeItem>> {
        hash_set(
            (1u16..=MAX_SEASON, 1u16..=MAX_EPISODE),
            0..=MAX_LIST_LEN,
        )
        .prop_flat_map(|pairs: HashSet<(u16, u16)>| {
            let n = pairs.len();
            // Pour chaque paire `(saison, épisode)`, on tire un
            // `(status, available)` indépendant. La taille du
            // vecteur d'attributs est exactement `n` pour zipper
            // 1-à-1 avec `pairs` lors de la projection finale.
            (
                Just(pairs),
                proptest::collection::vec((arb_status(), any::<bool>()), n),
            )
        })
        .prop_map(|(pairs, attrs)| {
            pairs
                .into_iter()
                .zip(attrs)
                .enumerate()
                .map(|(idx, ((season, episode), (status, available)))| EpisodeItem {
                    // `id` synthétique mais distinct par item — utile
                    // pour identifier sans ambiguïté l'élément
                    // sélectionné lors des assertions.
                    id: idx as i64,
                    season_number: season,
                    episode_number: episode,
                    status,
                    available,
                })
                .collect()
        })
    }

    /// Oracle indépendant de l'implémentation : reproduit le contrat
    /// `next_resume` à partir de la spécification (requirements
    /// 12.1 / 12.2 / 12.3) plutôt que de copier l'algorithme.
    ///
    /// Les étapes sont volontairement énoncées dans l'ordre du
    /// design pour rester lisibles et faciles à comparer avec la
    /// fonction sous test :
    ///
    /// 1. Filtrer les épisodes `available = true`.
    /// 2. Trier par `(season_number, episode_number)`.
    /// 3. Liste vide ⇒ `ResumeAction::None`.
    /// 4. Tous `Vu` ⇒ `RewatchFromStart(Some(premier))` (req 12.3).
    /// 5. Premier `EnCours` selon l'ordre (req 12.1, priorité 1).
    /// 6. Premier `NonVu` selon l'ordre (req 12.1, priorité 2).
    fn expected_resume(episodes: &[EpisodeItem]) -> ResumeAction {
        let mut available: Vec<&EpisodeItem> =
            episodes.iter().filter(|e| e.available).collect();
        available.sort_by_key(|e| (e.season_number, e.episode_number));

        if available.is_empty() {
            return ResumeAction::None;
        }
        if available.iter().all(|e| e.status == Status::Vu) {
            return ResumeAction::RewatchFromStart(Some(available[0].clone()));
        }
        if let Some(en_cours) = available.iter().find(|e| e.status == Status::EnCours) {
            return ResumeAction::Resume((*en_cours).clone());
        }
        if let Some(non_vu) = available.iter().find(|e| e.status == Status::NonVu) {
            return ResumeAction::Resume((*non_vu).clone());
        }
        // Inatteignable : la liste filtrée est non vide et n'est pas
        // entièrement `Vu`, donc elle contient au moins un `EnCours`
        // ou un `NonVu`. On reflète le comportement défensif de
        // `next_resume` plutôt que de paniquer.
        ResumeAction::None
    }

    proptest! {
        // ----------------------------------------------------------
        // Property 27 — Calcul du prochain épisode à reprendre.
        //
        // Pour toute liste arbitraire d'`EpisodeItem` (saisons /
        // épisodes / statuts variables, paires `(saison, épisode)`
        // uniques pour éviter les égalités de clé de tri),
        // `next_resume` doit produire exactement la même action que
        // l'oracle `expected_resume` qui réifie la spécification :
        //
        //   * liste filtrée (`available = true`) vide ⇒ `None` ;
        //   * tous les disponibles sont `Vu` ⇒
        //     `RewatchFromStart(Some(premier))` (req 12.3) ;
        //   * sinon, premier `EnCours` selon l'ordre `(saison,
        //     épisode)` (req 12.1, priorité 1) ;
        //   * sinon, premier `NonVu` selon le même ordre (req 12.1,
        //     priorité 2).
        //
        // Le résultat exposé par `next_resume` est ce qui alimente
        // le bouton `Reprendre` côté frontend (req 12.2) : valider
        // l'égalité avec l'oracle vaut donc validation de 12.2.
        //
        // Validates: Requirements 12.1, 12.2, 12.3
        // ----------------------------------------------------------
        #[test]
        fn p27_next_resume_matches_specification(
            episodes in arb_episode_list(),
        ) {
            let actual = next_resume(&episodes);
            let expected = expected_resume(&episodes);

            prop_assert_eq!(actual.clone(), expected.clone());

            // Garde-fous redondants — vérifient explicitement les
            // invariants ordinaux et de filtrage exigés par la
            // spécification, pour qu'un échec produise un message
            // ciblé sur la branche fautive plutôt qu'une simple
            // inégalité opaque entre `ResumeAction`.
            let mut available: Vec<&EpisodeItem> =
                episodes.iter().filter(|e| e.available).collect();
            available.sort_by_key(|e| (e.season_number, e.episode_number));

            match &actual {
                ResumeAction::None => {
                    // Cas dégénéré : la liste filtrée doit être vide
                    // — sinon une autre branche aurait dû être
                    // sélectionnée (req 12.1 ou 12.3).
                    prop_assert!(available.is_empty());
                }
                ResumeAction::RewatchFromStart(first) => {
                    // Req 12.3 : tous les disponibles sont `Vu` et
                    // le premier renvoyé est lex-minimum sur
                    // `(saison, épisode)`.
                    prop_assert!(!available.is_empty());
                    prop_assert!(available.iter().all(|e| e.status == Status::Vu));
                    prop_assert_eq!(first.as_ref(), Some(available[0]));
                }
                ResumeAction::Resume(picked) => {
                    // Req 12.1 : l'épisode renvoyé doit être
                    // disponible (présent dans `available`).
                    prop_assert!(picked.available);
                    prop_assert!(available.iter().any(|e| *e == picked));

                    // Priorité 1 : si un `EnCours` existe, le
                    // résultat est ce premier `EnCours`.
                    let first_en_cours = available
                        .iter()
                        .find(|e| e.status == Status::EnCours)
                        .copied();
                    if let Some(expected_pick) = first_en_cours {
                        prop_assert_eq!(picked.status, Status::EnCours);
                        prop_assert_eq!(picked, expected_pick);
                    } else {
                        // Priorité 2 : à défaut, premier `NonVu`
                        // selon l'ordre `(saison, épisode)`.
                        let first_non_vu = available
                            .iter()
                            .find(|e| e.status == Status::NonVu)
                            .copied();
                        prop_assert_eq!(picked.status, Status::NonVu);
                        prop_assert_eq!(Some(picked), first_non_vu);
                    }

                    // Invariant ordinal : aucun épisode disponible
                    // strictement antérieur au choix ne doit
                    // partager le même statut que le choix
                    // (sinon il aurait été sélectionné en premier).
                    let picked_key = (picked.season_number, picked.episode_number);
                    for e in &available {
                        let key = (e.season_number, e.episode_number);
                        if key < picked_key {
                            prop_assert_ne!(e.status, picked.status);
                        }
                    }
                }
            }
        }
    }
}
