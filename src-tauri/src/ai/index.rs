use super::models::{DEFAULT_GEMINI_GENERATION_MODEL, LEGACY_GEMINI_GENERATION_MODEL};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};

pub const AI_INDEX_MIGRATION: &str = include_str!("../../migrations/0100_ai_local_index.sql");
pub const DEFAULT_BATCH_SIZE: usize = 50;
pub const MAX_BATCH_SIZE: usize = 500;
pub const DEFAULT_CHUNK_CHARS: usize = 1_600;
pub const DEFAULT_CHUNK_OVERLAP: usize = 200;

pub type AiIndexResult<T> = Result<T, String>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountBatchCursor {
    pub account_id: String,
    pub last_sort_key: Option<String>,
    pub last_message_id: Option<String>,
    pub exhausted: bool,
}

impl AccountBatchCursor {
    fn new(account_id: String) -> Self {
        Self {
            account_id,
            last_sort_key: None,
            last_message_id: None,
            exhausted: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BatchCursor {
    pub next_account_index: usize,
    pub accounts: Vec<AccountBatchCursor>,
}

impl BatchCursor {
    pub fn new(account_ids: Vec<String>) -> Self {
        Self {
            next_account_index: 0,
            accounts: account_ids
                .into_iter()
                .map(AccountBatchCursor::new)
                .collect(),
        }
    }

    pub fn reconcile(&mut self, account_ids: &[String]) {
        let mut existing = self
            .accounts
            .drain(..)
            .map(|cursor| (cursor.account_id.clone(), cursor))
            .collect::<HashMap<_, _>>();
        self.accounts = account_ids
            .iter()
            .map(|account_id| {
                existing
                    .remove(account_id)
                    .unwrap_or_else(|| AccountBatchCursor::new(account_id.clone()))
            })
            .collect();
        self.next_account_index = if self.accounts.is_empty() {
            0
        } else {
            self.next_account_index % self.accounts.len()
        };
    }

    pub fn advance(&mut self, account_id: &str, sort_key: String, message_id: String) {
        if let Some(cursor) = self
            .accounts
            .iter_mut()
            .find(|item| item.account_id == account_id)
        {
            cursor.last_sort_key = Some(sort_key);
            cursor.last_message_id = Some(message_id);
            cursor.exhausted = false;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexProgress {
    pub job_id: String,
    pub status: String,
    pub processed_count: usize,
    pub total_count: usize,
    pub batch_processed: usize,
    pub cancel_requested: bool,
    pub cursor: BatchCursor,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone)]
struct IndexJob {
    id: String,
    account_id: Option<String>,
    status: String,
    batch_size: usize,
    processed_count: usize,
    total_count: usize,
    cursor: BatchCursor,
    cancel_requested: bool,
    last_error: Option<String>,
}

#[derive(Debug, Clone)]
struct IndexMessage {
    id: String,
    account_id: String,
    subject: String,
    sender: String,
    preview: String,
    body_text: String,
    sort_key: String,
}

pub fn apply_ai_index_migration(connection: &Connection) -> AiIndexResult<()> {
    connection
        .execute_batch(AI_INDEX_MIGRATION)
        .map_err(|error| format!("AI index migration failed: {error}"))?;

    // Upgrade only the old built-in default; keep an explicit user choice intact.
    connection
        .execute(
            "UPDATE ai_app_settings
             SET generation_model = ?1, updated_at = CURRENT_TIMESTAMP
             WHERE id = 1 AND generation_model = ?2",
            params![
                DEFAULT_GEMINI_GENERATION_MODEL,
                LEGACY_GEMINI_GENERATION_MODEL
            ],
        )
        .map_err(|error| format!("AI model default migration failed: {error}"))?;
    Ok(())
}

pub fn set_account_access(
    connection: &mut Connection,
    account_id: &str,
    local_index_enabled: bool,
    semantic_enabled: bool,
) -> AiIndexResult<()> {
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    let changed = transaction
        .execute(
            "INSERT INTO ai_account_settings (
               account_id, local_index_enabled, semantic_enabled, updated_at
             )
             SELECT id, ?2, ?3, CURRENT_TIMESTAMP FROM accounts WHERE id = ?1
             ON CONFLICT(account_id) DO UPDATE SET
               local_index_enabled = excluded.local_index_enabled,
               semantic_enabled = excluded.semantic_enabled,
               updated_at = CURRENT_TIMESTAMP",
            params![
                account_id,
                if local_index_enabled { 1 } else { 0 },
                if local_index_enabled && semantic_enabled {
                    1
                } else {
                    0
                }
            ],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err(format!("Account {account_id} does not exist"));
    }

    if !local_index_enabled {
        transaction
            .execute(
                "DELETE FROM ai_message_fts WHERE account_id = ?1",
                params![account_id],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "DELETE FROM ai_embeddings WHERE account_id = ?1",
                params![account_id],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "DELETE FROM ai_message_state WHERE account_id = ?1",
                params![account_id],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "DELETE FROM ai_cache WHERE account_id = ?1",
                params![account_id],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "UPDATE ai_jobs
                 SET cancel_requested = 1, updated_at = CURRENT_TIMESTAMP
                 WHERE status IN ('queued', 'running') AND (account_id = ?1 OR account_id IS NULL)",
                params![account_id],
            )
            .map_err(|error| error.to_string())?;
    } else if !semantic_enabled {
        transaction
            .execute(
                "DELETE FROM ai_embeddings WHERE account_id = ?1",
                params![account_id],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "DELETE FROM ai_cache WHERE account_id = ?1 AND model IS NOT NULL",
                params![account_id],
            )
            .map_err(|error| error.to_string())?;
    }

    transaction.commit().map_err(|error| error.to_string())
}

pub fn create_index_job(
    connection: &Connection,
    account_id: Option<&str>,
    batch_size: usize,
) -> AiIndexResult<String> {
    let account_ids = enabled_account_ids(connection, account_id)?;
    if account_ids.is_empty() {
        return Err("No account is enabled for the local AI index".into());
    }

    let scoped_account = account_id.map(str::trim).filter(|value| !value.is_empty());
    let existing = connection
        .query_row(
            "SELECT id FROM ai_jobs
             WHERE kind = 'index' AND status IN ('queued', 'running')
               AND ((account_id IS NULL AND ?1 IS NULL) OR account_id = ?1)
             ORDER BY created_at ASC LIMIT 1",
            params![scoped_account],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if let Some(job_id) = existing {
        return Ok(job_id);
    }

    let cursor = BatchCursor::new(account_ids);
    let cursor_json = serde_json::to_string(&cursor).map_err(|error| error.to_string())?;
    let total_count = count_pending_messages(connection, scoped_account)?;
    let job_id = format!("ai-job-{}", uuid::Uuid::new_v4());
    connection
        .execute(
            "INSERT INTO ai_jobs (
               id, account_id, kind, status, batch_size, total_count, cursor_json
             ) VALUES (?1, ?2, 'index', 'queued', ?3, ?4, ?5)",
            params![
                job_id,
                scoped_account,
                batch_size.clamp(1, MAX_BATCH_SIZE) as i64,
                total_count as i64,
                cursor_json
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(job_id)
}

pub fn request_job_cancel(connection: &Connection, job_id: &str) -> AiIndexResult<bool> {
    let changed = connection
        .execute(
            "UPDATE ai_jobs
             SET cancel_requested = 1, updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1 AND status IN ('queued', 'running')",
            params![job_id],
        )
        .map_err(|error| error.to_string())?;
    Ok(changed > 0)
}

pub fn run_index_batch(connection: &mut Connection, job_id: &str) -> AiIndexResult<IndexProgress> {
    let mut job = load_job(connection, job_id)?;
    if matches!(job.status.as_str(), "completed" | "cancelled" | "failed") {
        return Ok(progress_from_job(job, 0));
    }
    if job.cancel_requested {
        mark_job_cancelled(connection, job_id)?;
        job = load_job(connection, job_id)?;
        return Ok(progress_from_job(job, 0));
    }

    let account_ids = enabled_account_ids(connection, job.account_id.as_deref())?;
    job.cursor.reconcile(&account_ids);
    if job.cursor.accounts.is_empty() {
        mark_job_completed(connection, job_id, &job.cursor)?;
        return Ok(progress_from_job(load_job(connection, job_id)?, 0));
    }

    connection
        .execute(
            "UPDATE ai_jobs
             SET status = 'running', started_at = COALESCE(started_at, CURRENT_TIMESTAMP),
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
            params![job_id],
        )
        .map_err(|error| error.to_string())?;

    let fetch_limit = job.batch_size.clamp(1, MAX_BATCH_SIZE);
    let mut queues = Vec::with_capacity(job.cursor.accounts.len());
    let mut fetched_counts = Vec::with_capacity(job.cursor.accounts.len());
    for account_cursor in &mut job.cursor.accounts {
        if account_cursor.exhausted {
            queues.push(VecDeque::new());
            fetched_counts.push(0);
            continue;
        }
        let messages = fetch_account_candidates(connection, account_cursor, fetch_limit)?;
        if messages.is_empty() {
            account_cursor.exhausted = true;
        }
        fetched_counts.push(messages.len());
        queues.push(messages.into());
    }

    let (selected, next_account_index) =
        fair_take(&mut queues, job.cursor.next_account_index, fetch_limit);
    job.cursor.next_account_index = next_account_index;
    let mut selected_counts = vec![0usize; job.cursor.accounts.len()];
    let mut batch_processed = 0usize;

    for (account_index, message) in selected {
        if job_cancel_requested(connection, job_id)? {
            mark_job_cancelled(connection, job_id)?;
            return Ok(progress_from_job(
                load_job(connection, job_id)?,
                batch_processed,
            ));
        }

        selected_counts[account_index] += 1;
        let content_hash = hash_message_content(
            &message.subject,
            &message.sender,
            &message.preview,
            &message.body_text,
        );
        let embedding_text = format!(
            "{}\n{}\n{}\n{}",
            message.subject, message.sender, message.preview, message.body_text
        );
        let chunk_count =
            chunk_text(&embedding_text, DEFAULT_CHUNK_CHARS, DEFAULT_CHUNK_OVERLAP).len();
        let mut next_cursor = job.cursor.clone();
        next_cursor.advance(
            &message.account_id,
            message.sort_key.clone(),
            message.id.clone(),
        );

        if let Err(error) = index_one_message(
            connection,
            job_id,
            &message,
            &content_hash,
            chunk_count,
            &next_cursor,
        ) {
            mark_job_failed(connection, job_id, &error)?;
            return Err(error);
        }
        job.cursor = next_cursor;
        job.processed_count += 1;
        batch_processed += 1;
    }

    for index in 0..job.cursor.accounts.len() {
        if fetched_counts[index] < fetch_limit && selected_counts[index] == fetched_counts[index] {
            job.cursor.accounts[index].exhausted = true;
        }
    }
    let complete = job.cursor.accounts.iter().all(|cursor| cursor.exhausted);
    let cursor_json = serde_json::to_string(&job.cursor).map_err(|error| error.to_string())?;
    connection
        .execute(
            "UPDATE ai_jobs
             SET status = ?2, cursor_json = ?3,
                 finished_at = CASE WHEN ?2 = 'completed' THEN CURRENT_TIMESTAMP ELSE finished_at END,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
            params![job_id, if complete { "completed" } else { "running" }, cursor_json],
        )
        .map_err(|error| error.to_string())?;

    Ok(progress_from_job(
        load_job(connection, job_id)?,
        batch_processed,
    ))
}

pub fn store_embedding(
    connection: &Connection,
    message_id: &str,
    model: &str,
    chunk_index: usize,
    chunk_hash: &str,
    vector: &[f32],
) -> AiIndexResult<()> {
    if model.trim().is_empty() || chunk_hash.trim().is_empty() {
        return Err("Embedding model and chunk hash are required".into());
    }
    if vector.is_empty() || vector.iter().any(|value| !value.is_finite()) {
        return Err("Embedding vector must contain finite values".into());
    }
    let state = connection
        .query_row(
            "SELECT m.account_id, settings.semantic_enabled, state.content_hash
             FROM messages m
             JOIN ai_account_settings settings ON settings.account_id = m.account_id
             JOIN ai_message_state state ON state.message_id = m.id
             WHERE m.id = ?1 AND settings.local_index_enabled = 1",
            params![message_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)? == 1,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Message is not released and ready for local AI indexing".to_string())?;
    if !state.1 {
        return Err("Semantic indexing is disabled for this account".into());
    }

    connection
        .execute(
            "INSERT INTO ai_embeddings (
               message_id, account_id, model, chunk_index, dimensions, vector,
               content_hash, chunk_hash, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, CURRENT_TIMESTAMP)
             ON CONFLICT(message_id, model, chunk_index) DO UPDATE SET
               account_id = excluded.account_id,
               dimensions = excluded.dimensions,
               vector = excluded.vector,
               content_hash = excluded.content_hash,
               chunk_hash = excluded.chunk_hash,
               updated_at = CURRENT_TIMESTAMP",
            params![
                message_id,
                state.0,
                model.trim(),
                chunk_index as i64,
                vector.len() as i64,
                encode_f32_vector(vector),
                state.2,
                chunk_hash
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub fn hash_message_content(subject: &str, sender: &str, preview: &str, body_text: &str) -> String {
    let mut digest = Sha256::new();
    for field in [subject, sender, preview, body_text] {
        let bytes = field.as_bytes();
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn chunk_text(input: &str, max_chars: usize, overlap_chars: usize) -> Vec<String> {
    if max_chars == 0 {
        return Vec::new();
    }
    let words = input.split_whitespace().collect::<Vec<_>>();
    if words.is_empty() {
        return Vec::new();
    }

    let overlap_chars = overlap_chars.min(max_chars.saturating_sub(1));
    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < words.len() {
        let mut end = start;
        let mut char_count = 0usize;
        while end < words.len() {
            let word_chars = words[end].chars().count();
            let separator = usize::from(end > start);
            if end > start && char_count + separator + word_chars > max_chars {
                break;
            }
            char_count += separator + word_chars;
            end += 1;
            if end == start + 1 && word_chars > max_chars {
                break;
            }
        }
        if end == start {
            end += 1;
        }
        chunks.push(words[start..end].join(" "));
        if end >= words.len() {
            break;
        }

        let mut next_start = end;
        let mut overlap = 0usize;
        while next_start > start {
            let word_chars = words[next_start - 1].chars().count();
            let separator = usize::from(overlap > 0);
            if overlap + separator + word_chars > overlap_chars {
                break;
            }
            overlap += separator + word_chars;
            next_start -= 1;
        }
        start = if next_start == start { end } else { next_start };
    }
    chunks
}

fn enabled_account_ids(
    connection: &Connection,
    account_id: Option<&str>,
) -> AiIndexResult<Vec<String>> {
    let account_id = account_id.map(str::trim).filter(|value| !value.is_empty());
    let mut statement = connection
        .prepare(
            "SELECT a.id
             FROM accounts a
             JOIN ai_account_settings settings ON settings.account_id = a.id
             WHERE settings.local_index_enabled = 1 AND (?1 IS NULL OR a.id = ?1)
             ORDER BY a.created_at ASC, a.id ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![account_id], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn count_pending_messages(
    connection: &Connection,
    account_id: Option<&str>,
) -> AiIndexResult<usize> {
    connection
        .query_row(
            "SELECT COUNT(*)
             FROM messages m
             JOIN ai_account_settings settings ON settings.account_id = m.account_id
             LEFT JOIN ai_message_state state ON state.message_id = m.id
             WHERE settings.local_index_enabled = 1
               AND (?1 IS NULL OR m.account_id = ?1)
               AND (
                 state.message_id IS NULL OR state.status <> 'ready' OR
                 datetime(m.updated_at) > datetime(state.indexed_at)
               )",
            params![account_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count.max(0) as usize)
        .map_err(|error| error.to_string())
}

fn load_job(connection: &Connection, job_id: &str) -> AiIndexResult<IndexJob> {
    connection
        .query_row(
            "SELECT id, account_id, status, batch_size, processed_count, total_count,
                    cursor_json, cancel_requested, last_error
             FROM ai_jobs WHERE id = ?1",
            params![job_id],
            |row| {
                let cursor_json = row.get::<_, String>(6)?;
                let cursor = serde_json::from_str::<BatchCursor>(&cursor_json).unwrap_or_default();
                Ok(IndexJob {
                    id: row.get(0)?,
                    account_id: row.get(1)?,
                    status: row.get(2)?,
                    batch_size: row.get::<_, i64>(3)?.max(1) as usize,
                    processed_count: row.get::<_, i64>(4)?.max(0) as usize,
                    total_count: row.get::<_, i64>(5)?.max(0) as usize,
                    cursor,
                    cancel_requested: row.get::<_, i64>(7)? == 1,
                    last_error: row.get(8)?,
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("AI job {job_id} was not found"))
}

fn fetch_account_candidates(
    connection: &Connection,
    cursor: &AccountBatchCursor,
    limit: usize,
) -> AiIndexResult<Vec<IndexMessage>> {
    let mut statement = connection
        .prepare(
            "SELECT m.id, m.account_id, m.subject, m.from_addr,
                    substr(m.body_text, 1, 240), m.body_text,
                    COALESCE(m.updated_at, m.created_at)
             FROM messages m
             LEFT JOIN ai_message_state state ON state.message_id = m.id
             WHERE m.account_id = ?1
               AND (
                 state.message_id IS NULL OR state.status <> 'ready' OR
                 datetime(m.updated_at) > datetime(state.indexed_at)
               )
               AND (
                 ?2 IS NULL OR COALESCE(m.updated_at, m.created_at) > ?2 OR
                 (COALESCE(m.updated_at, m.created_at) = ?2 AND m.id > COALESCE(?3, ''))
               )
             ORDER BY COALESCE(m.updated_at, m.created_at) ASC, m.id ASC
             LIMIT ?4",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![
                cursor.account_id,
                cursor.last_sort_key,
                cursor.last_message_id,
                limit as i64
            ],
            |row| {
                Ok(IndexMessage {
                    id: row.get(0)?,
                    account_id: row.get(1)?,
                    subject: row.get(2)?,
                    sender: row.get(3)?,
                    preview: row.get(4)?,
                    body_text: row.get(5)?,
                    sort_key: row.get(6)?,
                })
            },
        )
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn fair_take<T>(
    queues: &mut [VecDeque<T>],
    start_index: usize,
    limit: usize,
) -> (Vec<(usize, T)>, usize) {
    if queues.is_empty() || limit == 0 {
        return (Vec::new(), 0);
    }
    let mut output = Vec::with_capacity(limit);
    let mut index = start_index % queues.len();
    let mut misses = 0usize;
    while output.len() < limit && misses < queues.len() {
        if let Some(item) = queues[index].pop_front() {
            output.push((index, item));
            misses = 0;
        } else {
            misses += 1;
        }
        index = (index + 1) % queues.len();
    }
    (output, index)
}

fn index_one_message(
    connection: &mut Connection,
    job_id: &str,
    message: &IndexMessage,
    content_hash: &str,
    chunk_count: usize,
    cursor: &BatchCursor,
) -> AiIndexResult<()> {
    let cursor_json = serde_json::to_string(cursor).map_err(|error| error.to_string())?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "DELETE FROM ai_message_fts WHERE message_id = ?1",
            params![message.id],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "INSERT INTO ai_message_fts (
               message_id, account_id, subject, sender, preview, body_text
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                message.id,
                message.account_id,
                message.subject,
                message.sender,
                message.preview,
                message.body_text
            ],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "DELETE FROM ai_embeddings WHERE message_id = ?1 AND content_hash <> ?2",
            params![message.id, content_hash],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "DELETE FROM ai_cache WHERE message_id = ?1 AND content_hash <> ?2",
            params![message.id, content_hash],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "INSERT INTO ai_message_state (
               message_id, account_id, content_hash, chunk_count, status,
               last_error, indexed_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, 'ready', NULL, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
             ON CONFLICT(message_id) DO UPDATE SET
               account_id = excluded.account_id,
               content_hash = excluded.content_hash,
               chunk_count = excluded.chunk_count,
               status = 'ready',
               last_error = NULL,
               indexed_at = CURRENT_TIMESTAMP,
               updated_at = CURRENT_TIMESTAMP",
            params![
                message.id,
                message.account_id,
                content_hash,
                chunk_count as i64
            ],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE ai_jobs
             SET processed_count = processed_count + 1, cursor_json = ?2,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
            params![job_id, cursor_json],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())
}

fn job_cancel_requested(connection: &Connection, job_id: &str) -> AiIndexResult<bool> {
    connection
        .query_row(
            "SELECT cancel_requested FROM ai_jobs WHERE id = ?1",
            params![job_id],
            |row| Ok(row.get::<_, i64>(0)? == 1),
        )
        .map_err(|error| error.to_string())
}

fn mark_job_cancelled(connection: &Connection, job_id: &str) -> AiIndexResult<()> {
    connection
        .execute(
            "UPDATE ai_jobs
             SET status = 'cancelled', finished_at = CURRENT_TIMESTAMP,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
            params![job_id],
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn mark_job_completed(
    connection: &Connection,
    job_id: &str,
    cursor: &BatchCursor,
) -> AiIndexResult<()> {
    let cursor_json = serde_json::to_string(cursor).map_err(|error| error.to_string())?;
    connection
        .execute(
            "UPDATE ai_jobs
             SET status = 'completed', cursor_json = ?2,
                 finished_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
            params![job_id, cursor_json],
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn mark_job_failed(connection: &Connection, job_id: &str, error: &str) -> AiIndexResult<()> {
    connection
        .execute(
            "UPDATE ai_jobs
             SET status = 'failed', last_error = ?2,
                 finished_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
            params![job_id, error],
        )
        .map(|_| ())
        .map_err(|db_error| db_error.to_string())
}

fn progress_from_job(job: IndexJob, batch_processed: usize) -> IndexProgress {
    IndexProgress {
        job_id: job.id,
        status: job.status,
        processed_count: job.processed_count,
        total_count: job.total_count,
        batch_processed,
        cancel_requested: job.cancel_requested,
        cursor: job.cursor,
        last_error: job.last_error,
    }
}

fn encode_f32_vector(vector: &[f32]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(vector.len() * std::mem::size_of::<f32>());
    for value in vector {
        blob.extend_from_slice(&value.to_le_bytes());
    }
    blob
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunking_respects_boundaries_and_overlap() {
        let chunks = chunk_text("one two three four five six", 15, 5);
        assert_eq!(chunks, vec!["one two three", "three four five", "five six"]);
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 15));
    }

    #[test]
    fn hashing_is_stable_and_field_boundaries_matter() {
        let first = hash_message_content("ab", "c", "", "body");
        let repeated = hash_message_content("ab", "c", "", "body");
        let different_boundaries = hash_message_content("a", "bc", "", "body");
        assert_eq!(first, repeated);
        assert_ne!(first, different_boundaries);
        assert_eq!(first.len(), 64);
    }

    #[test]
    fn batch_cursor_round_trips_and_advances_one_account() {
        let mut cursor = BatchCursor::new(vec!["a".into(), "b".into()]);
        cursor.next_account_index = 1;
        cursor.advance("b", "2026-07-12 10:00:00".into(), "msg-b-2".into());
        let encoded = serde_json::to_string(&cursor).unwrap();
        let decoded: BatchCursor = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, cursor);
        assert_eq!(decoded.accounts[0].last_message_id, None);
        assert_eq!(
            decoded.accounts[1].last_message_id.as_deref(),
            Some("msg-b-2")
        );
    }

    #[test]
    fn fair_take_rotates_accounts_without_starvation() {
        let mut queues = vec![
            VecDeque::from(vec!["a1", "a2", "a3"]),
            VecDeque::from(vec!["b1"]),
            VecDeque::from(vec!["c1", "c2"]),
        ];
        let (items, next) = fair_take(&mut queues, 1, 5);
        assert_eq!(
            items,
            vec![(1, "b1"), (2, "c1"), (0, "a1"), (2, "c2"), (0, "a2")]
        );
        assert_eq!(next, 1);
    }

    #[test]
    fn migration_creates_required_ai_schema() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE accounts (
                   id TEXT PRIMARY KEY,
                   created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                 );
                 CREATE TABLE messages (
                   id TEXT PRIMARY KEY,
                   account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
                   subject TEXT NOT NULL DEFAULT '',
                   from_addr TEXT NOT NULL DEFAULT '',
                   body_text TEXT NOT NULL DEFAULT ''
                 );",
            )
            .unwrap();
        apply_ai_index_migration(&connection).unwrap();
        apply_ai_index_migration(&connection).unwrap();

        for table in [
            "ai_account_settings",
            "ai_message_fts",
            "ai_message_state",
            "ai_embeddings",
            "ai_jobs",
            "ai_cache",
        ] {
            let count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE name = ?1",
                    params![table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "missing schema object {table}");
        }

        let default_model: String = connection
            .query_row(
                "SELECT generation_model FROM ai_app_settings WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(default_model, DEFAULT_GEMINI_GENERATION_MODEL);

        connection
            .execute(
                "UPDATE ai_app_settings SET generation_model = ?1 WHERE id = 1",
                params![LEGACY_GEMINI_GENERATION_MODEL],
            )
            .unwrap();
        apply_ai_index_migration(&connection).unwrap();
        let migrated_model: String = connection
            .query_row(
                "SELECT generation_model FROM ai_app_settings WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(migrated_model, DEFAULT_GEMINI_GENERATION_MODEL);

        connection
            .execute(
                "UPDATE ai_app_settings SET generation_model = 'custom-model' WHERE id = 1",
                [],
            )
            .unwrap();
        apply_ai_index_migration(&connection).unwrap();
        let custom_model: String = connection
            .query_row(
                "SELECT generation_model FROM ai_app_settings WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(custom_model, "custom-model");
    }
}
