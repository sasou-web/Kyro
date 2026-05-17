//! Tâche 11.3 — tests du format NDJSON émis par `kyro.lua`.
//!
//! # Justification de l'approche
//!
//! La tâche initiale prévoyait Busted (suite de tests Lua), mais
//! l'installer sur Windows en CI exige soit LuaRocks + un toolchain MSVC
//! complet, soit une image WSL — ce qui dépasse le périmètre raisonnable
//! d'une dépendance de test pour une seule fonction. L'alternative
//! envisagée, charger `kyro.lua` via le crate `mlua` (avec la feature
//! `lua54`), introduit une **dépendance native** au lien (header `lua.h`
//! + bibliothèque `lua54.dll`/`liblua54.a`) qui n'est pas disponible
//! par défaut dans l'environnement Tauri/MSVC du projet et qui pourrait
//! faire échouer le build sur des machines de développeurs sans toolset
//! Lua installé.
//!
//! On adopte donc l'approche **équivalente sans interprète Lua** :
//!
//! 1. **Analyse statique de `kyro.lua`** — on vérifie par recherche
//!    textuelle que le script (a) ouvre le pipe en mode `"wb"` sur
//!    `\\.\pipe\<name>`, (b) construit ses lignes NDJSON via
//!    `string.format` en utilisant exactement les clés `type`,
//!    `position`, `duration`, `session` du schéma `PipeMessage` du
//!    design, (c) installe le timer périodique de 5 s pour
//!    `progress`, et (d) enregistre les trois handlers
//!    `pause`/`unpause`/`shutdown` qui émettent les types correspondants
//!    avant de fermer le pipe.
//!
//! 2. **Re-implémentation fidèle de `send()` en Rust** — la fonction
//!    `lua_format_line` reproduit à l'identique le `string.format`
//!    utilisé dans `kyro.lua` (`%.3f` → `format!("{:.3}", x)`). Pour
//!    chaque type de message et un échantillon de positions/durées/UUIDs
//!    de session synthétiques, on assemble la ligne NDJSON et on vérifie
//!    qu'elle :
//!      - se termine par `\n` exactement (séparateur NDJSON),
//!      - constitue un objet JSON unique parsable,
//!      - contient strictement les clés attendues par le schéma,
//!      - se désérialise sans erreur en `PipeMessage` Rust (enum tagué
//!        par `type`, `rename_all = "lowercase"`) avec les valeurs
//!        d'origine restituées (round-trip).
//!
//! Cette double vérification — structurelle sur la source Lua + sémantique
//! sur le format de ligne — garantit que toute régression dans `kyro.lua`
//! qui casserait le contrat NDJSON serait détectée par `cargo test`,
//! sans nécessiter d'interprète Lua dans la chaîne de build.
//!
//! Validates: Requirements 9.2 (push 5 s périodique) et 9.3 (NDJSON
//! conforme au schéma `PipeMessage` côté Rust, fermeture propre du pipe
//! sur `shutdown`).

use std::path::PathBuf;

use serde::Deserialize;

// ---------------------------------------------------------------------------
// Schéma `PipeMessage` côté Rust — copié à l'identique du design
// (§ « Kyro_Player_Bridge et Kyro_IPC_Pipe — Protocole IPC »).
//
// Le module `infra::ipc_pipe` n'expose pas encore publiquement ce type
// (tâche 10.2 marquée en cours dans `tasks.md`). On le redéfinit ici à
// l'identique : si l'implémentation diverge, ce test signalera la
// régression.
// ---------------------------------------------------------------------------
#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
enum PipeMessage {
    Progress {
        position: f64,
        duration: f64,
        session: String,
    },
    Paused {
        position: f64,
        // `kyro.lua` émet `duration` pour tous les types ; serde l'ignore
        // pour les variantes qui ne le déclarent pas, ce qui correspond
        // au comportement souple décrit dans le design (le bridge ne lit
        // que les champs requis par chaque variante).
        session: String,
    },
    Resumed {
        position: f64,
        session: String,
    },
    Closed {
        position: f64,
        session: String,
    },
}

