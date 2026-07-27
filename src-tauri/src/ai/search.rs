use rusqlite::{params_from_iter, Connection};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

pub type AiSearchResult<T> = Result<T, String>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticQuery {
    pub model: String,
    pub vector: Vec<f32>,
    #[serde(default = "default_semantic_weight")]
    pub weight: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    #[serde(default)]
    pub account_ids: Vec<String>,
    #[serde(default = "default_result_limit")]
    pub limit: usize,
    #[serde(default)]
    pub semantic: Option<SemanticQuery>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessageSearchResult {
    pub message_id: String,
    pub account_id: String,
    pub date: String,
    pub sender: String,
    pub subject: String,
    pub snippet: String,
    pub reason: String,
    pub metadata_score: f32,
    pub fts_score: f32,
    pub semantic_score: Option<f32>,
    pub score: f32,
}

#[derive(Debug, Clone)]
struct LexicalCandidate {
    message_id: String,
    account_id: String,
    date: String,
    sender: String,
    subject: String,
    preview: String,
    body_text: String,
    snippet: String,
}

pub fn search_messages(
    connection: &Connection,
    request: &SearchRequest,
) -> AiSearchResult<Vec<MessageSearchResult>> {
    let Some(fts_query) = build_fts_query(&request.query) else {
        return Ok(Vec::new());
    };
    let result_limit = request.limit.clamp(1, 100);
    let candidate_limit = (result_limit * 5).clamp(result_limit, 500);
    let candidates = load_lexical_candidates(
        connection,
        &fts_query,
        &request.account_ids,
        candidate_limit,
    )?;
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let semantic_scores = match request.semantic.as_ref() {
        Some(semantic) => {
            validate_semantic_query(semantic)?;
            load_semantic_scores(connection, &candidates, semantic)?
        }
        None => HashMap::new(),
    };
    let semantic_weight = request
        .semantic
        .as_ref()
        .map(|semantic| semantic.weight.clamp(0.0, 0.7))
        .unwrap_or(0.0);
    let normalized_query = request.query.trim().to_lowercase();

    let mut results = candidates
        .into_iter()
        .enumerate()
        .map(|(rank, candidate)| {
            let fts_score = reciprocal_rank_score(rank);
            let (metadata_score, metadata_reason) = metadata_match(
                &candidate.subject,
                &candidate.sender,
                &normalized_query,
            );
            let semantic_score = semantic_scores.get(&candidate.message_id).copied();
            let score = hybrid_score(
                metadata_score,
                fts_score,
                semantic_score,
                semantic_weight,
            );
            let snippet = first_non_empty([
                candidate.snippet.as_str(),
                candidate.preview.as_str(),
                candidate.body_text.as_str(),
            ])
            .map(|value| truncate_chars(value, 320))
            .unwrap_or_default();
            let reason = match (metadata_reason, semantic_score) {
                (Some(reason), Some(value)) => {
                    format!("{reason}; semantisch aehnlich ({value:.2})")
                }
                (Some(reason), None) => reason,
                (None, Some(value)) => format!(
                    "Volltexttreffer in Vorschau oder Nachrichtentext; semantisch aehnlich ({value:.2})"
                ),
                (None, None) => "Volltexttreffer in Vorschau oder Nachrichtentext".into(),
            };
            MessageSearchResult {
                message_id: candidate.message_id,
                account_id: candidate.account_id,
                date: candidate.date,
                sender: candidate.sender,
                subject: candidate.subject,
                snippet,
                reason,
                metadata_score,
                fts_score,
                semantic_score,
                score,
            }
        })
        .collect::<Vec<_>>();

    results.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| right.date.cmp(&left.date))
            .then_with(|| left.message_id.cmp(&right.message_id))
    });
    results.truncate(result_limit);
    Ok(results)
}

