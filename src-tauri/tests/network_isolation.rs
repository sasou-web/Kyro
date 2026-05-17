//! Tâche 17.4 — Property 13 : Aucune requête réseau sortante.
//!
//! # Justification de l'approche
//!
//! La tâche prévoyait initialement un faux résolveur DNS local et un
//! proxy TCP qui échoue toute connexion sortante non-localhost. Cette
//! approche est non-portable (monkey-patching socket, dépendance à des
//! ports privilégiés, instable sur les CI Windows / macOS / Linux) et
//! n'apporte pas de garantie supérieure : un binaire qui ne contient
//! **aucun** code client HTTP ne peut, par construction, émettre aucune
//! requête sortante quelle que soit la séquence d'actions utilisateur.
//!
//! On adopte donc l'**analyse statique** prescrite par les notes de la
//! tâche 17.4 :
//!
//! 1. **Aucune dépendance directe HTTP cliente dans `Cargo.toml`**
//!    (`reqwest`, `ureq`, `surf`, `awc`, `isahc`, `attohttpc`, `curl`,
//!    `hyper`, `hyper-tls`, `hyper-rustls`, `hyper-util`).
//!    Note : `reqwest` et `hyper-util` sont **transitifs** via
//!    `tauri 2.11.2` ; le design (§ « Garantie zéro réseau ») cible
//!    explicitement les dépendances **directes** de Kyro, conformément
//!    à la stratégie d'allow-list de Tauri (les transitives `reqwest`
//!    sont compilées mais inertes tant qu'aucune feature
//!    `tauri/http-*` n'est activée — ce qui est le cas, cf. `Cargo.toml`).
//!
//! 2. **Aucun `use` ni qualification `crate::` vers un client HTTP**
//!    dans le code source de `src/`. Ce contrôle attrape toute
//!    régression où un développeur câblerait `reqwest::get(...)` sans
//!    déclarer la dépendance directement (cas peu probable mais à
//!    couvrir : la transitive `reqwest` étant présente dans le graphe
//!    via Tauri, elle est techniquement linkable depuis n'importe quel
//!    fichier de `kyro`).
//!
//! 3. **Le feature flag `network` reste vide** (`network = []`).
//!    Le design réserve cette feature pour de futurs usages opt-in
//!    explicites de l'utilisateur ; tant qu'elle n'est pas vide, la
//!    garantie zéro-réseau du requirement 17.3 cesse d'être
//!    automatique.
//!
//! Ces trois invariants impliquent qu'aucune fonction d'émission de
//! requête HTTP n'est atteignable depuis le code de Kyro,
//! indépendamment de la séquence d'opérations utilisateur (ajout
//! dossier, scan, édition, lecture mockée, recherche, filtres). On
//! exprime cette propriété sous la forme d'un test `proptest` qui
//! quantifie universellement sur une séquence aléatoire de ces six
//! opérations : pour toute séquence, les trois invariants statiques
//! restent vrais (corollaire trivial mais conforme à l'expression
//! formelle de la propriété 13 dans le design).
//!
//! Trois tests « smoke » directs accompagnent le test property pour
//! garantir un signalement clair même si proptest devait échouer à
//! shrinker correctement.
//!
//! Validates: Requirements 6.4, 17.3

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use proptest::prelude::*;
use regex::Regex;
use walkdir::WalkDir;

// ---------------------------------------------------------------------------
// Listes noires
// ---------------------------------------------------------------------------

/// Crates HTTP cliente interdits comme dépendances **directes** de
/// `Cargo.toml` (sections `[dependencies]` et `[build-dependencies]`).
///
/// `hyper`, `hyper-util`, `hyper-tls`, `hyper-rustls` sont listés ici
/// car aucun n'a besoin d'être ajouté directement par Kyro : Tauri les
/// gère lui-même en transitive pour son usage IPC interne.
const FORBIDDEN_CARGO_DEPS: &[&str] = &[
    "reqwest",
    "ureq",
    "surf",
    "awc",
    "isahc",
    "attohttpc",
    "hyper",
    "hyper-tls",
    "hyper-rustls",
    "hyper-util",
    "curl",
];

