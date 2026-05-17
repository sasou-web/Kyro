//! Tâche 12.4 — commandes Tauri du domaine **paramètres** (Settings).
//!
//! Ce module expose au frontend SvelteKit l'ensemble des commandes
//! permettant de configurer Kyro :
//!
//! * **Dossiers de bibliothèque** — `add_library_folder`,
//!   `remove_library_folder`, `list_library_folders`. Délèguent
//!   intégralement à la couche `infra::repo::Repo` (tâche 7.1) qui
//!   couvre déjà les requirements 2.1 → 2.5 et 7.1 (canonicalisation
//!   du chemin, refus si non dossier, marquage `available = 0` au
//!   retrait, etc.).
//! * **Lecteur externe** — `set_external_player_path`,
//!   `get_external_player_path`. La validation du chemin
//!   (existence + extension `.exe` insensible à la casse, requirement
//!   10.5) est portée par
//!   [`crate::infra::player_bridge::validate_external_player_path`] et
//!   appliquée **avant** toute écriture.
//! * **Chemin ffprobe manuel** — `set_ffprobe_path` (requirement 5.4).
//!   Valide existence + extension `.exe` via le même helper que pour le
//!   lecteur externe (par convention partagée Windows).
//! * **Scan au démarrage** — `set_scan_at_startup` (requirement 3.7,
//!   stocké comme `"true"` / `"false"`).
//! * **Thème** — `set_theme` / `get_theme` (requirements 16.1, 16.14).
//!   Valeurs acceptées : `"adwaita-dark"` ou `"adwaita-light"` ;
//!   défaut `"adwaita-dark"` au tout premier démarrage.
//! * **Dernière catégorie** — `set_last_category` /
//!   `get_last_category` (requirement 7.1). Valeurs acceptées :
//!   littéraux `Category::as_str` (`"Films"`, `"Series"`, `"Animes"`).
//! * **Résumé agrégé** — `get_settings_summary` retourne un DTO
//!   [`SettingsSummary`] regroupant l'ensemble des paramètres lus en
//!   une seule fois par la page Paramètres du frontend.
//! * **Initialisation premier démarrage** — [`init_default_settings`]
//!   tente de détecter `mpv.net.exe` aux emplacements d'installation
//!   standards (`%PROGRAMFILES%\mpv.net\mpv.net.exe`,
//!   `%LOCALAPPDATA%\mpv.net\mpv.net.exe`) et l'enregistre comme
//!   `external_player_path` si la clé est absente. Cf. requirement
//!   10.2 : « mpv.net défini comme valeur par défaut à la première
//!   installation ».
//!
//! ## Architecture pure-logique → adaptateur Tauri
//!
//! Chaque commande est scindée en deux niveaux :
//!
//! * Un helper `pub(crate) async fn ...` qui prend un `&Repo`
//!   directement — c'est ce helper qui porte la logique métier
//!   (validation, sérialisation valeur ↔ string, défauts) et qui est
//!   ciblé par les tests unitaires.
//! * Un wrapper `#[tauri::command] pub async fn ...` qui se contente
//!   d'extraire le `&Repo` du `tauri::State` puis délègue au helper.
//!
//! Cette séparation permet aux tests d'exercer la totalité du module
//! sans avoir besoin d'un `tauri::AppHandle` ni d'un runtime Tauri.
//!
//! ## Note registration
//!
//! Conformément à la note du module parent (`commands/mod.rs`), la
//! registration sur le `tauri::Builder` (`invoke_handler`) est
//! **explicitement déléguée** à une tâche ultérieure. Ce module se
//! contente d'exposer les fonctions et le DTO `SettingsSummary`.
//!
//! _Requirements: 2.1, 2.2, 2.3, 2.4, 2.5, 5.4, 7.1, 10.1, 10.2, 10.3,
//! 10.4, 16.1, 16.14_

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::domain::types::Category;
use crate::error::KyroError;
use crate::infra::player_bridge::validate_external_player_path;
use crate::infra::repo::{FolderId, LibraryFolder, Repo};
use crate::infra::settings_keys::{
    SETTING_KEY_EXTERNAL_PLAYER_PATH, SETTING_KEY_FFPROBE_PATH,
    SETTING_KEY_LAST_CATEGORY, SETTING_KEY_SCAN_AT_STARTUP, SETTING_KEY_THEME,
};

// =====================================================================
// Constantes — valeurs littérales acceptées et défauts.
// =====================================================================

/// Valeur **par défaut** du thème au tout premier démarrage
/// (requirement 16.1). Cette constante est aussi le repli de
/// [`get_theme_with`] lorsque la clé `theme` n'est pas encore en base.
pub const DEFAULT_THEME: &str = "adwaita-dark";

/// Valeur **alternative** du thème (requirement 16.14).
pub const ALTERNATE_THEME: &str = "adwaita-light";

