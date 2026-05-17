// Tâche 5.2 — Normalisation de recherche (Kyro_Frontend / Kyro_Backend).
//
// Couvre les exigences :
//   - 13.2 : recherche insensible à la casse et aux accents sur le titre
//            et le titre original d'un Media_Item.
//   - 13.4 : une chaîne de recherche vide retourne la liste complète
//            (matches retourne toujours `true`).
//
// Conformément au design (cf. § « Recherche et filtres »), la
// normalisation s'effectue en deux étapes :
//   1. Décomposition Unicode NFD puis suppression des marques
//      combinantes (diacritiques) via `unicode-normalization`.
//   2. Mise en minuscules via `str::to_lowercase()`.
//
// Note : `to_lowercase()` n'est pas locale-aware, mais le design
// pseudocode l'accepte explicitement pour ce module.
//
// Comme `MediaItem` n'est pas encore défini au moment de cette tâche
// (parallèle aux tâches 4.x / 5.1), on expose :
//   - `normalize(&str) -> String` : la normalisation pure.
//   - `matches_text(title, original_title, query) -> bool` : le coeur
//     comparatif sur des chaînes brutes (utilisable par n'importe
//     quelle structure).
//   - `Searchable` : trait minimal exposant `title()` et
//     `original_title()` pour brancher facilement le futur
//     `MediaItem` (et tout autre type recherchable).
//   - `matches<S: Searchable>(item, query) -> bool` : adaptateur direct
//     sur le trait, qui sera la cible naturelle des appels côté repo
//     et côté commands.

