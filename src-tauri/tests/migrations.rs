//! Tâche 2.4 — tests d'intégration pour le runner de migrations SQLx.
//!
//! Vérifie qu'une base SQLite neuve, ouverte avec les mêmes pragmas que
//! `init_sqlite_pool` (`journal_mode=WAL`, `synchronous=NORMAL`,
//! `foreign_keys=ON`, `create_if_missing=true`), reçoit toutes les
//! migrations versionnées présentes dans `src-tauri/migrations/` et que
//! le schéma résultant comporte les tables, index et contraintes CHECK
//! attendus par le design (§ « Persistance — DDL SQLite »).
//!
//! Ces tests vivent dans `src-tauri/tests/migrations.rs` et sont
//! exécutés en tant qu'**integration tests** Cargo : ils sont compilés
//! comme une crate distincte, mais `CARGO_MANIFEST_DIR` reste
//! `src-tauri/` donc le chemin passé à `sqlx::migrate!` est identique à
//! celui de `main.rs` (`./migrations`).
//!
//! Note : la dépendance `tokio` du projet n'active pas la feature
//! `macros` (cf. `Cargo.toml`), donc on ne peut pas annoter les tests
//! avec `#[tokio::test]`. À la place, chaque test instancie un runtime
//! `current_thread` localement via `tokio::runtime::Builder` — c'est
//! exactement la stratégie utilisée par `tauri::async_runtime::block_on`
//! côté `main.rs`, ce qui maintient les tests proches du chemin
//! d'exécution réel.
//!
//! Validates: Requirements 17.1 (Kyro_Library stockée localement —
//! schéma initial complet) et 17.2 (toutes les modifications persistées
//! — runner appliqué avec succès).

use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
};
use std::path::Path;
use tempfile::TempDir;

/// Construit un runtime `tokio` `current_thread` dédié au test. On
/// n'utilise pas `#[tokio::test]` car la feature `macros` de `tokio`
/// n'est pas activée dans `Cargo.toml` (volontairement, pour limiter la
/// surface de dépendances — cf. requirements 6.4 / 17.3).
fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("création d'un runtime tokio current_thread")
        .block_on(fut)
}

/// Ouvre un pool SQLite neuf sur un chemin temporaire, en reproduisant
/// fidèlement les pragmas de `init_sqlite_pool` (cf. `src/main.rs`).
async fn open_pool(db_path: &Path) -> SqlitePool {
    let connect_options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true);

    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(connect_options)
        .await
        .expect("ouverture du pool SQLite temporaire")
}