/// Allowlist exhaustive des valeurs acceptées pour la clé `theme`.
const ACCEPTED_THEMES: &[&str] = &[DEFAULT_THEME, ALTERNATE_THEME];

/// Allowlist des littéraux acceptés pour la clé `last_category`.
/// Les valeurs sont strictement celles produites par
/// [`Category::as_str`] (cohérent avec le `CHECK` SQL et le design).
const ACCEPTED_CATEGORIES: &[&str] = &["Films", "Series", "Animes"];

// =====================================================================
// DTO `SettingsSummary` — agrégat lu d'un coup par le frontend.
// =====================================================================

/// Vue agrégée de tous les paramètres persistés dans `setting_value`,
/// telle que la consomme la page Paramètres du frontend SvelteKit.
///
/// Les `Option<String>` reflètent la sémantique de
/// [`Repo::get_setting`] : `None` ⇔ clé absente. Le frontend choisira
/// l'affichage approprié (champ vide, placeholder, etc.) sans devoir
/// faire un appel par clé.
///
/// `theme` n'est pas optionnel : `get_settings_summary` applique le
/// défaut [`DEFAULT_THEME`] si la clé est absente, conformément au
/// requirement 16.1 (« valeur initiale = adwaita-dark »).
///
/// `scan_at_startup` est exposé en `bool` : la valeur stockée en base
/// est `"true"` ou `"false"` (cf. `set_scan_at_startup`), tout autre
/// littéral est considéré faux (politique tolérante côté lecture).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettingsSummary {
    /// Liste des dossiers de bibliothèque enregistrés (équivalent
    /// `list_library_folders`). `library_folder.id` / `path` /
    /// `category` sont projetés vers une forme sérialisable JSON.
    pub library_folders: Vec<SerializableFolder>,
    /// Chemin du lecteur externe enregistré (`None` si absent).
    pub external_player_path: Option<String>,
    /// Chemin manuel vers `ffprobe.exe` (`None` si absent).
    pub ffprobe_path: Option<String>,
    /// Drapeau « scan au démarrage » (requirement 3.7).
    pub scan_at_startup: bool,
    /// Thème actif. Toujours présent, défaut `"adwaita-dark"`.
    pub theme: String,
    /// Dernière catégorie consultée (`None` si jamais persistée).
    pub last_category: Option<String>,
}

/// Projection JSON-friendly de [`LibraryFolder`].
///
/// `LibraryFolder::id` est typé fort (`FolderId`) et `category` est un
/// `enum` ; l'IPC Tauri travaille en JSON, on dérive `Serialize` sur
/// une projection simple à champs primitifs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SerializableFolder {
    pub id: i64,
    pub path: String,
    pub category: &'static str,
}

impl From<&LibraryFolder> for SerializableFolder {
    fn from(f: &LibraryFolder) -> Self {
        Self {
            id: f.id.get(),
            path: f.path.clone(),
            category: f.category.as_str(),
        }
    }
}

// =====================================================================
// Helpers pure-logique (`*_with`) — ciblés par les tests unitaires.
// =====================================================================

/// Parse un littéral de catégorie (`"Films"`, `"Series"`, `"Animes"`)
/// vers une variante [`Category`]. Sensible à la casse (cohérent avec
/// le `CHECK` SQL).
fn parse_category_literal(raw: &str) -> Result<Category, KyroError> {
    match raw {
        "Films" => Ok(Category::Films),
        "Series" => Ok(Category::Series),
        "Animes" => Ok(Category::Animes),
        other => Err(KyroError::Validation(format!(
            "catégorie inconnue: {other:?} (attendu: Films, Series, Animes)"
        ))),
    }
}

/// Helper pure-logique pour `add_library_folder`. Délègue à
/// [`Repo::add_library_folder`] qui couvre déjà les requirements 2.2
/// (canonicalisation), 2.4 (refus si chemin invalide) et 2.5
/// (catégorie persistée).
pub(crate) async fn add_library_folder_with(
    repo: &Repo,
    path: &str,
    category: &str,
) -> Result<i64, KyroError> {
    let category = parse_category_literal(category)?;
    let folder_id = repo
        .add_library_folder(Path::new(path), category)
        .await?;
    Ok(folder_id.get())
}

/// Helper pure-logique pour `remove_library_folder`. Délègue
/// directement à [`Repo::remove_library_folder`] (requirement 2.3).
pub(crate) async fn remove_library_folder_with(
    repo: &Repo,
    id: i64,
) -> Result<(), KyroError> {
    repo.remove_library_folder(FolderId(id)).await
}

/// Helper pure-logique pour `list_library_folders` (requirement 2.1),
/// avec projection vers le DTO [`SerializableFolder`].
pub(crate) async fn list_library_folders_with(
    repo: &Repo,
) -> Result<Vec<SerializableFolder>, KyroError> {
    let folders = repo.list_library_folders().await?;
    Ok(folders.iter().map(SerializableFolder::from).collect())
}

