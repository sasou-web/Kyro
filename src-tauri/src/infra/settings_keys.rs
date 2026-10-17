//! Tâche 7.5 — clés réservées de la table `setting_value`.
//!
//! Ce module centralise les littéraux de clé acceptés par
//! [`Repo::upsert_setting`] et [`Repo::get_setting`]
//! (cf. `infra/repo.rs`). Le design § « Repository (sqlx) » et le DDL
//! `migrations/0001_init.sql` énumèrent **explicitement** les cinq clés
//! attendues :
//!
//! | Clé                   | Origine du requirement                  |
//! |-----------------------|-----------------------------------------|
//! | `external_player_path`| Requirement 10.1, 10.2 — chemin du lecteur |
//! | `ffprobe_path`        | Requirement 5.4 — chemin manuel ffprobe |
//! | `theme`               | Requirement 16.1, 16.14 — thème actif   |
//! | `scan_at_startup`     | Requirement 3.7 — scan au boot          |
//! | `last_category`       | Requirement 7.1 — dernière catégorie    |
//! | `playback_mode`       | Player embarqué — `embedded` (défaut) ou `external` |
//!
//! ## Pourquoi un *allowlist* explicite
//!
//! La table `setting_value` n'a pas de contrainte `CHECK` côté DDL
//! (laisser SQLite imposer un littéral parmi 5 clés rendrait la
//! migration plus rigide qu'utile). On porte donc l'*allowlist* dans
//! le code applicatif : toute clé hors de [`RESERVED_KEYS`] est
//! refusée par `upsert_setting` avec une `KyroError::Validation`.
//! Cela protège contre :
//!
//! 1. **Les fautes de frappe** côté commands Tauri (`scan_au_demarrage`
//!    vs `scan_at_startup`) qui produiraient silencieusement une
//!    seconde ligne en base et un comportement « setting non
//!    persisté ».
//! 2. **Les évolutions futures non documentées** : ajouter une clé
//!    impose de l'ajouter ici **et** dans le design — point unique de
//!    vérité.
//!
//! `get_setting` accepte au contraire **n'importe quelle** clé : la
//! lecture d'une clé inconnue retourne simplement `None` (cas
//! équivalent à « pas de valeur stockée »). Cela évite de casser la
//! lecture en cas de migration en cours qui aurait introduit une
//! nouvelle clé non encore enregistrée dans `RESERVED_KEYS`.

/// Clé `setting_value` portant le chemin absolu vers l'exécutable
/// External_Player choisi par l'utilisateur (cf. requirements 10.1,
/// 10.2). La valeur stockée est le `path.to_string_lossy()` du chemin
/// validé par la commande de réglage.
pub const SETTING_KEY_EXTERNAL_PLAYER_PATH: &str = "external_player_path";

/// Clé `setting_value` portant le chemin absolu vers `ffprobe.exe`
/// quand l'utilisateur l'a renseigné manuellement (priorité 2 du
/// résolveur — cf. `infra/ffprobe_locator.rs` et requirement 5.4).
pub const SETTING_KEY_FFPROBE_PATH: &str = "ffprobe_path";

/// Clé `setting_value` portant le thème actif. Valeurs attendues :
/// `"adwaita-dark"` (défaut au premier démarrage — requirement 16.1)
/// ou `"adwaita-light"` (requirement 16.14).
pub const SETTING_KEY_THEME: &str = "theme";

/// Clé `setting_value` portant le drapeau « scan au démarrage »
/// (requirement 3.7). Valeur attendue : `"true"` ou `"false"` (la
/// table stocke des `TEXT` ; la commande de réglage convertit le
/// booléen côté frontend / Rust).
pub const SETTING_KEY_SCAN_AT_STARTUP: &str = "scan_at_startup";

/// Clé `setting_value` portant la dernière catégorie consultée
/// (requirement 7.1). Valeurs attendues : littéraux `"Films"`,
/// `"Series"` ou `"Animes"` (cohérents avec
/// `domain::types::Category::as_str`).
pub const SETTING_KEY_LAST_CATEGORY: &str = "last_category";

/// Clé `setting_value` portant le mode de lecture actif
/// (player embarqué libmpv vs lecteur externe).
///
/// Valeurs acceptées :
/// * `"embedded"` — lit via libmpv embarqué dans une fenêtre Kyro
///   dédiée (défaut au premier démarrage si la DLL est présente).
/// * `"external"` — lit via l'application externe configurée par
///   `external_player_path` (mpv.net + IPC NDJSON ou autre lecteur).
///
/// La validation des valeurs est portée par
/// [`crate::infra::repo::Repo::upsert_setting`] via la combinaison
/// `is_reserved` (présence dans l'allowlist) + parsing applicatif
/// côté `commands::settings::set_playback_mode`.
pub const SETTING_KEY_PLAYBACK_MODE: &str = "playback_mode";

/// Allowlist exhaustive des clés acceptées par
/// [`crate::infra::repo::Repo::upsert_setting`].
///
/// L'ordre est purement déclaratif (ne reflète pas une priorité) et
/// reproduit l'énumération du design / du commentaire de la migration
/// `0001_init.sql`. Toute évolution doit être faite ici **et** dans le
/// design (cf. doc du module).
pub const RESERVED_KEYS: &[&str] = &[
    SETTING_KEY_EXTERNAL_PLAYER_PATH,
    SETTING_KEY_FFPROBE_PATH,
    SETTING_KEY_THEME,
    SETTING_KEY_SCAN_AT_STARTUP,
    SETTING_KEY_LAST_CATEGORY,
    SETTING_KEY_PLAYBACK_MODE,
];

/// Indique si `key` appartient à l'allowlist [`RESERVED_KEYS`].
///
/// Comparaison **strictement sensible à la casse** : la table
/// `setting_value` utilise `TEXT PRIMARY KEY` (sans `COLLATE NOCASE`)
/// et les littéraux du design sont en `snake_case` minuscule. Un
/// usage avec une casse différente est traité comme une faute de
/// frappe et refusé par `upsert_setting`.
pub fn is_reserved(key: &str) -> bool {
    RESERVED_KEYS.contains(&key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_keys_match_design_enumeration() {
        // Le test fige l'allowlist : toute évolution force une mise à
        // jour explicite ici (et donc dans le design).
        assert_eq!(
            RESERVED_KEYS,
            &[
                "external_player_path",
                "ffprobe_path",
                "theme",
                "scan_at_startup",
                "last_category",
                "playback_mode",
            ]
        );
    }

    #[test]
    fn is_reserved_accepts_each_key_exactly() {
        for k in RESERVED_KEYS {
            assert!(is_reserved(k), "la clé {k:?} doit être réservée");
        }
    }

    #[test]
    fn is_reserved_rejects_unknown_keys() {
        // Quelques fautes typiques explicitement écartées.
        for bogus in [
            "",
            "external-player-path", // tiret au lieu d'underscore
            "FFPROBE_PATH",         // majuscules
            "scan_au_demarrage",    // version FR
            "Theme",                // CamelCase
            "unknown",
        ] {
            assert!(!is_reserved(bogus), "{bogus:?} ne doit pas être réservée");
        }
    }
}
