-- Kyro migration 0002 — recherche FTS5 (v1.0).
--
-- Objectif : remplacer le full-scan en RAM (`Repo::search` charge
-- toutes les fiches puis filtre via `domain::search::matches`) par
-- une recherche indexée SQLite FTS5. Reste compatible avec la
-- sémantique existante :
--
--   * insensible à la casse (Requirement 13.2),
--   * insensible aux accents (Requirement 13.2),
--   * recherche par sous-chaîne (FTS5 prefix queries via `*`),
--   * requête vide retourne tout (Requirement 13.4) — géré côté Rust
--     en court-circuitant la requête FTS si `query.trim().is_empty()`.
--
-- ## Stratégie d'indexation
--
-- On utilise le tokenizer FTS5 `unicode61` configuré avec
-- `remove_diacritics 2` qui :
--   - décompose en NFKD,
--   - retire toutes les marques combinantes,
--   - tokenize sur les frontières Unicode standard.
--
-- C'est exactement ce que fait notre `domain::search::normalize` côté
-- Rust, donc l'index FTS reste cohérent avec la sémantique historique
-- sans qu'on ait à pré-normaliser les colonnes en SQL.
--
-- `content='media_item'` rend la table FTS un "external content table"
-- qui pointe sur `media_item` via `rowid = media_item.id`. La table
-- physique stocke uniquement l'index inversé, pas une copie des
-- colonnes (pas de surcoût stockage hors index).
--
-- ## Maintenance de l'index
--
-- Trois triggers SQLite synchronisent automatiquement la table FTS
-- avec `media_item.title` et `media_item.original_title` :
--
--   media_item_fts_ai (after insert)  — ajoute la nouvelle ligne
--   media_item_fts_ad (after delete)  — retire la ligne supprimée
--   media_item_fts_au (after update)  — re-indexe en cas de
--                                        changement title / original_title
--
-- Ces triggers utilisent la syntaxe « delete-then-insert » de FTS5
-- pour les mises à jour (un INSERT spécial avec rowid 'delete' au lieu
-- d'un UPDATE).

PRAGMA foreign_keys = ON;

-- ---------------------------------------------------------------------------
-- 1. Table virtuelle FTS5 indexant title + original_title.
-- ---------------------------------------------------------------------------

CREATE VIRTUAL TABLE media_item_fts USING fts5(
    title,
    original_title,
    content='media_item',
    content_rowid='id',
    tokenize='unicode61 remove_diacritics 2'
);

-- ---------------------------------------------------------------------------
-- 2. Triggers de synchronisation.
-- ---------------------------------------------------------------------------

CREATE TRIGGER media_item_fts_ai AFTER INSERT ON media_item BEGIN
    INSERT INTO media_item_fts(rowid, title, original_title)
    VALUES (new.id, new.title, COALESCE(new.original_title, ''));
END;

CREATE TRIGGER media_item_fts_ad AFTER DELETE ON media_item BEGIN
    INSERT INTO media_item_fts(media_item_fts, rowid, title, original_title)
    VALUES ('delete', old.id, old.title, COALESCE(old.original_title, ''));
END;

CREATE TRIGGER media_item_fts_au AFTER UPDATE OF title, original_title ON media_item BEGIN
    INSERT INTO media_item_fts(media_item_fts, rowid, title, original_title)
    VALUES ('delete', old.id, old.title, COALESCE(old.original_title, ''));
    INSERT INTO media_item_fts(rowid, title, original_title)
    VALUES (new.id, new.title, COALESCE(new.original_title, ''));
END;

-- ---------------------------------------------------------------------------
-- 3. Population initiale pour les bases existantes.
-- ---------------------------------------------------------------------------

INSERT INTO media_item_fts(rowid, title, original_title)
SELECT id, title, COALESCE(original_title, '') FROM media_item;