/// Helper pure-logique pour `set_external_player_path`. Valide
/// existence + extension `.exe` (requirement 10.5) **avant** d'écrire
/// la clé `external_player_path`.
pub(crate) async fn set_external_player_path_with(
    repo: &Repo,
    path: &str,
) -> Result<(), KyroError> {
    validate_external_player_path(Path::new(path))?;
    repo.upsert_setting(SETTING_KEY_EXTERNAL_PLAYER_PATH, path)
        .await
}

/// Helper pure-logique pour `get_external_player_path`. Retourne la
/// valeur brute persistée (sans validation : un chemin précédemment
/// valide peut être devenu invalide entre temps — la commande
/// `play_media` re-validera au moment du lancement, requirement 10.4).
pub(crate) async fn get_external_player_path_with(
    repo: &Repo,
) -> Result<Option<String>, KyroError> {
    repo.get_setting(SETTING_KEY_EXTERNAL_PLAYER_PATH).await
}

/// Helper pure-logique pour `set_ffprobe_path` (requirement 5.4).
/// Applique la même validation que pour le lecteur externe : existence
/// + extension `.exe` insensible à la casse (le binaire ffprobe sur
/// Windows est `ffprobe.exe`).
pub(crate) async fn set_ffprobe_path_with(
    repo: &Repo,
    path: &str,
) -> Result<(), KyroError> {
    validate_external_player_path(Path::new(path))?;
    repo.upsert_setting(SETTING_KEY_FFPROBE_PATH, path).await
}

/// Helper pure-logique pour `set_scan_at_startup` (requirement 3.7).
/// Stocke `"true"` ou `"false"` selon la valeur du booléen — chaîne
/// canonique consommée par le hook de boot (`startup.rs`).
pub(crate) async fn set_scan_at_startup_with(
    repo: &Repo,
    enabled: bool,
) -> Result<(), KyroError> {
    let value = if enabled { "true" } else { "false" };
    repo.upsert_setting(SETTING_KEY_SCAN_AT_STARTUP, value)
        .await
}

/// Helper pure-logique pour `set_theme` (requirements 16.1, 16.14).
/// Valide que `theme ∈ {"adwaita-dark", "adwaita-light"}` ; tout autre
/// littéral est rejeté avec [`KyroError::Validation`].
pub(crate) async fn set_theme_with(repo: &Repo, theme: &str) -> Result<(), KyroError> {
    if !ACCEPTED_THEMES.contains(&theme) {
        return Err(KyroError::Validation(format!(
            "thème inconnu: {theme:?} (attendu: adwaita-dark, adwaita-light)"
        )));
    }
    repo.upsert_setting(SETTING_KEY_THEME, theme).await
}

/// Helper pure-logique pour `get_theme`. Retourne [`DEFAULT_THEME`] si
/// la clé `theme` n'est pas encore présente en base (requirement 16.1).
pub(crate) async fn get_theme_with(repo: &Repo) -> Result<String, KyroError> {
    Ok(repo
        .get_setting(SETTING_KEY_THEME)
        .await?
        .unwrap_or_else(|| DEFAULT_THEME.to_string()))
}

/// Helper pure-logique pour `set_last_category` (requirement 7.1).
/// N'accepte que les littéraux `Category::as_str` (`Films`, `Series`,
/// `Animes`) — sensible à la casse, cohérent avec le `CHECK` SQL.
pub(crate) async fn set_last_category_with(
    repo: &Repo,
    category: &str,
) -> Result<(), KyroError> {
    if !ACCEPTED_CATEGORIES.contains(&category) {
        return Err(KyroError::Validation(format!(
            "catégorie inconnue: {category:?} (attendu: Films, Series, Animes)"
        )));
    }
    repo.upsert_setting(SETTING_KEY_LAST_CATEGORY, category)
        .await
}

/// Helper pure-logique pour `get_last_category`. Retourne `None` si
/// l'utilisateur n'a encore jamais ouvert de catégorie (cas typique :
/// premier lancement après `init_default_settings`).
pub(crate) async fn get_last_category_with(
    repo: &Repo,
) -> Result<Option<String>, KyroError> {
    repo.get_setting(SETTING_KEY_LAST_CATEGORY).await
}

/// Helper pure-logique pour `get_settings_summary` — agrège en une
/// seule opération l'ensemble des paramètres lus séparément ailleurs.
///
/// La lecture est faite en série (les opérations sont rapides : un
/// `SELECT` indexé par PK chacune) afin de garder la fonction simple
/// et la trace `tracing` lisible. Si la performance devient un sujet,
/// `try_join_all` permettrait de paralléliser sans changer la
/// signature publique.
pub(crate) async fn get_settings_summary_with(
    repo: &Repo,
) -> Result<SettingsSummary, KyroError> {
    let library_folders = list_library_folders_with(repo).await?;
    let external_player_path = get_external_player_path_with(repo).await?;
    let ffprobe_path = repo.get_setting(SETTING_KEY_FFPROBE_PATH).await?;
    let scan_at_startup = match repo.get_setting(SETTING_KEY_SCAN_AT_STARTUP).await? {
        Some(s) => s == "true",
        None => false,
    };
    let theme = get_theme_with(repo).await?;
    let last_category = get_last_category_with(repo).await?;

    Ok(SettingsSummary {
        library_folders,
        external_player_path,
        ffprobe_path,
        scan_at_startup,
        theme,
        last_category,
    })
}

