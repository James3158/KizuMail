CREATE TABLE IF NOT EXISTS ai_app_settings (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
  account_id TEXT REFERENCES accounts(id) ON DELETE SET NULL,
  provider TEXT NOT NULL DEFAULT 'gemini' CHECK (provider IN ('gemini', 'ollama')),
  generation_model TEXT NOT NULL DEFAULT 'gemini-3.1-flash-lite',
  embedding_model TEXT NOT NULL DEFAULT 'gemini-embedding-001',
  ollama_base_url TEXT NOT NULL DEFAULT 'http://127.0.0.1:11434',
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

INSERT OR IGNORE INTO ai_app_settings (id) VALUES (1);

CREATE TABLE IF NOT EXISTS ai_account_settings (
  account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
  local_index_enabled INTEGER NOT NULL DEFAULT 0 CHECK (local_index_enabled IN (0, 1)),
  semantic_enabled INTEGER NOT NULL DEFAULT 0 CHECK (semantic_enabled IN (0, 1)),
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE VIRTUAL TABLE IF NOT EXISTS ai_message_fts USING fts5(
  message_id UNINDEXED,
  account_id UNINDEXED,
  subject,
  sender,
  preview,
  body_text,
  tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TABLE IF NOT EXISTS ai_message_state (
  message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  content_hash TEXT NOT NULL,
  chunk_count INTEGER NOT NULL DEFAULT 0 CHECK (chunk_count >= 0),
  status TEXT NOT NULL DEFAULT 'ready' CHECK (status IN ('pending', 'ready', 'stale', 'failed')),
  last_error TEXT,
  indexed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS ai_message_state_account_status_idx
  ON ai_message_state(account_id, status, updated_at);

CREATE TABLE IF NOT EXISTS ai_embeddings (
  message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  model TEXT NOT NULL,
  chunk_index INTEGER NOT NULL DEFAULT 0 CHECK (chunk_index >= 0),
  dimensions INTEGER NOT NULL CHECK (dimensions > 0),
  vector BLOB NOT NULL,
  content_hash TEXT NOT NULL,
  chunk_hash TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (message_id, model, chunk_index)
);

CREATE INDEX IF NOT EXISTS ai_embeddings_model_account_idx
  ON ai_embeddings(model, account_id, message_id);

CREATE TABLE IF NOT EXISTS ai_jobs (
  id TEXT PRIMARY KEY,
  account_id TEXT REFERENCES accounts(id) ON DELETE CASCADE,
  kind TEXT NOT NULL DEFAULT 'index' CHECK (kind IN ('index', 'embed', 'rebuild')),
  status TEXT NOT NULL DEFAULT 'queued' CHECK (status IN ('queued', 'running', 'completed', 'cancelled', 'failed')),
  batch_size INTEGER NOT NULL DEFAULT 50 CHECK (batch_size BETWEEN 1 AND 500),
  processed_count INTEGER NOT NULL DEFAULT 0 CHECK (processed_count >= 0),
  total_count INTEGER NOT NULL DEFAULT 0 CHECK (total_count >= 0),
  cursor_json TEXT NOT NULL DEFAULT '{"next_account_index":0,"accounts":[]}',
  cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK (cancel_requested IN (0, 1)),
  last_error TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  started_at TEXT,
  finished_at TEXT,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS ai_jobs_status_created_idx ON ai_jobs(status, created_at);
CREATE INDEX IF NOT EXISTS ai_jobs_account_status_idx ON ai_jobs(account_id, status);

CREATE TABLE IF NOT EXISTS ai_cache (
  cache_key TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  message_id TEXT REFERENCES messages(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,
  model TEXT,
  content_hash TEXT NOT NULL,
  value_json TEXT,
  value_blob BLOB,
  expires_at TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CHECK (value_json IS NOT NULL OR value_blob IS NOT NULL)
);

CREATE INDEX IF NOT EXISTS ai_cache_account_kind_idx ON ai_cache(account_id, kind, updated_at);
CREATE INDEX IF NOT EXISTS ai_cache_message_idx ON ai_cache(message_id, content_hash);
CREATE INDEX IF NOT EXISTS ai_cache_expiry_idx ON ai_cache(expires_at);

CREATE TRIGGER IF NOT EXISTS ai_messages_delete_fts
AFTER DELETE ON messages
BEGIN
  DELETE FROM ai_message_fts WHERE message_id = OLD.id;
END;

CREATE TRIGGER IF NOT EXISTS ai_messages_mark_stale
AFTER UPDATE OF subject, from_addr, body_text ON messages
BEGIN
  UPDATE ai_message_state
     SET status = 'stale', updated_at = CURRENT_TIMESTAMP
   WHERE message_id = NEW.id;
  DELETE FROM ai_cache WHERE message_id = NEW.id;
END;
