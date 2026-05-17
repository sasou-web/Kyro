//! Statut de visionnage d'un Media_Item ou d'un Episode_Item et calcul
//! associé.
//!
//! Ce module reste **pur** (aucune IO, aucune dépendance Tauri ou
//! sqlx) et est testable directement par `cargo test`.
//!
//! Trois sources de vérité doivent rester alignées sur les chaînes
//! `non_vu` / `en_cours` / `vu` :
//!
//! * la migration SQLite `0001_init.sql`
//!   (`status IN ('non_vu','en_cours','vu')`),
//! * la sérialisation JSON exposée au Kyro_Frontend (Tauri commands),
//! * le format texte utilisé pour les logs / messages d'erreur.
//!
//! La fonction [`compute_status`] couvre la dérivation automatique du
//! statut à partir d'un Watch_Progress et d'une durée (requirements
//! 8.1, 8.2, 8.3) tout en respectant un éventuel override manuel
//! (requirement 8.4).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Statut de visionnage d'un Media_Item (de type `Film`) ou d'un
/// Episode_Item.
///
/// Les variantes correspondent une-à-une aux chaînes acceptées par le
/// CHECK SQLite (`status IN ('non_vu','en_cours','vu')`). La
/// sérialisation `serde` utilise [`serde(rename_all = "snake_case")`]
/// pour produire/accepter exactement ces trois chaînes côté JSON et
/// côté DB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// `non_vu` — jamais lancé ou progression nulle.
    NonVu,
    /// `en_cours` — progression strictement entre 0 % et 90 %
    /// (requirement 8.3).
    EnCours,
    /// `vu` — progression > 90 % (requirement 8.2) ou statut forcé
    /// manuellement (requirement 8.4).
    Vu,
}

impl Status {
    /// Représentation textuelle stable du statut.
    ///
    /// Identique au `serde(rename_all = "snake_case")` ci-dessus,
    /// mais exposée comme `&'static str` pour les usages non-serde
    /// (paramètres `sqlx`, logs `tracing`, comparaisons directes).
    pub const fn as_str(self) -> &'static str {
        match self {
            Status::NonVu => "non_vu",
            Status::EnCours => "en_cours",
            Status::Vu => "vu",
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Erreur retournée par [`Status::from_str`] pour une entrée non
/// reconnue. Permet aux appels (notamment depuis la couche
/// `infra::repo`) de propager une erreur typée plutôt qu'une chaîne.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("statut inconnu: `{value}` (attendu: non_vu | en_cours | vu)")]
pub struct ParseStatusError {
    /// Valeur d'entrée rejetée. Conservée pour faciliter le diagnostic
    /// (logs, messages d'erreur côté frontend).
    pub value: String,
}

impl FromStr for Status {
    type Err = ParseStatusError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "non_vu" => Ok(Status::NonVu),
            "en_cours" => Ok(Status::EnCours),
            "vu" => Ok(Status::Vu),
            other => Err(ParseStatusError {
                value: other.to_owned(),
            }),
        }
    }
}