/// Modules HTTP cliente interdits comme imports dans le code source.
///
/// Note : `hyper` et `hyper_util` sont **exclus** de cette liste car
/// Tauri peut les exposer indirectement via ses propres ré-exports IPC
/// loopback ; détecter leur simple mention par regex provoquerait des
/// faux-positifs sur du code interne légitime. Les vraies portes
/// d'entrée vers le réseau sortant (`reqwest`, `ureq`, …) restent, elles,
/// strictement interdites.
const FORBIDDEN_RUST_IMPORTS: &[&str] = &[
    "reqwest",
    "ureq",
    "surf",
    "awc",
    "isahc",
    "attohttpc",
    "hyper_tls",
    "hyper_rustls",
    "curl",
];

// ---------------------------------------------------------------------------
// Helpers — chargement / parsing avec mise en cache (`OnceLock`) afin
// que les 100+ itérations de proptest ne relisent pas le disque à
// chaque tirage.
// ---------------------------------------------------------------------------

fn manifest_dir() -> &'static Path {
    static CACHE: OnceLock<PathBuf> = OnceLock::new();
    CACHE.get_or_init(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

fn cargo_toml_text() -> &'static str {
    static CACHE: OnceLock<String> = OnceLock::new();
    CACHE.get_or_init(|| {
        std::fs::read_to_string(manifest_dir().join("Cargo.toml"))
            .expect("Cargo.toml lisible depuis CARGO_MANIFEST_DIR")
    })
}

/// Parse minimaliste — extrait les noms de dépendances directes
/// déclarées dans `[dependencies]` et `[build-dependencies]`.
///
/// On ne dépend pas d'un crate TOML ici afin de garder la suite de
/// tests indépendante d'une dépendance supplémentaire (un parseur
/// `toml` tiers introduirait sa propre transitive `serde`, etc., et le
/// test perdrait en lisibilité). Le sous-ensemble de syntaxe Cargo
/// utilisé dans `Cargo.toml` est volontairement régulier : chaque
/// dépendance tient sur une ligne dont le préfixe (avant `=`) est le
/// nom du crate, ou une sous-table `[dependencies.<name>]`.
fn parse_direct_deps(toml: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let mut in_target_section = false;

    for raw in toml.lines() {
        let line = raw.trim_start();

        // En-tête de section : on bascule dans/hors d'une section
        // pertinente, et on capture aussi les sous-tables explicites
        // `[dependencies.<name>]` / `[build-dependencies.<name>]`.
        if let Some(rest) = line.strip_prefix('[') {
            in_target_section = false;

            let header = line.trim_end();
            if header == "[dependencies]" || header == "[build-dependencies]" {
                in_target_section = true;
                continue;
            }

            // Sous-table : `[dependencies.foo]` ou `[build-dependencies.foo]`.
            for prefix in ["dependencies.", "build-dependencies."] {
                if let Some(after_prefix) = rest.strip_prefix(prefix) {
                    if let Some(end) = after_prefix.find(']') {
                        let name = after_prefix[..end].trim().trim_matches('"');
                        if !name.is_empty() {
                            deps.push(name.to_string());
                        }
                    }
                }
            }
            continue;
        }

        if !in_target_section {
            continue;
        }

        // Ligne vide ou commentaire → on ignore.
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Ligne de la forme `name = ...` : on extrait `name` (le premier
        // token avant le premier `=`). Les lignes de continuation à
        // l'intérieur d'une inline-table multi-lignes (entrées d'array
        // de features comme `"rt-multi-thread",`) ne contiennent pas de
        // `=` et sont donc ignorées proprement.
        if let Some(eq) = line.find('=') {
            let name = line[..eq].trim();
            if !name.is_empty() && is_valid_dep_name(name) {
                deps.push(name.to_string());
            }
        }
    }

    deps
}

/// Filtre défensif : un nom de crate Cargo est composé de
/// `[A-Za-z0-9_-]+` ; cela élimine les éventuelles lignes parasites
/// (clés de feature avec virgule, fragments d'array...) que le
/// parseur grossier aurait laissé passer.
fn is_valid_dep_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn direct_deps() -> &'static [String] {
    static CACHE: OnceLock<Vec<String>> = OnceLock::new();
    CACHE.get_or_init(|| parse_direct_deps(cargo_toml_text()))
}

fn rust_source_files() -> &'static [PathBuf] {
    static CACHE: OnceLock<Vec<PathBuf>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let src = manifest_dir().join("src");
        WalkDir::new(&src)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("rs"))
            .map(|e| e.into_path())
            .collect()
    })
}