// =====================================================================
// Initialisation premier démarrage — détection mpv.net.
// =====================================================================

/// Sous-chemin standard `mpv.net\mpv.net.exe` au sein de la racine
/// d'installation (`%PROGRAMFILES%`, `%LOCALAPPDATA%`, …).
const MPV_NET_EXE_SUBPATH: &[&str] = &["mpv.net", "mpv.net.exe"];

/// Construit la liste ordonnée des chemins candidats pour l'auto-détection
/// de mpv.net au premier démarrage. L'ordre reflète la probabilité
/// d'installation : d'abord `%PROGRAMFILES%` (installation système,
/// classique), puis `%LOCALAPPDATA%` (installation par utilisateur via
/// MSI scope « per-user »).
///
/// Exposé `pub(crate)` pour permettre aux tests unitaires d'observer
/// la stratégie de résolution sans dépendre de l'environnement réel.
pub(crate) fn mpv_net_default_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for var in &["PROGRAMFILES", "LOCALAPPDATA"] {
        if let Some(root) = std::env::var_os(var) {
            let mut p = PathBuf::from(root);
            for seg in MPV_NET_EXE_SUBPATH {
                p.push(seg);
            }
            out.push(p);
        }
    }
    out
}

/// Sélectionne le premier chemin candidat qui existe et passe la
/// validation [`validate_external_player_path`]. Retourne `None` si
/// aucun candidat n'est exploitable.
fn detect_mpv_net_executable(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates
        .iter()
        .find(|p| validate_external_player_path(p).is_ok())
        .cloned()
}

/// Initialise les paramètres au tout premier démarrage : si la clé
/// `external_player_path` est absente de `setting_value`, tente une
/// détection automatique de `mpv.net.exe` aux emplacements
/// d'installation standards Windows et persiste le chemin trouvé.
///
/// Si aucun candidat n'est exploitable (mpv.net pas installé, chemin
/// non standard), la clé reste **non définie** : le frontend pourra
/// alors guider l'utilisateur via la page Paramètres pour qu'il
/// pointe son lecteur préféré (requirement 10.1).
///
/// Cette fonction est **idempotente** : elle ne fait rien si la clé
/// est déjà présente — l'utilisateur peut donc supprimer
/// `mpv.net.exe` du chemin par défaut sans que le boot le remette en
/// place. Elle est non-fatale : toute erreur (lecture base, écriture
/// base, validation chemin) est journalisée et la fonction retourne
/// silencieusement, cohérent avec la politique du hook
/// `scan_at_startup` (cf. `startup.rs`).
///
/// _Requirements: 10.1, 10.2_
pub async fn init_default_settings(repo: &Repo) {
    let already_set = match repo.get_setting(SETTING_KEY_EXTERNAL_PLAYER_PATH).await {
        Ok(v) => v.is_some(),
        Err(err) => {
            tracing::warn!(
                error = %err,
                "Kyro_Settings: lecture external_player_path échouée — init_default_settings ignoré"
            );
            return;
        }
    };
    if already_set {
        tracing::debug!(
            "Kyro_Settings: external_player_path déjà défini, init_default_settings no-op"
        );
        return;
    }

    let candidates = mpv_net_default_candidates();
    let Some(found) = detect_mpv_net_executable(&candidates) else {
        tracing::info!(
            candidates = ?candidates,
            "Kyro_Settings: mpv.net introuvable aux emplacements standards — \
             external_player_path laissé non défini"
        );
        return;
    };

    let value = found.to_string_lossy().into_owned();
    if let Err(err) = repo
        .upsert_setting(SETTING_KEY_EXTERNAL_PLAYER_PATH, &value)
        .await
    {
        tracing::warn!(
            path = %value,
            error = %err,
            "Kyro_Settings: échec persistance external_player_path par défaut (ignoré)"
        );
        return;
    }

    tracing::info!(
        path = %value,
        "Kyro_Settings: external_player_path initialisé à mpv.net détecté automatiquement"
    );
}

// =====================================================================
// Commandes Tauri — wrappers fins autour des helpers `*_with`.
// =====================================================================

