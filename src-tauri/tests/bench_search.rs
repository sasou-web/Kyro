//! Tâche 17.3 — bench de la recherche sur 5000 Media_Item.
//!
//! Peuple une base SQLite temporaire avec 5000 fiches `media_item`
//! synthétiques (titres variés, accents inclus pour exercer la
//! normalisation Unicode), puis exécute plusieurs requêtes via
//! `Repo::search` et mesure le temps de chaque appel avec
//! `std::time::Instant`.
//!
//! Cible (requirement 13.3) : ≤ 200 ms par recherche, à frappe
//! équivalente à une saisie utilisateur dans la TopBar.
//!
//! Ces tests sont marqués `#[ignore]` afin de ne pas alourdir le run
//! `cargo test` standard. Ils s'exécutent à la demande :
//!
//! ```bash
//! cargo test --test bench_search --release -- --ignored --nocapture
//! ```
//!
//! Le mode `--release` est nécessaire pour obtenir des temps
//! représentatifs (les optimisations rustc + sqlx font une différence
//! d'un ordre de grandeur). Le couplage avec le bench frontend
//! (`frontend/src/lib/stores/filtered.bench.test.ts`) couvre la même
//! cible côté store dérivé.
//!
//! Validates: Requirement 13.3 (recherche ≤ 200 ms par frappe pour
//! 5000 items).

use std::path::Path;
use std::time::Instant;

use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
};
use tempfile::TempDir;

use kyro::infra::repo::Repo;

/// Seuil dur d'acceptabilité (requirement 13.3).
const SEARCH_BUDGET_MS: u128 = 200;

/// Nombre de fiches synthétiques injectées avant la mesure.
const NUM_ITEMS: usize = 5000;

/// Construit un runtime tokio `current_thread` local. La feature
/// `macros` de `tokio` n'est pas utilisée pour les tests d'intégration
/// du projet (cf. `tests/migrations.rs`).
fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("création d'un runtime tokio current_thread")
        .block_on(fut)
}

/// Reproduit fidèlement les pragmas de `init_sqlite_pool` (cf.
/// `src/main.rs`) : WAL + synchronous=NORMAL + foreign_keys=ON.
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

/// Génère des titres synthétiques variés. La diversité est essentielle
/// pour que `domain::search::matches` exerce réellement la
/// normalisation Unicode et n'optimise pas trivialement par
/// court-circuit. On combine :
///   * un préfixe accentué tournant (`Évangelion`, `Pokémon`, `Café`,
///     `Naruto`, `Spirited`, `Inception`, `Akira`) — couvre les NFD ;
///   * un suffixe numérique unique pour garantir 5000 titres
///     distincts ;
///   * un titre original optionnel (1 sur 2) pour exercer la branche
///     `original_title` de `matches`.
fn synth_title(i: usize) -> (String, Option<String>) {
    const PREFIXES: &[&str] = &[
        "Évangelion",
        "Pokémon",
        "Café Society",
        "Naruto",
        "Spirited Away",
        "Inception",
        "Akira",
        "Cowboy Bebop",
        "Le Voyage de Chihiro",
        "Mononoké",
    ];
    let prefix = PREFIXES[i % PREFIXES.len()];
    let title = format!("{} {}", prefix, i);
    let original = if i % 2 == 0 {
        Some(format!("Original {}", i))
    } else {
        None
    };
    (title, original)
}

/// Insère `n` fiches synthétiques dans la table `media_item`.
///
/// On injecte directement via `INSERT` SQL (et non via `Repo::upsert_*`)
/// pour rester focalisé sur la mesure du **chemin de lecture** : on
/// veut benchmarker `search`, pas `upsert`. Un `BEGIN IMMEDIATE` /
/// `COMMIT` enveloppe l'ensemble pour rester sous la seconde de
/// peuplement même en `--debug`.
async fn populate(pool: &SqlitePool, n: usize) {
    let mut tx = pool.begin().await.expect("BEGIN tx peuplement");
    for i in 0..n {
        let (title, original) = synth_title(i);
        // Attention : `media_item.file_path` n'a pas de UNIQUE en DDL,
        // mais on en fournit un distinct par fiche pour rester
        // représentatif des données réelles produites par le scanner.
        let file_path = format!("C:/Users/Test/Films/synthetic_{:05}.mkv", i);
        let media_type = match i % 3 {
            0 => "Film",
            1 => "Series",
            _ => "Anime",
        };
        let year: i64 = 1990 + (i as i64 % 30);
        sqlx::query(
            "INSERT INTO media_item ( \
                 type, file_path, size_bytes, title, original_title, year, \
                 watch_progress_seconds, status, favorite, to_watch, available, \
                 numbering_unknown, last_scanned_at \
             ) VALUES ( \
                 ?, ?, ?, ?, ?, ?, \
                 0, 'non_vu', 0, 0, 1, \
                 0, CURRENT_TIMESTAMP \
             )",
        )
        .bind(media_type)
        .bind(&file_path)
        .bind((i as i64) * 1024)
        .bind(&title)
        .bind(original.as_deref())
        .bind(year)
        .execute(&mut *tx)
        .await
        .expect("INSERT media_item synthétique");
    }
    tx.commit().await.expect("COMMIT tx peuplement");
}

#[test]
#[ignore = "bench long, lance via `cargo test --test bench_search --release -- --ignored`"]
fn search_5000_items_under_200ms_per_query() {
    block_on(async {
        // 1. Pool temporaire + migrations (schéma identique à la prod).
        let tmp = TempDir::new().expect("création TempDir");
        let db_path = tmp.path().join("library.db");
        let pool = open_pool(&db_path).await;
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("application des migrations");

        // 2. Peuplement à 5000 fiches synthétiques.
        populate(&pool, NUM_ITEMS).await;

        let count: i64 =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM media_item")
                .fetch_one(&pool)
                .await
                .expect("comptage post-populate");
        assert_eq!(
            count, NUM_ITEMS as i64,
            "le peuplement doit produire exactement {NUM_ITEMS} fiches"
        );

        // 3. Préparer le repo et un panel de requêtes représentatives
        //    d'une saisie utilisateur (touche par touche). Chaque
        //    requête est mesurée individuellement et doit rester sous
        //    `SEARCH_BUDGET_MS`.
        let repo = Repo::new(pool.clone());
        let queries = [
            "",            // requête vide — passage rapide
            "e",           // 1 caractère, sélectivité minimale
            "ev",          //
            "eva",         //
            "evan",        //
            "evangelion",  // sous-chaîne fréquente
            "POKEMON",     // casse mixte → exerce la normalisation
            "pokémon",     // accent → exerce NFD
            "café",        // diacritique
            "introuvable", // 0 résultat
        ];

        for q in queries {
            let start = Instant::now();
            let results = repo
                .search(q)
                .await
                .unwrap_or_else(|err| panic!("Repo::search(\"{q}\") a échoué : {err:?}"));
            let elapsed_ms = start.elapsed().as_millis();

            eprintln!(
                "search({q:?}) → {} résultats en {} ms",
                results.len(),
                elapsed_ms
            );

            assert!(
                elapsed_ms <= SEARCH_BUDGET_MS,
                "Repo::search(\"{q}\") = {elapsed_ms} ms > budget {SEARCH_BUDGET_MS} ms \
                 (requirement 13.3, {NUM_ITEMS} items)"
            );
        }

        pool.close().await;
    });
}