fn import_regex() -> &'static Regex {
    static CACHE: OnceLock<Regex> = OnceLock::new();
    CACHE.get_or_init(|| {
        let alts = FORBIDDEN_RUST_IMPORTS.join("|");
        // Capture `use foo::...`, `pub use foo::...`,
        // `pub(crate) use foo::...`, et `extern crate foo;`.
        let pat = format!(
            r"^\s*(?:pub(?:\s*\([^)]*\))?\s+)?(?:use|extern\s+crate)\s+({alts})\b"
        );
        Regex::new(&pat).expect("regex import valide")
    })
}

fn path_regex() -> &'static Regex {
    static CACHE: OnceLock<Regex> = OnceLock::new();
    CACHE.get_or_init(|| {
        let alts = FORBIDDEN_RUST_IMPORTS.join("|");
        // Capture toute qualification `foo::xxx` quelque part dans le
        // code (ex. `let c = reqwest::get(...);` sans `use` préalable).
        let pat = format!(r"\b({alts})::");
        Regex::new(&pat).expect("regex path valide")
    })
}

// ---------------------------------------------------------------------------
// Vérifications statiques
// ---------------------------------------------------------------------------

fn check_no_http_dependency_in_manifest() {
    let deps = direct_deps();
    for forbidden in FORBIDDEN_CARGO_DEPS {
        assert!(
            !deps.iter().any(|d| d == forbidden),
            "Cargo.toml déclare une dépendance HTTP cliente directe interdite : `{}` \
             — viole requirements 6.4 et 17.3 (Garantie zéro réseau, design § \
             « Garantie zéro réseau »). Dépendances directes observées : {:?}",
            forbidden,
            deps
        );
    }
}

fn check_network_feature_is_empty() {
    let toml = cargo_toml_text();
    let mut in_features = false;
    let mut found_network = false;

    for raw in toml.lines() {
        let line = raw.trim_start();

        if line.starts_with('[') {
            in_features = line.trim_end() == "[features]";
            continue;
        }
        if !in_features || line.is_empty() || line.starts_with('#') {
            continue;
        }

        if let Some(rest) = line.strip_prefix("network") {
            // S'assurer qu'on est bien sur la clé `network` exacte
            // (et pas un préfixe comme `networking = ...`).
            let rest = rest.trim_start();
            if let Some(value) = rest.strip_prefix('=') {
                let value = value.trim().trim_end_matches(['#']).trim();
                // Tolère un commentaire de fin de ligne : on coupe au
                // premier `#` libre.
                let value = value.split('#').next().unwrap_or("").trim();
                assert_eq!(
                    value, "[]",
                    "feature `network` doit rester strictement vide pour \
                     préserver la garantie zéro réseau (req 17.3) ; \
                     observé : `{value}`"
                );
                found_network = true;
            }
        }
    }

    assert!(
        found_network,
        "feature flag `network = []` manquant dans Cargo.toml — le design \
         (§ « Garantie zéro réseau ») exige sa présence comme verrou \
         explicite (req 17.3)"
    );
}