/// Commande Tauri : ajoute un dossier de bibliothèque (requirements
/// 2.2, 2.4, 2.5, 7.1).
#[tauri::command]
pub async fn add_library_folder(
    path: String,
    category: String,
    repo: tauri::State<'_, Repo>,
    watcher_state: tauri::State<'_, std::sync::Arc<crate::infra::watcher::WatcherState>>,
) -> Result<i64, KyroError> {
    let id = add_library_folder_with(&repo, &path, &category).await?;
    // Le watcher est rechargé après ajout pour qu'il commence à
    // surveiller le nouveau dossier sans redémarrer l'app.
    let state_clone = watcher_state.inner().clone();
    let repo_clone = repo.inner().clone();
    tauri::async_runtime::spawn(async move {
        crate::infra::watcher::reload_watches(state_clone, repo_clone).await;
    });
    Ok(id)
}

/// Commande Tauri : retire un dossier de bibliothèque, marque ses
/// items `available = 0` (requirement 2.3).
#[tauri::command]
pub async fn remove_library_folder(
    id: i64,
    repo: tauri::State<'_, Repo>,
    watcher_state: tauri::State<'_, std::sync::Arc<crate::infra::watcher::WatcherState>>,
) -> Result<(), KyroError> {
    remove_library_folder_with(&repo, id).await?;
    let state_clone = watcher_state.inner().clone();
    let repo_clone = repo.inner().clone();
    tauri::async_runtime::spawn(async move {
        crate::infra::watcher::reload_watches(state_clone, repo_clone).await;
    });
    Ok(())
}

/// Commande Tauri : liste tous les dossiers de bibliothèque
/// (requirement 2.1).
#[tauri::command]
pub async fn list_library_folders(
    repo: tauri::State<'_, Repo>,
) -> Result<Vec<SerializableFolder>, KyroError> {
    list_library_folders_with(&repo).await
}

/// Commande Tauri : enregistre le chemin du lecteur externe après
/// validation existence + extension `.exe` (requirements 10.1, 10.3,
/// 10.5).
#[tauri::command]
pub async fn set_external_player_path(
    path: String,
    repo: tauri::State<'_, Repo>,
) -> Result<(), KyroError> {
    set_external_player_path_with(&repo, &path).await
}

/// Commande Tauri : retourne le chemin du lecteur externe enregistré
/// (`None` si jamais défini).
#[tauri::command]
pub async fn get_external_player_path(
    repo: tauri::State<'_, Repo>,
) -> Result<Option<String>, KyroError> {
    get_external_player_path_with(&repo).await
}

/// Commande Tauri : enregistre le chemin manuel vers `ffprobe.exe`
/// (requirement 5.4).
#[tauri::command]
pub async fn set_ffprobe_path(
    path: String,
    repo: tauri::State<'_, Repo>,
) -> Result<(), KyroError> {
    set_ffprobe_path_with(&repo, &path).await
}

/// Commande Tauri : active / désactive le scan automatique au
/// démarrage (requirement 3.7).
#[tauri::command]
pub async fn set_scan_at_startup(
    enabled: bool,
    repo: tauri::State<'_, Repo>,
) -> Result<(), KyroError> {
    set_scan_at_startup_with(&repo, enabled).await
}

/// Commande Tauri : enregistre le thème actif (requirements 16.1,
/// 16.14).
#[tauri::command]
pub async fn set_theme(
    theme: String,
    repo: tauri::State<'_, Repo>,
) -> Result<(), KyroError> {
    set_theme_with(&repo, &theme).await
}

/// Commande Tauri : retourne le thème actif, défaut
/// [`DEFAULT_THEME`].
#[tauri::command]
pub async fn get_theme(repo: tauri::State<'_, Repo>) -> Result<String, KyroError> {
    get_theme_with(&repo).await
}

/// Commande Tauri : mémorise la dernière catégorie consultée
/// (requirement 7.1).
#[tauri::command]
pub async fn set_last_category(
    category: String,
    repo: tauri::State<'_, Repo>,
) -> Result<(), KyroError> {
    set_last_category_with(&repo, &category).await
}

/// Commande Tauri : retourne la dernière catégorie consultée
/// (`None` au premier démarrage).
#[tauri::command]
pub async fn get_last_category(
    repo: tauri::State<'_, Repo>,
) -> Result<Option<String>, KyroError> {
    get_last_category_with(&repo).await
}

/// Commande Tauri : retourne l'agrégat [`SettingsSummary`] consommé
/// par la page Paramètres du frontend.
#[tauri::command]
pub async fn get_settings_summary(
    repo: tauri::State<'_, Repo>,
) -> Result<SettingsSummary, KyroError> {
    get_settings_summary_with(&repo).await
}