/// Calcule le statut de visionnage d'un Media_Item ou Episode_Item.
///
/// Cf. design § « Calcul du statut » et requirements 8.1/8.2/8.3/8.4 :
///
/// 1. Si `manual_override` est `Some(s)`, retourne `s` tel quel —
///    l'utilisateur a forcé le statut depuis le frontend
///    (requirement 8.4) et la dérivation automatique ne doit pas
///    écraser ce choix.
/// 2. Sinon, si `duration_seconds == 0`, retourne `Status::NonVu`. Une
///    durée nulle correspond soit à un fichier dont la sonde ffprobe a
///    échoué (requirement 5.5, métadonnées techniques `null`) soit à
///    un item jamais probé : aucun ratio fiable n'est calculable.
/// 3. Sinon, calcule `ratio = progress / duration_seconds` en `f64`
///    et applique :
///    * `ratio > 0.9` → `Status::Vu` (requirement 8.2),
///    * `ratio > 0.0 && ratio <= 0.9` → `Status::EnCours`
///      (requirement 8.3 : `0 < progress ≤ 90 %`),
///    * sinon (i.e. `ratio == 0.0`) → `Status::NonVu`
///      (requirement 8.1 par défaut).
///
/// `progress` peut excéder `duration_seconds` (off-by-one en fin de
/// fichier, position ffprobe arrondie). Le ratio est alors `>= 1.0`,
/// ce qui retombe naturellement sur `Status::Vu`.
pub fn compute_status(
    progress: u32,
    duration_seconds: u32,
    manual_override: Option<Status>,
) -> Status {
    // 1. Override manuel prioritaire (requirement 8.4).
    if let Some(s) = manual_override {
        return s;
    }

    // 2. Durée inconnue / nulle : pas de ratio calculable, on retombe
    //    sur le statut par défaut (requirement 8.1).
    if duration_seconds == 0 {
        return Status::NonVu;
    }

    // 3. Comparaison strictement supérieure à 0.9 pour `Vu` afin que
    //    le seuil 90 % exact reste `EnCours` (requirement 8.3 :
    //    `progress ≤ 90 %` reste `en_cours`).
    let ratio = f64::from(progress) / f64::from(duration_seconds);
    if ratio > 0.9 {
        Status::Vu
    } else if ratio > 0.0 {
        Status::EnCours
    } else {
        Status::NonVu
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --------------------------------------------------------------
    // Override manuel — requirement 8.4 : prioritaire sur la
    // dérivation automatique, peu importe progress/durée.
    // --------------------------------------------------------------

    #[test]
    fn manual_override_vu_wins_even_when_progress_is_zero() {
        // Sans override, (0, 0) ou (0, 100) seraient `NonVu`. L'override
        // manuel doit pourtant retourner `Vu` tel quel.
        assert_eq!(compute_status(0, 0, Some(Status::Vu)), Status::Vu);
        assert_eq!(compute_status(0, 100, Some(Status::Vu)), Status::Vu);
    }

    #[test]
    fn manual_override_non_vu_wins_even_when_ratio_would_be_vu() {
        // Sans override, (95, 100) donnerait `Vu`. L'override doit
        // forcer `NonVu`.
        assert_eq!(
            compute_status(95, 100, Some(Status::NonVu)),
            Status::NonVu
        );
    }

    #[test]
    fn manual_override_en_cours_is_returned_as_is() {
        // Override `EnCours` admis : la fonction n'inspecte pas la
        // valeur, elle ne fait que la propager.
        assert_eq!(
            compute_status(100, 100, Some(Status::EnCours)),
            Status::EnCours
        );
    }

    // --------------------------------------------------------------
    // Durée nulle — pas de ratio calculable, retombe sur `NonVu`.
    // --------------------------------------------------------------

    #[test]
    fn duration_zero_returns_non_vu_regardless_of_progress() {
        assert_eq!(compute_status(0, 0, None), Status::NonVu);
        assert_eq!(compute_status(42, 0, None), Status::NonVu);
        assert_eq!(compute_status(u32::MAX, 0, None), Status::NonVu);
    }

    // --------------------------------------------------------------
    // Branche `NonVu` — progress = 0 (ratio == 0.0).
    // --------------------------------------------------------------

    #[test]
    fn progress_zero_with_known_duration_is_non_vu() {
        assert_eq!(compute_status(0, 100, None), Status::NonVu);
        assert_eq!(compute_status(0, 7200, None), Status::NonVu);
    }

    // --------------------------------------------------------------
    // Branche `EnCours` — 0 < ratio <= 0.9 (requirement 8.3).
    // --------------------------------------------------------------

    #[test]
    fn ratio_in_open_zero_to_ninety_percent_is_en_cours() {
        // Petit ratio — 1 / 100 = 0.01.
        assert_eq!(compute_status(1, 100, None), Status::EnCours);
        // Ratio médian — 50 %.
        assert_eq!(compute_status(50, 100, None), Status::EnCours);
        // Juste sous le seuil — 89 / 100 = 0.89.
        assert_eq!(compute_status(89, 100, None), Status::EnCours);
    }

    #[test]
    fn ratio_exactly_ninety_percent_stays_en_cours() {
        // Requirement 8.3 : `progress ≤ 90 %` reste `en_cours`.
        // 90 / 100 = 0.9 exact — ne doit PAS basculer en `Vu`.
        assert_eq!(compute_status(90, 100, None), Status::EnCours);
        // Multiple plus grand pour valider une autre durée.
        assert_eq!(compute_status(900, 1000, None), Status::EnCours);
    }

    // --------------------------------------------------------------
    // Branche `Vu` — ratio > 0.9 (requirement 8.2).
    // --------------------------------------------------------------

    #[test]
    fn ratio_strictly_above_ninety_percent_is_vu() {
        // 91 / 100 = 0.91, juste au-dessus du seuil.
        assert_eq!(compute_status(91, 100, None), Status::Vu);
        // 95 / 100 = 0.95.
        assert_eq!(compute_status(95, 100, None), Status::Vu);
    }

    #[test]
    fn ratio_at_or_above_one_is_vu() {
        // Cas progress == duration : ratio == 1.0 > 0.9.
        assert_eq!(compute_status(100, 100, None), Status::Vu);
        // Cas progress > duration (off-by-one fin de lecture).
        assert_eq!(compute_status(200, 100, None), Status::Vu);
    }

    // --------------------------------------------------------------
    // Round-trip Display / FromStr — garantie que les chaînes
    // exposées au frontend et stockées en DB matchent la migration.
    // --------------------------------------------------------------

    #[test]
    fn display_matches_db_check_strings() {
        assert_eq!(Status::NonVu.to_string(), "non_vu");
        assert_eq!(Status::EnCours.to_string(), "en_cours");
        assert_eq!(Status::Vu.to_string(), "vu");
    }

    #[test]
    fn as_str_matches_display() {
        for s in [Status::NonVu, Status::EnCours, Status::Vu] {
            assert_eq!(s.as_str(), s.to_string());
        }
    }

    #[test]
    fn from_str_round_trip_for_all_variants() {
        for s in [Status::NonVu, Status::EnCours, Status::Vu] {
            let parsed: Status = s.as_str().parse().expect("variant must round-trip");
            assert_eq!(parsed, s);
        }
    }

    #[test]
    fn from_str_rejects_unknown_value_with_typed_error() {
        let err = "watched"
            .parse::<Status>()
            .expect_err("unknown variant must error");
        assert_eq!(err.value, "watched");
        // Le message d'erreur doit citer la valeur fautive (utile pour
        // les logs et les messages côté frontend).
        assert!(err.to_string().contains("watched"));
    }

    #[test]
    fn from_str_is_case_sensitive() {
        // La DB stocke en minuscules ; on ne tolère pas `Vu`/`VU` pour
        // éviter les divergences d'écriture entre couches.
        assert!("Vu".parse::<Status>().is_err());
        assert!("EN_COURS".parse::<Status>().is_err());
    }

    // --------------------------------------------------------------
    // Sérialisation / désérialisation JSON — contrat exposé aux
    // commandes Tauri.
    // --------------------------------------------------------------

    #[test]
    fn serde_serializes_to_snake_case_strings() {
        assert_eq!(serde_json::to_string(&Status::NonVu).unwrap(), "\"non_vu\"");
        assert_eq!(
            serde_json::to_string(&Status::EnCours).unwrap(),
            "\"en_cours\""
        );
        assert_eq!(serde_json::to_string(&Status::Vu).unwrap(), "\"vu\"");
    }

    #[test]
    fn serde_deserializes_from_snake_case_strings() {
        assert_eq!(
            serde_json::from_str::<Status>("\"non_vu\"").unwrap(),
            Status::NonVu
        );
        assert_eq!(
            serde_json::from_str::<Status>("\"en_cours\"").unwrap(),
            Status::EnCours
        );
        assert_eq!(
            serde_json::from_str::<Status>("\"vu\"").unwrap(),
            Status::Vu
        );
    }

    #[test]
    fn serde_rejects_unknown_status_string() {
        // Un statut inconnu doit échouer à la désérialisation : la
        // couche IPC ne doit jamais accepter silencieusement un
        // statut hors contrat.
        assert!(serde_json::from_str::<Status>("\"watched\"").is_err());
    }
}

#[cfg(test)]
mod proptests {
    //! Tests de propriétés couvrant le calcul de [`compute_status`].
    //!
    //! Ces tests utilisent `proptest` (cf. `dev-dependencies` du
    //! `Cargo.toml`) et sont isolés dans un sous-module dédié pour ne
    //! pas alourdir le module `tests` exemple-par-exemple ci-dessus.
    //!
    //! Chaque propriété est numérotée selon le tableau du design
    //! (« 37 propriétés ») et annonce explicitement les exigences
    //! qu'elle valide.

    use super::*;
    use proptest::prelude::*;

    /// Borne supérieure pour la durée générée. On limite à
    /// `u32::MAX / 2` pour conserver une marge confortable côté
    /// arithmétique flottante (`f64::from(u32)` est exact, mais on
    /// évite les durées proches de `u32::MAX` qui n'ont pas de sens
    /// physique pour un fichier vidéo et restent loin des cas réels
    /// du scanner).
    const MAX_DURATION: u32 = u32::MAX / 2;

    proptest! {
        // ----------------------------------------------------------
        // Property 16 — Calcul déterministe du statut.
        //
        // Sans override manuel, `compute_status` doit dépendre
        // uniquement du ratio `progress / duration` et respecter les
        // trois branches du seuil 90 % :
        //   * ratio > 0.9                ⇒ Status::Vu      (req 8.2)
        //   * 0 < ratio ≤ 0.9            ⇒ Status::EnCours (req 8.3)
        //   * ratio == 0.0 (progress=0)  ⇒ Status::NonVu   (req 8.1)
        //
        // Validates: Requirements 8.2, 8.3
        // ----------------------------------------------------------
        #[test]
        fn p16_compute_status_is_deterministic_against_threshold(
            // `duration` strictement positif pour avoir un ratio
            // calculable ; borné par `MAX_DURATION` (cf. constante
            // ci-dessus). `progress` est ensuite contraint dans
            // `0..=duration` pour rester dans le cas physique
            // « progression sans dépassement ».
            (duration, progress) in (1u32..=MAX_DURATION)
                .prop_flat_map(|d| (Just(d), 0u32..=d)),
        ) {
            let actual = compute_status(progress, duration, None);

            // Référence : on recalcule la branche attendue à partir
            // du ratio f64. La fonction étant elle-même définie via
            // ce ratio, l'oracle reproduit le contrat plutôt que
            // l'implémentation : on vérifie la cohérence avec les
            // seuils de la spécification (8.1/8.2/8.3) — pas un
            // simple `==` syntaxique avec la fonction.
            let ratio = f64::from(progress) / f64::from(duration);
            let expected = if ratio > 0.9 {
                Status::Vu
            } else if progress > 0 {
                Status::EnCours
            } else {
                Status::NonVu
            };

            prop_assert_eq!(actual, expected);

            // Garde-fous redondants — vérifient explicitement chaque
            // branche du seuil 90 % indépendamment de l'oracle, pour
            // que l'échec d'une seule clause produise un message
            // ciblé.
            if progress == 0 {
                prop_assert_eq!(actual, Status::NonVu);
            } else if ratio > 0.9 {
                prop_assert_eq!(actual, Status::Vu);
            } else {
                // 0 < ratio ≤ 0.9 (req 8.3 : `progress ≤ 90 %` reste
                // `en_cours`, seuil exact inclus).
                prop_assert_eq!(actual, Status::EnCours);
            }
        }

        // ----------------------------------------------------------
        // Property 17 — Prévalence du statut manuel.
        //
        // Quand l'utilisateur force `Vu` ou `NonVu` depuis le
        // frontend (requirement 8.4), `compute_status` doit renvoyer
        // l'override tel quel sans considérer `progress` ni
        // `duration_seconds`. La spec mentionne explicitement les
        // forçages `Vu` et `NonVu` — on cible donc ces deux
        // variantes.
        //
        // Validates: Requirements 8.4
        // ----------------------------------------------------------
        #[test]
        fn p17_manual_override_prevails_over_progress(
            // Progression et durée totalement libres dans `u32` —
            // l'override doit gagner même quand la dérivation
            // automatique donnerait un autre statut (ex. ratio > 0.9
            // alors que l'override est `NonVu`, ou progress = 0
            // alors que l'override est `Vu`).
            progress in any::<u32>(),
            duration in any::<u32>(),
            // `prop_oneof!` choisit uniformément entre les deux
            // statuts forcés cités par la spec. `Just` réutilise la
            // dérivation `Copy` de `Status` pour produire un
            // générateur trivial.
            forced in prop_oneof![Just(Status::Vu), Just(Status::NonVu)],
        ) {
            let actual = compute_status(progress, duration, Some(forced));
            prop_assert_eq!(actual, forced);
        }
    }
}