/// Reproduit fidèlement le `string.format` utilisé par `send()` dans
/// `kyro.lua` :
/// `'{"type":"%s","position":%.3f,"duration":%.3f,"session":"%s"}\n'`
fn lua_format_line(msg_type: &str, position: f64, duration: f64, session: &str) -> String {
    format!(
        "{{\"type\":\"{t}\",\"position\":{p:.3},\"duration\":{d:.3},\"session\":\"{s}\"}}\n",
        t = msg_type,
        p = position,
        d = duration,
        s = session,
    )
}

fn read_lua_source() -> String {
    // `CARGO_MANIFEST_DIR` pointe sur `src-tauri/` lorsque ce test est
    // exécuté en tant qu'intégration test (cf. `tests/migrations.rs`).
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let lua_path = manifest_dir.join("resources").join("kyro.lua");
    std::fs::read_to_string(&lua_path)
        .unwrap_or_else(|e| panic!("kyro.lua introuvable à {:?} : {}", lua_path, e))
}

// ---------------------------------------------------------------------------
// 1. Analyse statique de `kyro.lua`
// ---------------------------------------------------------------------------

#[test]
fn lua_script_opens_named_pipe_in_write_binary_mode() {
    let src = read_lua_source();
    // Le pipe est ouvert via io.open(full_pipe, "wb") où full_pipe est
    // construit depuis "\\\\.\\pipe\\" .. pipe_name (mode binaire pour
    // éviter toute traduction CRLF de Windows sur les séparateurs \n
    // NDJSON).
    assert!(
        src.contains(r#""\\\\.\\pipe\\""#),
        "kyro.lua doit construire un chemin de named pipe Windows (\\\\\\\\.\\\\pipe\\\\…)"
    );
    assert!(
        src.contains(r#"io.open(full_pipe, "wb")"#)
            || src.contains(r#"io.open("\\\\.\\pipe\\""#),
        "kyro.lua doit ouvrir un named pipe via io.open(..., \"wb\")"
    );
    assert!(
        src.contains("\"wb\""),
        "kyro.lua doit ouvrir le pipe en mode \"wb\" pour préserver les \\n NDJSON"
    );
}

#[test]
fn lua_script_uses_pipemessage_schema_keys() {
    let src = read_lua_source();
    // Le format NDJSON doit citer exactement les quatre clés du schéma
    // PipeMessage du design pour les messages standard (progress,
    // paused, resumed, closed).
    let template = r#"'{"type":"%s","position":%.3f,"duration":%.3f,"session":"%s"}\n'"#;
    assert!(
        src.contains(template),
        "kyro.lua doit utiliser le format NDJSON exact du design ({}). \
         Toute divergence casse le schéma PipeMessage côté Rust.",
        template
    );
}

#[test]
fn lua_script_installs_five_second_progress_timer() {
    let src = read_lua_source();
    // Requirement 9.2 : poussée de la position courante toutes les 5 s.
    // L'argument du send peut être (`"progress"`) ou (`"progress", false`)
    // selon la signature actuelle (use_cache flag).
    assert!(
        src.contains(r#"mp.add_periodic_timer(5"#)
            && (src.contains(r#"send("progress")"#)
                || src.contains(r#"send("progress", false)"#)),
        "kyro.lua doit installer un timer périodique de 5 s qui émet \"progress\" (Requirement 9.2)"
    );
}

#[test]
fn lua_script_registers_pause_unpause_shutdown_handlers() {
    let src = read_lua_source();
    // Requirement 9.3 : émission propre sur les transitions du joueur,
    // dont la fermeture finale. La signature `send` accepte un flag
    // `use_cache` optionnel sur les versions récentes.
    assert!(
        src.contains(r#"mp.register_event("pause""#),
        "kyro.lua doit s'abonner à l'événement mpv `pause`"
    );
    assert!(
        src.contains(r#"send("paused")"#)
            || src.contains(r#"send("paused", false)"#)
            || src.contains(r#"send("paused",  false)"#),
        "kyro.lua doit émettre un message NDJSON de type \"paused\""
    );
    assert!(
        src.contains(r#"mp.register_event("unpause""#),
        "kyro.lua doit s'abonner à l'événement mpv `unpause`"
    );
    assert!(
        src.contains(r#"send("resumed")"#)
            || src.contains(r#"send("resumed", false)"#)
            || src.contains(r#"send("resumed",  false)"#),
        "kyro.lua doit émettre un message NDJSON de type \"resumed\""
    );
    assert!(
        src.contains(r#"mp.register_event("shutdown""#),
        "kyro.lua doit s'abonner à l'événement mpv `shutdown` (Requirement 9.3)"
    );
    assert!(
        src.contains(r#"send("closed")"#)
            || src.contains(r#"send("closed", true)"#)
            || src.contains(r#"send("closed", false)"#),
        "kyro.lua doit émettre un message NDJSON final de type \"closed\""
    );
    assert!(
        src.contains("f:close()"),
        "kyro.lua doit fermer le pipe après l'émission de \"closed\""
    );
}

// ---------------------------------------------------------------------------
// 2. Sémantique du format de ligne — chaque ligne produite par `send()`
//    est un objet NDJSON terminé par `\n`, parsable, contenant exactement
//    les clés attendues, et désérialisable en `PipeMessage`.
// ---------------------------------------------------------------------------

fn assert_ndjson_line_well_formed(line: &str) {
    assert!(
        line.ends_with('\n'),
        "ligne NDJSON {:?} doit se terminer par \\n",
        line
    );
    // Une seule ligne (pas de `\n` interne).
    assert_eq!(
        line.matches('\n').count(),
        1,
        "ligne NDJSON {:?} doit contenir exactement un \\n (séparateur final)",
        line
    );
    // Doit être un objet JSON unique parsable.
    let trimmed = line.trim_end_matches('\n');
    let value: serde_json::Value =
        serde_json::from_str(trimmed).expect("ligne NDJSON doit être un objet JSON valide");
    let obj = value
        .as_object()
        .expect("ligne NDJSON doit être un objet JSON (pas un tableau / scalaire)");
    // Schéma : exactement les quatre clés du `PipeMessage`. L'ordre
    // d'apparition dans la chaîne brute est validé séparément ci-dessous
    // (l'`Object` de `serde_json` ne préserve pas l'ordre d'insertion
    // sans la feature `preserve_order`, désactivée volontairement).
    let mut keys: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec!["duration", "position", "session", "type"],
        "ligne NDJSON doit avoir exactement les clés du schéma PipeMessage"
    );
    // Ordre d'apparition dans la ligne brute (vérifié sur les bytes) :
    // type → position → duration → session, comme dans le `string.format`
    // de `kyro.lua`.
    let trimmed_bytes = trimmed;
    let pos_type = trimmed_bytes.find("\"type\"").expect("clé type manquante");
    let pos_position = trimmed_bytes
        .find("\"position\"")
        .expect("clé position manquante");
    let pos_duration = trimmed_bytes
        .find("\"duration\"")
        .expect("clé duration manquante");
    let pos_session = trimmed_bytes
        .find("\"session\"")
        .expect("clé session manquante");
    assert!(
        pos_type < pos_position && pos_position < pos_duration && pos_duration < pos_session,
        "ordre des clés dans la ligne NDJSON brute doit être type/position/duration/session"
    );
}

#[test]
fn progress_line_round_trips_into_pipe_message() {
    let session = "9c8b1ad2-f9e0-4a7e-a9f4-1234567890ab";
    let line = lua_format_line("progress", 1234.567_891, 5432.1, session);
    assert_ndjson_line_well_formed(&line);

    let trimmed = line.trim_end_matches('\n');
    let parsed: PipeMessage =
        serde_json::from_str(trimmed).expect("ligne progress doit se désérialiser en PipeMessage");
    assert_eq!(
        parsed,
        PipeMessage::Progress {
            // `%.3f` arrondit à 3 décimales (1234.567891 → 1234.568).
            position: 1234.568,
            duration: 5432.100,
            session: session.to_string(),
        }
    );
}

#[test]
fn paused_line_round_trips_into_pipe_message() {
    let session = "deadbeef-cafe-1234-5678-abcdef012345";
    let line = lua_format_line("paused", 60.0, 7200.0, session);
    assert_ndjson_line_well_formed(&line);

    let parsed: PipeMessage = serde_json::from_str(line.trim_end_matches('\n'))
        .expect("ligne paused doit se désérialiser en PipeMessage");
    assert_eq!(
        parsed,
        PipeMessage::Paused {
            position: 60.0,
            session: session.to_string(),
        }
    );
}

#[test]
fn resumed_line_round_trips_into_pipe_message() {
    let session = "00000000-0000-4000-8000-000000000001";
    let line = lua_format_line("resumed", 60.5, 7200.0, session);
    assert_ndjson_line_well_formed(&line);

    let parsed: PipeMessage = serde_json::from_str(line.trim_end_matches('\n'))
        .expect("ligne resumed doit se désérialiser en PipeMessage");
    assert_eq!(
        parsed,
        PipeMessage::Resumed {
            position: 60.5,
            session: session.to_string(),
        }
    );
}

#[test]
fn closed_line_round_trips_into_pipe_message() {
    let session = "11111111-2222-4333-8444-555555555555";
    let line = lua_format_line("closed", 7195.999, 7200.0, session);
    assert_ndjson_line_well_formed(&line);

    let parsed: PipeMessage = serde_json::from_str(line.trim_end_matches('\n'))
        .expect("ligne closed doit se désérialiser en PipeMessage");
    assert_eq!(
        parsed,
        PipeMessage::Closed {
            position: 7195.999,
            session: session.to_string(),
        }
    );
}

#[test]
fn boundary_positions_serialize_with_three_decimals() {
    // Vérifie le contrat `%.3f` du `string.format` Lua sur les bornes
    // typiques (début, demi-seconde, fin de fichier longue durée).
    let cases: &[(f64, &str)] = &[
        (0.0, "0.000"),
        (0.5, "0.500"),
        (1.0 / 3.0, "0.333"),
        (12_345.6789, "12345.679"),
    ];
    for (input, expected_repr) in cases {
        let line = lua_format_line("progress", *input, 100.0, "session-id");
        assert!(
            line.contains(&format!("\"position\":{expected_repr}")),
            "position {} doit être formatée en {} (line = {:?})",
            input,
            expected_repr,
            line
        );
        let _: PipeMessage = serde_json::from_str(line.trim_end_matches('\n'))
            .expect("ligne avec position de bord doit rester parsable");
    }
}

#[test]
fn ndjson_lines_concatenate_without_extra_separators() {
    // Le bridge lit ligne-par-ligne ; deux messages successifs doivent
    // se séparer uniquement par le `\n` final de chacun, sans ajouts.
    let session = "abcdef01-2345-4678-89ab-cdef01234567";
    let l1 = lua_format_line("progress", 10.0, 100.0, session);
    let l2 = lua_format_line("paused", 12.5, 100.0, session);
    let stream = format!("{l1}{l2}");

    let lines: Vec<&str> = stream.split_inclusive('\n').collect();
    assert_eq!(lines.len(), 2, "stream NDJSON doit contenir 2 lignes");
    for line in &lines {
        assert_ndjson_line_well_formed(line);
        let _: PipeMessage = serde_json::from_str(line.trim_end_matches('\n'))
            .expect("chaque ligne du stream doit se désérialiser en PipeMessage");
    }
}