use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// Normalise une chaîne pour la recherche : décomposition NFD,
/// suppression des marques combinantes (diacritiques), puis passage
/// en minuscules.
///
/// Exemples :
///   * `normalize("Amélie")` ⇒ `"amelie"`
///   * `normalize("CAFÉ")`   ⇒ `"cafe"`
///   * `normalize("")`       ⇒ `""`
pub fn normalize(s: &str) -> String {
    s.nfd()
        .filter(|c| !is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase()
}

/// Trait minimal pour les entités recherchables. Sera implémenté par
/// `MediaItem` (tâche 5.1 / repo) et toute future structure dotée d'un
/// titre et, optionnellement, d'un titre original.
pub trait Searchable {
    /// Titre principal affiché à l'utilisateur.
    fn title(&self) -> &str;

    /// Titre original (langue d'origine), optionnel.
    fn original_title(&self) -> Option<&str>;
}

/// Coeur de la recherche, sans dépendance à un type particulier : teste
/// si une requête `query` correspond au titre ou au titre original
/// fournis. Utile pour tester la logique sans construire de `MediaItem`
/// complet et pour être appelé par d'autres composants (filtres, etc.).
///
/// Règles (design § Recherche et filtres) :
///   * Si `query` est vide, on retourne toujours `true` (requirement
///     13.4).
///   * Sinon, on retourne `true` ssi `normalize(query)` est une
///     sous-chaîne de `normalize(title)` ou de `normalize(original_title)`
///     (requirement 13.2). L'absence de `original_title` n'empêche pas
///     un `match` sur le `title`.
pub fn matches_text(title: &str, original_title: Option<&str>, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let nq = normalize(query);
    if normalize(title).contains(&nq) {
        return true;
    }
    match original_title {
        Some(ot) => normalize(ot).contains(&nq),
        None => false,
    }
}

/// Adaptateur sur le trait `Searchable` : équivalent à `matches_text`
/// appliqué sur `(item.title(), item.original_title(), query)`.
pub fn matches<S: Searchable + ?Sized>(item: &S, query: &str) -> bool {
    matches_text(item.title(), item.original_title(), query)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Petite structure de test ad hoc, en attendant `MediaItem` (5.1).
    struct Item<'a> {
        title: &'a str,
        original_title: Option<&'a str>,
    }
    impl<'a> Searchable for Item<'a> {
        fn title(&self) -> &str {
            self.title
        }
        fn original_title(&self) -> Option<&str> {
            self.original_title
        }
    }

    // ---- normalize ----------------------------------------------------

    #[test]
    fn normalize_removes_accents() {
        assert_eq!(normalize("Amélie"), "amelie");
        assert_eq!(normalize("café"), "cafe");
        assert_eq!(normalize("naïve"), "naive");
        assert_eq!(normalize("éàçùôîÿñ"), "eacuoiyn");
    }

    #[test]
    fn normalize_lowercases() {
        assert_eq!(normalize("HELLO"), "hello");
        assert_eq!(normalize("HeLLo WoRLD"), "hello world");
    }

    #[test]
    fn normalize_handles_empty_string() {
        assert_eq!(normalize(""), "");
    }

    #[test]
    fn normalize_preserves_non_letter_characters() {
        // Espaces, ponctuation et chiffres sont conservés tels quels.
        assert_eq!(normalize("Spider-Man 2 (2004)"), "spider-man 2 (2004)");
    }

    // ---- matches_text -------------------------------------------------

    #[test]
    fn matches_text_substring_case_insensitive() {
        assert!(matches_text("The Matrix", None, "matrix"));
        assert!(matches_text("The Matrix", None, "MATRIX"));
        assert!(matches_text("The Matrix", None, "tHe"));
    }

    #[test]
    fn matches_text_substring_accent_insensitive() {
        // requête sans accents trouve le titre accentué.
        assert!(matches_text("Amélie", None, "amelie"));
        // requête accentuée trouve un titre sans accents.
        assert!(matches_text("Amelie", None, "amélie"));
    }

    #[test]
    fn matches_text_falls_back_to_original_title() {
        assert!(matches_text(
            "Le Voyage de Chihiro",
            Some("千と千尋の神隠し"),
            "千尋"
        ));
    }

    #[test]
    fn matches_text_returns_false_when_no_substring_match() {
        assert!(!matches_text("The Matrix", None, "inception"));
        assert!(!matches_text(
            "The Matrix",
            Some("The Matrix"),
            "inception"
        ));
    }

    #[test]
    fn matches_text_empty_query_always_true() {
        // requirement 13.4 : la chaîne vide retourne toujours `true`,
        // même quand le titre lui-même est vide.
        assert!(matches_text("anything", None, ""));
        assert!(matches_text("anything", Some("autre"), ""));
        assert!(matches_text("", None, ""));
        assert!(matches_text("", Some(""), ""));
    }

    #[test]
    fn matches_text_no_original_title_does_not_panic() {
        // Sans original_title, on se rabat sur le seul title.
        assert!(matches_text("Inception", None, "incep"));
        assert!(!matches_text("Inception", None, "matrix"));
    }

    // ---- matches via Searchable --------------------------------------

    #[test]
    fn matches_via_trait_uses_title_and_original_title() {
        let item = Item {
            title: "Le Voyage de Chihiro",
            original_title: Some("千と千尋の神隠し"),
        };
        assert!(matches(&item, "voyage"));
        assert!(matches(&item, "VOYAGE"));
        assert!(matches(&item, "chihiro"));
        assert!(matches(&item, "千尋"));
        assert!(!matches(&item, "matrix"));
    }

    #[test]
    fn matches_via_trait_empty_query_returns_true() {
        let item = Item {
            title: "Anything",
            original_title: None,
        };
        assert!(matches(&item, ""));
    }
}

#[cfg(test)]
mod proptests {
    //! Tests de propriétés pour le module `search` (tâches 5.3 & 5.4).
    //!
    //! ## Property 28 — Recherche insensible à la casse et aux accents
    //! Pour tout titre `t` (lettres ASCII et accentuées, casse mixte) et
    //! toute requête `q`, on doit avoir :
    //!     `matches_text(t, None, q) ⇔ normalize(t).contains(&normalize(q))`.
    //! Cette équivalence capture à la fois l'insensibilité à la casse,
    //! l'insensibilité aux accents, et la règle « requête vide ⇒ true »
    //! (puisque toute chaîne contient la chaîne vide).
    //! **Validates: Requirements 13.2**
    //!
    //! ## Property 29 — Recherche vide retourne tout
    //! Pour toute bibliothèque `L` (liste arbitraire de couples
    //! `(title, original_title)`), `apply_search("", L) == L`.
    //! **Validates: Requirements 13.4**