pub fn build_fts_query(input: &str) -> Option<String> {
    let mut terms = Vec::new();
    let mut current = String::new();
    for character in input.chars() {
        if character.is_alphanumeric() || matches!(character, '@' | '_' | '-' | '.') {
            current.push(character);
        } else if !current.is_empty() {
            terms.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        terms.push(current);
    }
    terms.retain(|term| term.chars().any(char::is_alphanumeric));
    terms.dedup();
    if terms.is_empty() {
        return None;
    }
    Some(
        terms
            .into_iter()
            .map(|term| format!("\"{}\"*", term.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" AND "),
    )
}

pub fn cosine_similarity(left: &[f32], right: &[f32]) -> Option<f32> {
    if left.is_empty() || left.len() != right.len() {
        return None;
    }
    let mut dot = 0.0f64;
    let mut left_norm = 0.0f64;
    let mut right_norm = 0.0f64;
    for (&left_value, &right_value) in left.iter().zip(right) {
        if !left_value.is_finite() || !right_value.is_finite() {
            return None;
        }
        let left_value = left_value as f64;
        let right_value = right_value as f64;
        dot += left_value * right_value;
        left_norm += left_value * left_value;
        right_norm += right_value * right_value;
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        return None;
    }
    Some((dot / (left_norm.sqrt() * right_norm.sqrt())).clamp(-1.0, 1.0) as f32)
}

pub fn hybrid_score(
    metadata_score: f32,
    fts_score: f32,
    semantic_score: Option<f32>,
    semantic_weight: f32,
) -> f32 {
    let base = 0.35 * metadata_score.clamp(0.0, 1.0) + 0.65 * fts_score.clamp(0.0, 1.0);
    match semantic_score {
        Some(semantic) => {
            let weight = semantic_weight.clamp(0.0, 0.7);
            let normalized_semantic = ((semantic.clamp(-1.0, 1.0) + 1.0) / 2.0).clamp(0.0, 1.0);
            (1.0 - weight) * base + weight * normalized_semantic
        }
        None => base,
    }
}

fn load_lexical_candidates(
    connection: &Connection,
    fts_query: &str,
    requested_account_ids: &[String],
    limit: usize,
) -> AiSearchResult<Vec<LexicalCandidate>> {
    let account_ids = requested_account_ids
        .iter()
        .map(|account_id| account_id.trim())
        .filter(|account_id| !account_id.is_empty())
        .collect::<HashSet<_>>();
    let mut bind_values = vec![fts_query.to_string()];
    let account_filter = if account_ids.is_empty() {
        String::new()
    } else {
        let mut placeholders = Vec::with_capacity(account_ids.len());
        let mut sorted_accounts = account_ids.into_iter().collect::<Vec<_>>();
        sorted_accounts.sort_unstable();
        for account_id in sorted_accounts {
            bind_values.push(account_id.to_string());
            placeholders.push(format!("?{}", bind_values.len()));
        }
        format!(" AND fts.account_id IN ({})", placeholders.join(", "))
    };
    bind_values.push(limit.to_string());
    let limit_parameter = bind_values.len();
    let sql = format!(
        "SELECT fts.message_id,
                fts.account_id,
                COALESCE(m.date, m.created_at),
                fts.sender,
                fts.subject,
                fts.preview,
                fts.body_text,
                CASE
                  WHEN trim(fts.preview) <> ''
                    THEN snippet(ai_message_fts, 4, '[', ']', ' ... ', 24)
                  ELSE snippet(ai_message_fts, 5, '[', ']', ' ... ', 24)
                END AS match_snippet
         FROM ai_message_fts fts
         JOIN messages m ON m.id = fts.message_id AND m.account_id = fts.account_id
         JOIN ai_account_settings settings ON settings.account_id = fts.account_id
         JOIN ai_message_state state ON state.message_id = fts.message_id
         WHERE ai_message_fts MATCH ?1
           AND settings.local_index_enabled = 1
           AND state.status = 'ready'
           {account_filter}
         ORDER BY bm25(ai_message_fts, 0.0, 0.0, 5.0, 3.5, 2.0, 1.0) ASC,
                  datetime(COALESCE(m.date, m.created_at)) DESC,
                  m.id ASC
         LIMIT ?{limit_parameter}"
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params_from_iter(bind_values.iter()), |row| {
            Ok(LexicalCandidate {
                message_id: row.get(0)?,
                account_id: row.get(1)?,
                date: row.get(2)?,
                sender: row.get(3)?,
                subject: row.get(4)?,
                preview: row.get(5)?,
                body_text: row.get(6)?,
                snippet: row.get(7)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn load_semantic_scores(
    connection: &Connection,
    candidates: &[LexicalCandidate],
    semantic: &SemanticQuery,
) -> AiSearchResult<HashMap<String, f32>> {
    let mut scores = HashMap::<String, f32>::new();
    for candidate_chunk in candidates.chunks(400) {
        let mut bind_values = vec![semantic.model.trim().to_string()];
        let mut placeholders = Vec::with_capacity(candidate_chunk.len());
        for candidate in candidate_chunk {
            bind_values.push(candidate.message_id.clone());
            placeholders.push(format!("?{}", bind_values.len()));
        }
        let sql = format!(
            "SELECT embedding.message_id, embedding.dimensions, embedding.vector
             FROM ai_embeddings embedding
             JOIN ai_account_settings settings ON settings.account_id = embedding.account_id
             JOIN ai_message_state state ON state.message_id = embedding.message_id
             WHERE embedding.model = ?1
               AND embedding.message_id IN ({})
               AND settings.local_index_enabled = 1
               AND settings.semantic_enabled = 1
               AND state.status = 'ready'
               AND embedding.content_hash = state.content_hash",
            placeholders.join(", ")
        );
        let mut statement = connection
            .prepare(&sql)
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params_from_iter(bind_values.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        for row in rows {
            let (message_id, dimensions, blob) = row.map_err(|error| error.to_string())?;
            let Some(vector) = decode_f32_vector(&blob, dimensions.max(0) as usize) else {
                continue;
            };
            let Some(score) = cosine_similarity(&semantic.vector, &vector) else {
                continue;
            };
            scores
                .entry(message_id)
                .and_modify(|current| *current = current.max(score))
                .or_insert(score);
        }
    }
    Ok(scores)
}

fn validate_semantic_query(semantic: &SemanticQuery) -> AiSearchResult<()> {
    if semantic.model.trim().is_empty() {
        return Err("Semantic model is required".into());
    }
    if semantic.vector.is_empty() || semantic.vector.iter().any(|value| !value.is_finite()) {
        return Err("Semantic query vector must contain finite values".into());
    }
    Ok(())
}

fn decode_f32_vector(blob: &[u8], dimensions: usize) -> Option<Vec<f32>> {
    if dimensions == 0 || blob.len() != dimensions.checked_mul(std::mem::size_of::<f32>())? {
        return None;
    }
    let mut vector = Vec::with_capacity(dimensions);
    for bytes in blob.chunks_exact(4) {
        let value = f32::from_le_bytes(bytes.try_into().ok()?);
        if !value.is_finite() {
            return None;
        }
        vector.push(value);
    }
    Some(vector)
}

fn reciprocal_rank_score(rank: usize) -> f32 {
    1.0 / (1.0 + rank as f32)
}

fn metadata_match(subject: &str, sender: &str, normalized_query: &str) -> (f32, Option<String>) {
    if normalized_query.is_empty() {
        return (0.0, None);
    }
    let subject = subject.to_lowercase();
    let sender = sender.to_lowercase();
    if subject == normalized_query {
        (1.0, Some("Exakter Treffer im Betreff".into()))
    } else if sender == normalized_query {
        (0.95, Some("Exakter Treffer beim Absender".into()))
    } else if subject.contains(normalized_query) {
        (0.85, Some("Treffer im Betreff".into()))
    } else if sender.contains(normalized_query) {
        (0.75, Some("Treffer beim Absender".into()))
    } else {
        (0.0, None)
    }
}

fn first_non_empty<'a>(values: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    values.into_iter().find(|value| !value.trim().is_empty())
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_chars {
        return normalized;
    }
    normalized.chars().take(max_chars).collect::<String>() + "..."
}

fn default_result_limit() -> usize {
    25
}

fn default_semantic_weight() -> f32 {
    0.25
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fts_query_uses_safe_prefix_terms() {
        assert_eq!(
            build_fts_query("invoice (Alice) 2026"),
            Some("\"invoice\"* AND \"Alice\"* AND \"2026\"*".into())
        );
        assert_eq!(build_fts_query("***"), None);
    }

    #[test]
    fn cosine_similarity_rejects_bad_dimensions() {
        assert_eq!(cosine_similarity(&[1.0, 0.0], &[1.0]), None);
        assert_eq!(cosine_similarity(&[0.0, 0.0], &[0.0, 0.0]), None);
        assert!((cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]).unwrap() - 1.0).abs() < 0.0001);
    }

    #[test]
    fn semantic_score_can_rerank_but_cannot_create_a_result() {
        let lexical_first = hybrid_score(0.5, 0.8, Some(0.0), 0.35);
        let semantic_first = hybrid_score(0.2, 0.6, Some(1.0), 0.35);
        assert!(semantic_first > lexical_first);
        assert_eq!(hybrid_score(0.2, 0.6, None, 0.35), 0.46);
    }

    #[test]
    fn rank_sort_is_deterministic_for_equal_scores() {
        let mut results = vec![
            MessageSearchResult {
                message_id: "b".into(),
                account_id: "a".into(),
                date: "2026-07-11".into(),
                sender: "Sender".into(),
                subject: "Subject".into(),
                snippet: "Body".into(),
                reason: "FTS".into(),
                metadata_score: 0.0,
                fts_score: 0.5,
                semantic_score: None,
                score: 0.5,
            },
            MessageSearchResult {
                message_id: "a".into(),
                account_id: "a".into(),
                date: "2026-07-12".into(),
                sender: "Sender".into(),
                subject: "Subject".into(),
                snippet: "Body".into(),
                reason: "FTS".into(),
                metadata_score: 0.0,
                fts_score: 0.5,
                semantic_score: None,
                score: 0.5,
            },
        ];
        results.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| right.date.cmp(&left.date))
                .then_with(|| left.message_id.cmp(&right.message_id))
        });
        assert_eq!(results[0].message_id, "a");
    }

    #[test]
    fn finder_returns_only_real_messages_from_enabled_accounts() {
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
                   body_text TEXT NOT NULL DEFAULT '',
                   date TEXT,
                   created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                   updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                 );",
            )
            .unwrap();
        connection
            .execute_batch(include_str!("../../migrations/0100_ai_local_index.sql"))
            .unwrap();
        connection
            .execute_batch(
                "INSERT INTO accounts (id) VALUES ('enabled'), ('disabled');
                 INSERT INTO messages (id, account_id, subject, from_addr, body_text, date)
                 VALUES
                   ('msg-real', 'enabled', 'Invoice July', 'alice@example.com', 'The invoice is attached.', '2026-07-12'),
                   ('msg-hidden', 'disabled', 'Invoice Secret', 'hidden@example.com', 'Must not appear.', '2026-07-11');
                 INSERT INTO ai_account_settings (account_id, local_index_enabled, semantic_enabled)
                 VALUES ('enabled', 1, 0), ('disabled', 0, 0);
                 INSERT INTO ai_message_state (message_id, account_id, content_hash, status)
                 VALUES ('msg-real', 'enabled', 'hash-real', 'ready'),
                        ('msg-hidden', 'disabled', 'hash-hidden', 'ready');
                 INSERT INTO ai_message_fts (message_id, account_id, subject, sender, preview, body_text)
                 VALUES ('msg-real', 'enabled', 'Invoice July', 'alice@example.com', 'The invoice is attached.', 'The invoice is attached.'),
                        ('msg-hidden', 'disabled', 'Invoice Secret', 'hidden@example.com', 'Must not appear.', 'Must not appear.');",
            )
            .unwrap();

        let results = search_messages(
            &connection,
            &SearchRequest {
                query: "invoice".into(),
                account_ids: Vec::new(),
                limit: 10,
                semantic: None,
            },
        )
        .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].message_id, "msg-real");
        assert_eq!(results[0].account_id, "enabled");
        assert_eq!(results[0].sender, "alice@example.com");
        assert_eq!(results[0].subject, "Invoice July");
        assert_eq!(results[0].date, "2026-07-12");
        assert!(!results[0].snippet.is_empty());
        assert!(!results[0].reason.is_empty());
    }
}