#[test]
fn fresh_database_applies_all_migrations_and_creates_expected_schema() {
    block_on(async {
        // 1. DB neuve sur un dossier temporaire (jamais en mémoire afin
        //    de couvrir le cas réel d'un fichier `library.db` sur disque,
        //    comme en production).
        let tmp = TempDir::new().expect("création du dossier temporaire");
        let db_path = tmp.path().join("library.db");
        let pool = open_pool(&db_path).await;

        // 2. Application des migrations versionnées. Le chemin est
        //    résolu à la compile-time relativement à
        //    `CARGO_MANIFEST_DIR` (`src-tauri/`), exactement comme dans
        //    `main.rs`.
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("exécution du runner de migrations sur DB neuve");

        // 3a. Toutes les tables métier sont présentes (on ignore la
        //     table interne `_sqlx_migrations` ainsi que les tables
        //     internes SQLite préfixées `sqlite_*`).
        let tables: Vec<String> = sqlx::query_scalar::<_, String>(
            "SELECT name FROM sqlite_master \
             WHERE type = 'table' \
               AND name NOT LIKE '\\_sqlx\\_%' ESCAPE '\\' \
               AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' \
             ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .expect("lecture sqlite_master pour les tables");

        let expected_tables = [
            "audio_track",
            "episode_item",
            "genre",
            "library_folder",
            "media_genre",
            "media_item",
            "setting_value",
            "subtitle_track",
        ];
        for expected in expected_tables {
            assert!(
                tables.iter().any(|t| t == expected),
                "table `{}` manquante après migrations ; tables présentes = {:?}",
                expected,
                tables
            );
        }

        // 3b. Tous les index nommés du design sont présents. On filtre
        //     `sql IS NOT NULL` pour ne conserver que les index créés
        //     explicitement par la migration (les index auto-générés
        //     pour `UNIQUE` ont `sql = NULL` et un nom préfixé
        //     `sqlite_autoindex_`).
        let explicit_indexes: Vec<String> = sqlx::query_scalar::<_, String>(
            "SELECT name FROM sqlite_master \
             WHERE type = 'index' AND sql IS NOT NULL \
             ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .expect("lecture sqlite_master pour les index explicites");

        let expected_indexes = [
            "idx_episode_series",
            "idx_media_item_status",
            "idx_media_item_title",
            "idx_media_item_type",
            "idx_media_item_year",
        ];
        for expected in expected_indexes {
            assert!(
                explicit_indexes.iter().any(|i| i == expected),
                "index `{}` manquant après migrations ; index explicites présents = {:?}",
                expected,
                explicit_indexes
            );
        }

        // 3c. Les contraintes UNIQUE déclarées dans le DDL produisent
        //     leurs auto-index : on vérifie au moins la présence de
        //     l'auto-index associé à `library_folder.path` et
        //     `episode_item.file_path` en interrogeant l'ensemble
        //     complet `sqlite_master` (sql peut être NULL pour les
        //     autoindex).
        let all_indexes: Vec<String> = sqlx::query_scalar::<_, String>(
            "SELECT name FROM sqlite_master WHERE type = 'index' ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .expect("lecture sqlite_master pour tous les index (y compris autoindex)");
        assert!(
            all_indexes
                .iter()
                .any(|i| i.starts_with("sqlite_autoindex_library_folder")),
            "auto-index UNIQUE pour `library_folder.path` manquant ; index = {:?}",
            all_indexes
        );
        assert!(
            all_indexes
                .iter()
                .any(|i| i.starts_with("sqlite_autoindex_episode_item")),
            "auto-index UNIQUE pour `episode_item.file_path` manquant ; index = {:?}",
            all_indexes
        );

        // 4. Validation rapide d'une contrainte CHECK :
        //    `library_folder.category` doit refuser une valeur hors
        //    {Films, Series, Animes}.
        sqlx::query("INSERT INTO library_folder (path, category) VALUES (?, ?)")
            .bind("C:/Users/Test/Films")
            .bind("Films")
            .execute(&pool)
            .await
            .expect("insertion d'une catégorie valide doit réussir");

        let bad_insert =
            sqlx::query("INSERT INTO library_folder (path, category) VALUES (?, ?)")
                .bind("C:/Users/Test/Foo")
                .bind("Foo")
                .execute(&pool)
                .await;
        assert!(
            bad_insert.is_err(),
            "insertion d'une catégorie invalide (`Foo`) devrait échouer sur la \
             contrainte CHECK, mais a réussi : {:?}",
            bad_insert
        );

        pool.close().await;
    });
}

#[test]
fn migrations_are_idempotent_when_run_twice() {
    block_on(async {
        // Lancer le runner deux fois sur la même DB ne doit produire ni
        // erreur ni divergence d'état (le runner sqlx repose sur la
        // table `_sqlx_migrations` pour ignorer les versions déjà
        // appliquées — c'est précisément ce que valide ce test,
        // requirement 17.2).
        let tmp = TempDir::new().expect("création du dossier temporaire");
        let db_path = tmp.path().join("library.db");
        let pool = open_pool(&db_path).await;

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("première exécution du runner");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("seconde exécution du runner — doit être idempotente");

        // Vérification qu'on a bien exactement le même nombre d'entrées
        // de migration appliquées qu'attendu. Compte dynamiquement le
        // nombre de fichiers `.sql` dans `migrations/` pour rester
        // robuste à l'ajout de nouvelles migrations.
        let applied: i64 =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM _sqlx_migrations")
                .fetch_one(&pool)
                .await
                .expect("comptage des migrations appliquées");

        let migrations_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("migrations");
        let expected: i64 = std::fs::read_dir(&migrations_dir)
            .expect("lecture dossier migrations")
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .and_then(|s| s.to_str())
                    .map(|s| s.eq_ignore_ascii_case("sql"))
                    .unwrap_or(false)
            })
            .count() as i64;
        assert_eq!(
            applied, expected,
            "le nombre de migrations appliquées doit valoir {expected} \
             (= nb de fichiers .sql dans migrations/), même après deux \
             runs ; observé = {applied}"
        );

        // Et le schéma reste cohérent : `media_item` est toujours là.
        let media_item_present: Option<String> = sqlx::query_scalar::<_, String>(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'media_item'",
        )
        .fetch_optional(&pool)
        .await
        .expect("relecture sqlite_master après second run");
        assert_eq!(media_item_present.as_deref(), Some("media_item"));

        pool.close().await;
    });
}