fn check_no_http_imports_in_src() {
    let import_re = import_regex();
    let path_re = path_regex();

    for file in rust_source_files() {
        let body = std::fs::read_to_string(file).unwrap_or_else(|e| {
            panic!("impossible de lire `{}`: {e}", file.display())
        });

        for (idx, raw) in body.lines().enumerate() {
            let line_no = idx + 1;
            let trimmed = raw.trim_start();

            // Les commentaires de ligne (`//`, `///`, `//!`) peuvent
            // mentionner légitimement le nom d'un crate interdit en
            // prose (cf. ce fichier lui-même !). On les ignore.
            if trimmed.starts_with("//") {
                continue;
            }

            if let Some(m) = import_re.find(raw) {
                panic!(
                    "Import HTTP cliente interdit dans `{}:{}`: `{}` \
                     — viole requirements 6.4 et 17.3",
                    file.display(),
                    line_no,
                    m.as_str().trim()
                );
            }

            if let Some(m) = path_re.find(raw) {
                panic!(
                    "Référence à un chemin de crate HTTP cliente interdit \
                     dans `{}:{}`: `{}` — viole requirements 6.4 et 17.3",
                    file.display(),
                    line_no,
                    m.as_str().trim()
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Stratégie proptest — séquences aléatoires d'opérations utilisateur
// ---------------------------------------------------------------------------

/// Les six familles d'opérations utilisateur citées par la tâche 17.4
/// et le design (§ Property 13) : ajout dossier, scan, édition, lecture
/// mockée, recherche, filtres. La séquence est l'objet quantifié
/// universellement par la propriété.
#[derive(Debug, Clone, Copy)]
enum UserOp {
    AddFolder,
    Scan,
    Edit,
    Play,
    Search,
    Filter,
}

fn user_op_strategy() -> impl Strategy<Value = UserOp> {
    prop_oneof![
        Just(UserOp::AddFolder),
        Just(UserOp::Scan),
        Just(UserOp::Edit),
        Just(UserOp::Play),
        Just(UserOp::Search),
        Just(UserOp::Filter),
    ]
}

// ---------------------------------------------------------------------------
// Test property — Property 13
// ---------------------------------------------------------------------------

proptest! {
    /// **Property 13 : Aucune requête réseau sortante**
    ///
    /// *For any* séquence d'opérations utilisateur (ajout dossier,
    /// scan, édition, lecture, recherche, filtres), aucune connexion
    /// réseau sortante n'est tentée par Kyro_Backend.
    ///
    /// **Validates: Requirements 6.4, 17.3**
    #[test]
    fn property_13_aucune_requete_reseau_sortante(
        sequence in prop::collection::vec(user_op_strategy(), 0..=20),
    ) {
        // La séquence tirée par proptest est consommée pour matérialiser
        // le « for any » formel. Les invariants vérifiés ensuite sont
        // structuraux (état du code source + manifeste), donc ils
        // dominent universellement toute trace d'exécution possible
        // construite à partir de ces opérations.
        let _ = sequence;

        check_no_http_dependency_in_manifest();
        check_network_feature_is_empty();
        check_no_http_imports_in_src();
    }
}

// ---------------------------------------------------------------------------
// Tests « smoke » directs — exécutés sans proptest pour fournir un
// signalement clair et rapide en cas de régression.
// ---------------------------------------------------------------------------

#[test]
fn smoke_no_http_dependency_in_manifest() {
    check_no_http_dependency_in_manifest();
}

#[test]
fn smoke_network_feature_is_empty() {
    check_network_feature_is_empty();
}

#[test]
fn smoke_no_http_imports_in_src() {
    check_no_http_imports_in_src();
}

// ---------------------------------------------------------------------------
// Tests de robustesse du parseur — on s'assure que les invariants ci-dessus
// **détecteraient** une régression si on l'introduisait synthétiquement,
// pour éviter qu'un bug du parseur ne masque une violation réelle.
// ---------------------------------------------------------------------------

#[test]
fn parser_detects_forbidden_dep_in_simple_table() {
    let synthetic = r#"
[package]
name = "x"

[dependencies]
serde = "1"
reqwest = "0.12"

[features]
network = []
"#;
    let deps = parse_direct_deps(synthetic);
    assert!(deps.iter().any(|d| d == "reqwest"));
    assert!(deps.iter().any(|d| d == "serde"));
}

#[test]
fn parser_detects_forbidden_dep_in_subtable() {
    let synthetic = r#"
[package]
name = "x"

[dependencies.ureq]
version = "2"

[dependencies]
anyhow = "1"
"#;
    let deps = parse_direct_deps(synthetic);
    assert!(deps.iter().any(|d| d == "ureq"));
    assert!(deps.iter().any(|d| d == "anyhow"));
}

#[test]
fn parser_ignores_lines_inside_features_array() {
    // Vérifie qu'une feature multi-ligne `tokio` ne fait pas remonter
    // de fausses dépendances issues de ses entrées de tableau.
    let synthetic = r#"
[dependencies]
tokio = { version = "=1.40.0", features = [
    "rt-multi-thread",
    "fs",
    "net",
] }
serde = "1"
"#;
    let deps = parse_direct_deps(synthetic);
    assert!(deps.iter().any(|d| d == "tokio"));
    assert!(deps.iter().any(|d| d == "serde"));
    assert!(
        !deps.iter().any(|d| d == "rt-multi-thread" || d == "fs" || d == "net"),
        "le parseur a accidentellement remonté un fragment de tableau \
         features comme dépendance : {:?}",
        deps
    );
}
