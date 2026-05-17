-- Kyro initial schema (migration 0001_init.sql)
-- Reference: .kiro/specs/kyro/design.md > Data Models > DDL SQLite
-- Validates: Requirement 17.1 (Kyro_Library stockée localement) — schéma initial

PRAGMA foreign_keys = ON;

-- ---------------------------------------------------------------------------
-- library_folder : dossiers configurés par l'utilisateur (Requirement 2)
-- ---------------------------------------------------------------------------
CREATE TABLE library_folder (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  path        TEXT NOT NULL UNIQUE,
  category    TEXT NOT NULL CHECK (category IN ('Films','Series','Animes')),
  created_at  DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- ---------------------------------------------------------------------------
-- media_item : Film / Series / Anime (Requirements 6, 7, 8, 11)
-- ---------------------------------------------------------------------------
CREATE TABLE media_item (
  id                       INTEGER PRIMARY KEY AUTOINCREMENT,
  folder_id                INTEGER REFERENCES library_folder(id) ON DELETE SET NULL,
  type                     TEXT NOT NULL CHECK (type IN ('Film','Series','Anime')),
  file_path                TEXT,                              -- NULL si Series/Anime racine
  size_bytes               INTEGER,
  title                    TEXT NOT NULL,
  original_title           TEXT,
  year                     INTEGER,
  quality                  TEXT,
  synopsis                 TEXT,
  studio                   TEXT,
  user_rating              INTEGER CHECK (user_rating BETWEEN 0 AND 10),
  poster_path              TEXT,
  duration_seconds         INTEGER,
  video_codec              TEXT,
  width                    INTEGER,
  height                   INTEGER,
  watch_progress_seconds   INTEGER NOT NULL DEFAULT 0 CHECK (watch_progress_seconds >= 0),
  status                   TEXT NOT NULL DEFAULT 'non_vu'
                           CHECK (status IN ('non_vu','en_cours','vu')),
  favorite                 INTEGER NOT NULL DEFAULT 0 CHECK (favorite IN (0,1)),
  to_watch                 INTEGER NOT NULL DEFAULT 0 CHECK (to_watch IN (0,1)),
  available                INTEGER NOT NULL DEFAULT 1 CHECK (available IN (0,1)),
  numbering_unknown        INTEGER NOT NULL DEFAULT 0 CHECK (numbering_unknown IN (0,1)),
  last_scanned_at          DATETIME
);
CREATE INDEX idx_media_item_type    ON media_item(type);
CREATE INDEX idx_media_item_status  ON media_item(status);
CREATE INDEX idx_media_item_year    ON media_item(year);
CREATE INDEX idx_media_item_title   ON media_item(title COLLATE NOCASE);

-- ---------------------------------------------------------------------------
-- episode_item : épisodes d'une Series/Anime (Requirements 4, 6.5, 8, 12)
-- ---------------------------------------------------------------------------
CREATE TABLE episode_item (
  id                       INTEGER PRIMARY KEY AUTOINCREMENT,
  series_id                INTEGER NOT NULL REFERENCES media_item(id) ON DELETE CASCADE,
  season_number            INTEGER NOT NULL,
  episode_number           INTEGER NOT NULL,
  file_path                TEXT NOT NULL UNIQUE,
  size_bytes               INTEGER,
  title                    TEXT,
  synopsis                 TEXT,
  duration_seconds         INTEGER,
  video_codec              TEXT,
  width                    INTEGER,
  height                   INTEGER,
  watch_progress_seconds   INTEGER NOT NULL DEFAULT 0 CHECK (watch_progress_seconds >= 0),
  status                   TEXT NOT NULL DEFAULT 'non_vu'
                           CHECK (status IN ('non_vu','en_cours','vu')),
  available                INTEGER NOT NULL DEFAULT 1 CHECK (available IN (0,1)),
  numbering_unknown        INTEGER NOT NULL DEFAULT 0 CHECK (numbering_unknown IN (0,1)),
  poster_path              TEXT,
  last_scanned_at          DATETIME
);
CREATE INDEX idx_episode_series ON episode_item(series_id, season_number, episode_number);

-- ---------------------------------------------------------------------------
-- genre + media_genre : tags genres (Requirement 6, 14)
-- ---------------------------------------------------------------------------
CREATE TABLE genre (
  id   INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL UNIQUE
);

CREATE TABLE media_genre (
  media_item_id INTEGER NOT NULL REFERENCES media_item(id) ON DELETE CASCADE,
  genre_id      INTEGER NOT NULL REFERENCES genre(id) ON DELETE CASCADE,
  PRIMARY KEY (media_item_id, genre_id)
);

-- ---------------------------------------------------------------------------
-- audio_track / subtitle_track : pistes techniques (Requirement 5)
-- Contrainte XOR : la piste appartient soit à un media_item, soit à un episode_item.
-- ---------------------------------------------------------------------------
CREATE TABLE audio_track (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  media_item_id   INTEGER REFERENCES media_item(id) ON DELETE CASCADE,
  episode_item_id INTEGER REFERENCES episode_item(id) ON DELETE CASCADE,
  codec           TEXT,
  language        TEXT,
  CHECK ((media_item_id IS NULL) <> (episode_item_id IS NULL))
);

CREATE TABLE subtitle_track (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  media_item_id   INTEGER REFERENCES media_item(id) ON DELETE CASCADE,
  episode_item_id INTEGER REFERENCES episode_item(id) ON DELETE CASCADE,
  codec           TEXT,
  language        TEXT,
  CHECK ((media_item_id IS NULL) <> (episode_item_id IS NULL))
);

-- ---------------------------------------------------------------------------
-- setting_value : key/value des préférences applicatives (Requirements 2, 5.4, 9.7, 10, 16)
-- Clés attendues : external_player_path, ffprobe_path, theme,
--                  scan_at_startup, last_category.
-- ---------------------------------------------------------------------------
CREATE TABLE setting_value (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