    use super::*;
    use proptest::prelude::*;

    /// Petit alphabet contenant des lettres ASCII en casse mixte, des
    /// lettres accentuées (latin-1 + `ñ`/`Ñ`) en casse mixte, ainsi que
    /// quelques séparateurs et chiffres. Suffisant pour exercer la
    /// pipeline NFD + suppression des marques combinantes + lowercase
    /// sans alourdir la stratégie.
    const SAMPLE_CHARS: &[char] = &[
        // ASCII minuscules
        'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p',
        // ASCII majuscules (mélangées pour casse mixte)
        'A', 'B', 'C', 'D', 'E', 'F', 'M',
        // Lettres accentuées minuscules
        'é', 'è', 'ê', 'ë', 'à', 'â', 'ä', 'î', 'ï', 'ô', 'ö', 'ù', 'û', 'ü', 'ç', 'ñ',
        // Lettres accentuées majuscules
        'É', 'À', 'Ç', 'Ñ',
        // Séparateurs et chiffres
        ' ', '-', '0', '1', '2', '3',
    ];

    fn char_strategy() -> impl Strategy<Value = char> {
        (0usize..SAMPLE_CHARS.len()).prop_map(|idx| SAMPLE_CHARS[idx])
    }

    /// Génère une chaîne arbitraire bornée à partir de l'alphabet
    /// `SAMPLE_CHARS` (lettres mixtes, accents, casse mixte).
    fn text_strategy(max_len: usize) -> impl Strategy<Value = String> {
        proptest::collection::vec(char_strategy(), 0..max_len)
            .prop_map(|chars| chars.into_iter().collect())
    }

    /// Helper « bibliothèque » utilisé par la Property 29. Filtre la
    /// liste avec `matches_text` appliquée sur chaque couple
    /// `(title, original_title)`. Équivalent fonctionnel à
    /// `items.iter().filter(|i| matches(i, query))` sur un type
    /// implémentant `Searchable`.
    fn apply_search(
        query: &str,
        items: &[(String, Option<String>)],
    ) -> Vec<(String, Option<String>)> {
        items
            .iter()
            .filter(|(t, ot)| matches_text(t, ot.as_deref(), query))
            .cloned()
            .collect()
    }

    proptest! {
        /// Property 28 — Recherche insensible à la casse et aux accents.
        ///
        /// **Validates: Requirements 13.2**
        ///
        /// Quel que soit le titre (avec accents et casse mixte) et la
        /// requête, `matches_text(title, None, query)` est exactement
        /// `normalize(title).contains(&normalize(query))`.
        ///
        /// On utilise `original_title = None` pour isoler la branche
        /// titre principal — c'est le coeur de l'exigence 13.2.
        #[test]
        fn prop28_matches_iff_normalized_substring(
            title in text_strategy(40),
            query in text_strategy(15),
        ) {
            let expected = normalize(&title).contains(&normalize(&query));
            let actual = matches_text(&title, None, &query);
            prop_assert_eq!(actual, expected);
        }

        /// Property 29 — Recherche vide retourne tout.
        ///
        /// **Validates: Requirements 13.4**
        ///
        /// Pour toute bibliothèque arbitraire, appliquer la recherche
        /// avec `query = ""` doit retourner exactement la liste d'entrée
        /// (chaque item passe le filtre, et l'ordre est préservé puisque
        /// `apply_search` est un simple `iter().filter()`).
        #[test]
        fn prop29_empty_query_returns_all(
            items in proptest::collection::vec(
                (text_strategy(20), proptest::option::of(text_strategy(20))),
                0..20,
            ),
        ) {
            let filtered = apply_search("", &items);
            prop_assert_eq!(filtered, items);
        }
    }
}