// =====================================================================
// Tests unitaires.
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{
        SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions,
        SqliteSynchronous,
    };
    use tempfile::TempDir;

    /// Runtime `tokio` `current_thread` dédié à un test (cohérent avec
    /// la stratégie déjà utilisée dans `infra::repo::tests`).
    fn block_on<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("création runtime tokio")
            .block_on(fut)
    }

    /// Pool SQLite frais avec migrations appliquées.
    async fn fresh_pool(db_dir: &Path) -> SqlitePool {
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
            .expect("migrations test");
        pool
    }

    /// Crée un faux fichier `.exe` valide pour la validation
    /// `validate_external_player_path` (existence + extension `.exe`).
    fn make_fake_exe(tmp: &TempDir, name: &str) -> PathBuf {
        let p = tmp.path().join(name);
        std::fs::write(&p, b"MZ").expect("écriture faux exe");
        p
    }

    #[test]
    fn list_folders_empty_initially() {
        block_on(async {
            let db = TempDir::new().unwrap();
            let pool = fresh_pool(db.path()).await;
            let repo = Repo::new(pool);

            let folders = list_library_folders_with(&repo).await.unwrap();
            assert!(folders.is_empty());
        });
    }

    #[test]
    fn add_then_list_then_remove_library_folder() {
        block_on(async {
            let db = TempDir::new().unwrap();
            let lib = TempDir::new().unwrap();
            let pool = fresh_pool(db.path()).await;
            let repo = Repo::new(pool);

            let id = add_library_folder_with(
                &repo,
                lib.path().to_string_lossy().as_ref(),
                "Films",
            )
            .await
            .expect("ajout dossier ok");
            assert!(id > 0);

            let folders = list_library_folders_with(&repo).await.unwrap();
            assert_eq!(folders.len(), 1);
            assert_eq!(folders[0].id, id);
            assert_eq!(folders[0].category, "Films");

            remove_library_folder_with(&repo, id)
                .await
                .expect("retrait ok");
            let folders = list_library_folders_with(&repo).await.unwrap();
            assert!(folders.is_empty());
        });
    }

    #[test]
    fn add_library_folder_rejects_unknown_category() {
        block_on(async {
            let db = TempDir::new().unwrap();
            let lib = TempDir::new().unwrap();
            let pool = fresh_pool(db.path()).await;
            let repo = Repo::new(pool);

            let err = add_library_folder_with(
                &repo,
                lib.path().to_string_lossy().as_ref(),
                "Documentaire",
            )
            .await
            .expect_err("catégorie inconnue doit être rejetée");
            assert!(matches!(err, KyroError::Validation(_)));
        });
    }

    #[test]
    fn set_external_player_path_validates_then_persists() {
        block_on(async {
            let db = TempDir::new().unwrap();
            let exe_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db.path()).await;
            let repo = Repo::new(pool);

            // Aucune valeur initialement.
            let v = get_external_player_path_with(&repo).await.unwrap();
            assert!(v.is_none());

            let exe = make_fake_exe(&exe_dir, "mpv.net.exe");
            let exe_str = exe.to_string_lossy().into_owned();
            set_external_player_path_with(&repo, &exe_str)
                .await
                .expect("set ok");

            let v = get_external_player_path_with(&repo).await.unwrap();
            assert_eq!(v.as_deref(), Some(exe_str.as_str()));
        });
    }

    #[test]
    fn set_external_player_path_rejects_nonexistent_file() {
        block_on(async {
            let db = TempDir::new().unwrap();
            let pool = fresh_pool(db.path()).await;
            let repo = Repo::new(pool.clone());

            let err = set_external_player_path_with(
                &repo,
                "C:/chemin/inexistant/mpv.net.exe",
            )
            .await
            .expect_err("chemin inexistant doit être refusé");
            assert!(matches!(err, KyroError::Validation(_)));

            // Aucune ligne n'a été insérée par l'appel refusé.
            let count: i64 = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM setting_value WHERE key = ?",
            )
            .bind(SETTING_KEY_EXTERNAL_PLAYER_PATH)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(count, 0);
        });
    }

    #[test]
    fn set_external_player_path_rejects_non_exe_extension() {
        block_on(async {
            let db = TempDir::new().unwrap();
            let exe_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db.path()).await;
            let repo = Repo::new(pool);

            let bat = make_fake_exe(&exe_dir, "player.bat");
            let err = set_external_player_path_with(
                &repo,
                bat.to_string_lossy().as_ref(),
            )
            .await
            .expect_err("extension .bat doit être refusée");
            assert!(matches!(err, KyroError::Validation(_)));
        });
    }

    #[test]
    fn set_ffprobe_path_validates_extension() {
        block_on(async {
            let db = TempDir::new().unwrap();
            let exe_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db.path()).await;
            let repo = Repo::new(pool);

            let exe = make_fake_exe(&exe_dir, "ffprobe.exe");
            set_ffprobe_path_with(&repo, exe.to_string_lossy().as_ref())
                .await
                .expect("set ok");

            let v = repo.get_setting(SETTING_KEY_FFPROBE_PATH).await.unwrap();
            assert_eq!(v.as_deref(), Some(exe.to_string_lossy().as_ref()));
        });
    }

    #[test]
    fn set_scan_at_startup_stores_canonical_string() {
        block_on(async {
            let db = TempDir::new().unwrap();
            let pool = fresh_pool(db.path()).await;
            let repo = Repo::new(pool);

            set_scan_at_startup_with(&repo, true).await.unwrap();
            let v = repo.get_setting(SETTING_KEY_SCAN_AT_STARTUP).await.unwrap();
            assert_eq!(v.as_deref(), Some("true"));

            set_scan_at_startup_with(&repo, false).await.unwrap();
            let v = repo.get_setting(SETTING_KEY_SCAN_AT_STARTUP).await.unwrap();
            assert_eq!(v.as_deref(), Some("false"));
        });
    }

    #[test]
    fn theme_defaults_to_adwaita_dark_when_unset() {
        block_on(async {
            let db = TempDir::new().unwrap();
            let pool = fresh_pool(db.path()).await;
            let repo = Repo::new(pool);

            let theme = get_theme_with(&repo).await.unwrap();
            assert_eq!(theme, DEFAULT_THEME);
            assert_eq!(theme, "adwaita-dark");
        });
    }

    #[test]
    fn set_theme_accepts_each_allowed_value_and_rejects_others() {
        block_on(async {
            let db = TempDir::new().unwrap();
            let pool = fresh_pool(db.path()).await;
            let repo = Repo::new(pool);

            for ok in ["adwaita-dark", "adwaita-light"] {
                set_theme_with(&repo, ok)
                    .await
                    .unwrap_or_else(|_| panic!("thème {ok} doit être accepté"));
                assert_eq!(get_theme_with(&repo).await.unwrap(), ok);
            }

            for bogus in ["", "dark", "Adwaita-Dark", "solarized"] {
                let err = set_theme_with(&repo, bogus)
                    .await
                    .expect_err("thème inconnu doit être rejeté");
                assert!(matches!(err, KyroError::Validation(_)));
            }
        });
    }

    #[test]
    fn last_category_round_trip_each_value() {
        block_on(async {
            let db = TempDir::new().unwrap();
            let pool = fresh_pool(db.path()).await;
            let repo = Repo::new(pool);

            assert!(get_last_category_with(&repo).await.unwrap().is_none());

            for cat in ["Films", "Series", "Animes"] {
                set_last_category_with(&repo, cat).await.unwrap();
                assert_eq!(
                    get_last_category_with(&repo).await.unwrap().as_deref(),
                    Some(cat)
                );
            }
        });
    }

    #[test]
    fn set_last_category_rejects_unknown_value() {
        block_on(async {
            let db = TempDir::new().unwrap();
            let pool = fresh_pool(db.path()).await;
            let repo = Repo::new(pool);

            for bogus in ["", "films", "Documentaire", "ANIMES"] {
                let err = set_last_category_with(&repo, bogus)
                    .await
                    .expect_err("catégorie inconnue doit être rejetée");
                assert!(matches!(err, KyroError::Validation(_)));
            }
        });
    }

    #[test]
    fn settings_summary_aggregates_all_keys() {
        block_on(async {
            let db = TempDir::new().unwrap();
            let lib = TempDir::new().unwrap();
            let exe_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db.path()).await;
            let repo = Repo::new(pool);

            let exe = make_fake_exe(&exe_dir, "mpv.net.exe");
            let ffp = make_fake_exe(&exe_dir, "ffprobe.exe");

            add_library_folder_with(
                &repo,
                lib.path().to_string_lossy().as_ref(),
                "Series",
            )
            .await
            .unwrap();
            set_external_player_path_with(&repo, exe.to_string_lossy().as_ref())
                .await
                .unwrap();
            set_ffprobe_path_with(&repo, ffp.to_string_lossy().as_ref())
                .await
                .unwrap();
            set_scan_at_startup_with(&repo, true).await.unwrap();
            set_theme_with(&repo, "adwaita-light").await.unwrap();
            set_last_category_with(&repo, "Series").await.unwrap();

            let s = get_settings_summary_with(&repo).await.unwrap();
            assert_eq!(s.library_folders.len(), 1);
            assert_eq!(s.library_folders[0].category, "Series");
            assert_eq!(
                s.external_player_path.as_deref(),
                Some(exe.to_string_lossy().as_ref())
            );
            assert_eq!(
                s.ffprobe_path.as_deref(),
                Some(ffp.to_string_lossy().as_ref())
            );
            assert!(s.scan_at_startup);
            assert_eq!(s.theme, "adwaita-light");
            assert_eq!(s.last_category.as_deref(), Some("Series"));
        });
    }

    #[test]
    fn settings_summary_uses_defaults_when_keys_absent() {
        block_on(async {
            let db = TempDir::new().unwrap();
            let pool = fresh_pool(db.path()).await;
            let repo = Repo::new(pool);

            let s = get_settings_summary_with(&repo).await.unwrap();
            assert!(s.library_folders.is_empty());
            assert!(s.external_player_path.is_none());
            assert!(s.ffprobe_path.is_none());
            assert!(!s.scan_at_startup);
            assert_eq!(s.theme, DEFAULT_THEME);
            assert!(s.last_category.is_none());
        });
    }

    #[test]
    fn init_default_settings_is_noop_when_key_already_set() {
        block_on(async {
            let db = TempDir::new().unwrap();
            let exe_dir = TempDir::new().unwrap();
            let pool = fresh_pool(db.path()).await;
            let repo = Repo::new(pool);

            let exe = make_fake_exe(&exe_dir, "mpv.net.exe");
            let exe_str = exe.to_string_lossy().into_owned();
            set_external_player_path_with(&repo, &exe_str)
                .await
                .unwrap();

            // L'init ne doit pas écraser une valeur déjà choisie par
            // l'utilisateur même si mpv.net.exe est détectable
            // ailleurs sur le système.
            init_default_settings(&repo).await;

            let v = get_external_player_path_with(&repo).await.unwrap();
            assert_eq!(v.as_deref(), Some(exe_str.as_str()));
        });
    }

    #[test]
    fn init_default_settings_persists_detected_mpv_net() {
        block_on(async {
            let db = TempDir::new().unwrap();
            let pf = TempDir::new().unwrap();
            let pool = fresh_pool(db.path()).await;
            let repo = Repo::new(pool);

            // Place un faux mpv.net.exe sous `<tmp>/mpv.net/mpv.net.exe`
            // et pointe `PROGRAMFILES` dessus le temps de l'appel.
            let mpv_dir = pf.path().join("mpv.net");
            std::fs::create_dir_all(&mpv_dir).unwrap();
            let mpv_exe = mpv_dir.join("mpv.net.exe");
            std::fs::write(&mpv_exe, b"MZ").unwrap();

            let original_pf = std::env::var_os("PROGRAMFILES");
            let original_lad = std::env::var_os("LOCALAPPDATA");
            std::env::set_var("PROGRAMFILES", pf.path());
            // On retire LOCALAPPDATA pour s'assurer que c'est bien le
            // candidat PROGRAMFILES qui est sélectionné.
            std::env::remove_var("LOCALAPPDATA");

            init_default_settings(&repo).await;

            // Restauration env avant assertions pour ne pas polluer
            // d'autres tests si une assertion panique.
            match original_pf {
                Some(v) => std::env::set_var("PROGRAMFILES", v),
                None => std::env::remove_var("PROGRAMFILES"),
            }
            match original_lad {
                Some(v) => std::env::set_var("LOCALAPPDATA", v),
                None => std::env::remove_var("LOCALAPPDATA"),
            }

            let v = get_external_player_path_with(&repo).await.unwrap();
            assert_eq!(v.as_deref(), Some(mpv_exe.to_string_lossy().as_ref()));
        });
    }

    #[test]
    fn init_default_settings_leaves_key_absent_when_no_candidate() {
        block_on(async {
            let db = TempDir::new().unwrap();
            let pool = fresh_pool(db.path()).await;
            let repo = Repo::new(pool);

            // Aucun candidat plausible : on pointe les variables sur
            // un dossier vide pour la durée du test.
            let empty = TempDir::new().unwrap();
            let original_pf = std::env::var_os("PROGRAMFILES");
            let original_lad = std::env::var_os("LOCALAPPDATA");
            std::env::set_var("PROGRAMFILES", empty.path());
            std::env::set_var("LOCALAPPDATA", empty.path());

            init_default_settings(&repo).await;

            match original_pf {
                Some(v) => std::env::set_var("PROGRAMFILES", v),
                None => std::env::remove_var("PROGRAMFILES"),
            }
            match original_lad {
                Some(v) => std::env::set_var("LOCALAPPDATA", v),
                None => std::env::remove_var("LOCALAPPDATA"),
            }

            let v = get_external_player_path_with(&repo).await.unwrap();
            assert!(v.is_none());
        });
    }

    #[test]
    fn mpv_net_default_candidates_use_standard_subpath() {
        // Ce test ne dépend pas des variables d'env (il vérifie
        // simplement que les chemins produits se terminent par
        // mpv.net\mpv.net.exe). On configure les deux variables pour
        // garantir que les deux candidats sont produits.
        let pf = TempDir::new().unwrap();
        let lad = TempDir::new().unwrap();
        let original_pf = std::env::var_os("PROGRAMFILES");
        let original_lad = std::env::var_os("LOCALAPPDATA");
        std::env::set_var("PROGRAMFILES", pf.path());
        std::env::set_var("LOCALAPPDATA", lad.path());

        let candidates = mpv_net_default_candidates();

        match original_pf {
            Some(v) => std::env::set_var("PROGRAMFILES", v),
            None => std::env::remove_var("PROGRAMFILES"),
        }
        match original_lad {
            Some(v) => std::env::set_var("LOCALAPPDATA", v),
            None => std::env::remove_var("LOCALAPPDATA"),
        }

        assert_eq!(candidates.len(), 2);
        for c in &candidates {
            assert!(
                c.ends_with(Path::new("mpv.net").join("mpv.net.exe")),
                "candidat inattendu : {c:?}"
            );
        }
    }
}
