CREATE TABLE accounts (
  id TEXT PRIMARY KEY,
  provider TEXT NOT NULL,
  email TEXT NOT NULL,
  display_name TEXT NOT NULL,
  auth_type TEXT NOT NULL,
  secret_ref TEXT,
  imap_host TEXT,
  imap_port INTEGER,
  smtp_host TEXT,
  smtp_port INTEGER,
  sync_enabled INTEGER NOT NULL DEFAULT 1,
  last_sync_at TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX accounts_email_provider_idx ON accounts(email, provider);

CREATE TABLE folders (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  path TEXT NOT NULL,
  display_name TEXT NOT NULL,
  role TEXT,
  uid_validity TEXT,
  uid_next TEXT,
  last_synced_at TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX folders_account_path_idx ON folders(account_id, path);

CREATE TABLE messages (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  folder_id TEXT NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
  uid TEXT NOT NULL,
  message_id TEXT,
  thread_key TEXT,
  subject TEXT NOT NULL DEFAULT '',
  from_addr TEXT NOT NULL DEFAULT '',
  to_addrs_json TEXT NOT NULL DEFAULT '[]',
  date TEXT,
  flags_json TEXT NOT NULL DEFAULT '[]',
  body_text TEXT NOT NULL DEFAULT '',
  body_html_sanitized TEXT,
  raw_headers_json TEXT NOT NULL DEFAULT '{}',
  has_attachments INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX messages_account_folder_uid_idx ON messages(account_id, folder_id, uid);
CREATE INDEX messages_account_date_idx ON messages(account_id, date);
CREATE INDEX messages_from_addr_idx ON messages(from_addr);

CREATE VIRTUAL TABLE message_search USING fts5(
  subject,
  from_addr,
  to_addrs,
  body_text,
  content='messages',
  content_rowid='rowid'
);

CREATE TABLE senders (
  id TEXT PRIMARY KEY,
  normalized_email TEXT NOT NULL UNIQUE,
  domain TEXT NOT NULL,
  display_name TEXT,
  first_seen_at TEXT NOT NULL,
  last_seen_at TEXT NOT NULL,
  message_count INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX senders_domain_idx ON senders(domain);

CREATE TABLE blocked_senders (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  sender_email TEXT NOT NULL,
  domain TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX blocked_senders_account_email_idx ON blocked_senders(account_id, sender_email);

CREATE TABLE newsletter_subscriptions (
  id TEXT PRIMARY KEY,
  sender_id TEXT NOT NULL REFERENCES senders(id) ON DELETE CASCADE,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  evidence_json TEXT NOT NULL DEFAULT '[]',
  unsubscribe_type TEXT NOT NULL DEFAULT 'unknown',
  unsubscribe_target TEXT,
  confidence_score INTEGER NOT NULL DEFAULT 0,
  risk_score INTEGER NOT NULL DEFAULT 100,
  status TEXT NOT NULL DEFAULT 'candidate',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX newsletter_account_sender_idx ON newsletter_subscriptions(account_id, sender_id);

CREATE TABLE unsubscribe_actions (
  id TEXT PRIMARY KEY,
  subscription_id TEXT NOT NULL REFERENCES newsletter_subscriptions(id) ON DELETE CASCADE,
  method TEXT NOT NULL,
  target TEXT NOT NULL,
  dry_run_result_json TEXT,
  executed_at TEXT,
  result_status TEXT NOT NULL DEFAULT 'draft',
  result_summary TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE rules (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  conditions_json TEXT NOT NULL DEFAULT '{}',
  actions_json TEXT NOT NULL DEFAULT '{}',
  enabled INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE sync_state (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  folder_id TEXT NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
  cursor_json TEXT NOT NULL DEFAULT '{}',
  last_error TEXT,
  backoff_until TEXT,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX sync_state_account_folder_idx ON sync_state(account_id, folder_id);

CREATE TABLE attachments (
  id TEXT PRIMARY KEY,
  message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  filename TEXT,
  mime_type TEXT,
  size INTEGER NOT NULL DEFAULT 0,
  content_id TEXT,
  storage_ref TEXT,
  scan_status TEXT NOT NULL DEFAULT 'pending',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE audit_log (
  id TEXT PRIMARY KEY,
  actor TEXT NOT NULL DEFAULT 'local_user',
  action TEXT NOT NULL,
  entity_type TEXT NOT NULL,
  entity_id TEXT,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX audit_log_created_at_idx ON audit_log(created_at);
CREATE INDEX audit_log_action_idx ON audit_log(action);
