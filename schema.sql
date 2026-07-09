CREATE TABLE IF NOT EXISTS parallels (
  id           INTEGER PRIMARY KEY,
  zh_text      TEXT NOT NULL,
  zh_norm      TEXT NOT NULL,
  foreign_lang TEXT NOT NULL,
  foreign_text TEXT NOT NULL,
  confidence   REAL,
  cbeta_id     TEXT,
  title_zh     TEXT,
  juan_num     INTEGER
);

-- Write-once artifact: only an AFTER INSERT trigger syncs the FTS index.
-- If a future path UPDATEs/DELETEs parallels, add matching triggers or the FTS index desyncs.
CREATE VIRTUAL TABLE IF NOT EXISTS parallels_fts USING fts5(
  zh_norm,
  content='parallels',
  content_rowid='id',
  tokenize='trigram'
);

CREATE TRIGGER IF NOT EXISTS parallels_ai AFTER INSERT ON parallels BEGIN
  INSERT INTO parallels_fts(rowid, zh_norm) VALUES (new.id, new.zh_norm);
END;

CREATE TABLE IF NOT EXISTS meta (
  key   TEXT PRIMARY KEY,
  value TEXT
);

CREATE TABLE IF NOT EXISTS norm_map (
  from_char TEXT PRIMARY KEY,
  to_char   TEXT NOT NULL
);
