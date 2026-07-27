use super::index::{
    chunk_text, create_index_job, hash_message_content, request_job_cancel, run_index_batch,
    set_account_access, store_embedding, IndexProgress, DEFAULT_BATCH_SIZE,
};
use super::models::{
    CancellationFlag, EmbeddingPurpose, EmbeddingRequest, GeminiConfig, GenerationOptions,
    GenerationRequest, OllamaConfig, ProviderConfig, UntrustedText,
};
use super::provider::{create_provider, normalize_gemini_model, AiProvider};
use super::search::{search_messages, MessageSearchResult, SearchRequest, SemanticQuery};
use super::security::{
    build_finder_rerank_prompt, build_quick_reply_prompt, build_summary_prompt,
    redact_inline_secrets, validate_quick_reply_output, validate_summary_output, FinderCandidate,
    InferenceTarget, MailPromptInput, PromptAttachment, PromptRequest, PromptSecurityOptions,
};
use crate::{AppState, KEYRING_SERVICES};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use tauri::{AppHandle, Manager, State};

const GEMINI_SECRET_USER: &str = "ai:gemini:api_key";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettings {
    enabled: bool,
    account_id: Option<String>,
    provider: String,
    generation_model: String,
    embedding_model: String,
    ollama_base_url: Option<String>,
    gemini_key_configured: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettingsUpdateInput {
    enabled: bool,
    account_id: Option<String>,
    provider: String,
    generation_model: String,
    embedding_model: String,
    ollama_base_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiMessageActionInput {
    message_id: String,
    account_id: String,
    #[serde(default)]
    include_attachments: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSummaryResult {
    summary: String,
    evidence: Vec<String>,
    provider: String,
    model: String,
    generated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiQuickReply {
    id: String,
    label: String,
    tone: String,
    text: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiQuickRepliesResult {
    replies: Vec<AiQuickReply>,
    provider: String,
    model: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSearchInput {
    query: String,
    account_id: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSearchHit {
    message_id: String,
    account_id: String,
    sender: String,
    sender_email: String,
    subject: String,
    date: String,
    snippet: String,
    evidence: Vec<String>,
    score: f32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSearchResponse {
    query: String,
    hits: Vec<AiSearchHit>,
    summary: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiIndexInput {
    account_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiIndexStatus {
    state: String,
    account_id: Option<String>,
    processed: usize,
    total: usize,
    progress: f32,
    message: String,
    last_error: Option<String>,
    updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AiSecretSetInput {
    provider: String,
    secret: String,
}

#[derive(Debug, Deserialize)]
pub struct AiSecretDeleteInput {
    provider: String,
}

#[tauri::command]
pub fn ai_settings_get(state: State<'_, AppState>) -> Result<AiSettings, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    load_settings(&db)
}

#[tauri::command]
pub fn ai_settings_update(
    state: State<'_, AppState>,
    input: AiSettingsUpdateInput,
) -> Result<AiSettings, String> {
    validate_settings_input(&input)?;
    let mut db = state.db.lock().map_err(|error| error.to_string())?;
    let account_id = normalize_optional(input.account_id);
    if let Some(ref account_id) = account_id {
        ensure_account_exists(&db, account_id)?;
    }
    let ollama_base_url = input
        .ollama_base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("http://127.0.0.1:11434");
    let generation_model = if input.provider.trim() == "gemini" {
        normalize_gemini_model(input.generation_model.trim())
    } else {
        input.generation_model.trim().to_string()
    };
    db.execute(
        "UPDATE ai_app_settings SET enabled = ?1, account_id = ?2, provider = ?3,
         generation_model = ?4, embedding_model = ?5, ollama_base_url = ?6,
         updated_at = CURRENT_TIMESTAMP WHERE id = 1",
        params![
            input.enabled as i64,
            account_id,
            input.provider.trim(),
            generation_model,
            input.embedding_model.trim(),
            ollama_base_url
        ],
    )
    .map_err(|error| error.to_string())?;

    let account_ids = db
        .prepare("SELECT id FROM accounts ORDER BY created_at")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(|error| error.to_string())?;
    for current_id in account_ids {
        let selected = account_id
            .as_ref()
            .map(|selected_id| selected_id == &current_id)
            .unwrap_or(true);
        set_account_access(
            &mut db,
            &current_id,
            input.enabled && selected,
            input.enabled && selected,
        )?;
    }
    load_settings(&db)
}

#[tauri::command]
pub fn ai_secret_set(input: AiSecretSetInput) -> Result<(), String> {
    if input.provider != "gemini" {
        return Err("Nur Gemini verwendet derzeit einen API-Key.".into());
    }
    let secret = input.secret.trim();
    if secret.len() < 12 || secret.bytes().any(|byte| byte.is_ascii_control()) {
        return Err("Der Gemini API-Key ist leer oder ungueltig.".into());
    }
    let mut stored = false;
    let mut last_error = None;
    for service in KEYRING_SERVICES {
        match keyring::Entry::new(service, GEMINI_SECRET_USER) {
            Ok(entry) => match entry.set_password(secret) {
                Ok(()) => stored = true,
                Err(error) => last_error = Some(error.to_string()),
            },
            Err(error) => last_error = Some(error.to_string()),
        }
    }
    if stored {
        Ok(())
    } else {
        Err(format!(
            "Gemini-Key konnte nicht sicher gespeichert werden: {}",
            last_error.unwrap_or_else(|| "kein Credential-Service erreichbar".into())
        ))
    }
}

#[tauri::command]
pub fn ai_secret_delete(input: AiSecretDeleteInput) -> Result<(), String> {
    if input.provider != "gemini" {
        return Err("Nur Gemini verwendet derzeit einen API-Key.".into());
    }
    let mut completed = false;
    let mut failed = None;
    for service in KEYRING_SERVICES {
        match keyring::Entry::new(service, GEMINI_SECRET_USER) {
            Ok(entry) => match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => completed = true,
                Err(error) => failed = Some(error.to_string()),
            },
            Err(error) => failed = Some(error.to_string()),
        }
    }
    if completed {
        Ok(())
    } else {
        Err(format!(
            "Gemini-Key konnte nicht geloescht werden: {}",
            failed.unwrap_or_else(|| "kein Credential-Service erreichbar".into())
        ))
    }
}

#[tauri::command]
pub async fn ai_summary_generate(
    app: AppHandle,
    input: AiMessageActionInput,
) -> Result<AiSummaryResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let (settings, mail) = load_action_context(&state, &input)?;
        let options = security_options(&settings, input.include_attachments);
        let secure_prompt = build_summary_prompt(&mail, &options).map_err(safe_error)?;
        let provider = provider_for_settings(&settings)?;
        let response = generate_secure(provider.as_ref(), secure_prompt, 900)?;
        let output = validate_summary_output(&response.text).map_err(safe_error)?;
        let mut evidence = output.key_points;
        evidence.extend(
            output
                .action_items
                .into_iter()
                .map(|item| format!("Aktion: {item}")),
        );
        Ok(AiSummaryResult {
            summary: output.summary,
            evidence,
            provider: settings.provider,
            model: response.model,
            generated_at: now_label(),
        })
    })
    .await
    .map_err(|error| format!("KI-Worker wurde unerwartet beendet: {error}"))?
}

#[tauri::command]
pub async fn ai_quick_replies_generate(
    app: AppHandle,
    input: AiMessageActionInput,
) -> Result<AiQuickRepliesResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let (settings, mail) = load_action_context(&state, &input)?;
        let options = security_options(&settings, input.include_attachments);
        let provider = provider_for_settings(&settings)?;
        let tones = [
            ("short", "Kurz", "Antworte kurz, direkt und professionell."),
            (
                "friendly",
                "Freundlich",
                "Antworte freundlich, warm und professionell.",
            ),
            (
                "formal",
                "Formell",
                "Antworte formell, praezise und zurueckhaltend.",
            ),
        ];
        let mut replies = Vec::with_capacity(tones.len());
        let mut model = settings.generation_model.clone();
        for (tone, label, guidance) in tones {
            let secure_prompt =
                build_quick_reply_prompt(&mail, guidance, &options).map_err(safe_error)?;
            let response = generate_secure(provider.as_ref(), secure_prompt, 650)?;
            model = response.model;
            let output = validate_quick_reply_output(&response.text).map_err(safe_error)?;
            replies.push(AiQuickReply {
                id: format!("ai-reply-{tone}"),
                label: label.into(),
                tone: tone.into(),
                text: output.reply,
            });
        }
        Ok(AiQuickRepliesResult {
            replies,
            provider: settings.provider,
            model,
        })
    })
    .await
    .map_err(|error| format!("KI-Worker wurde unerwartet beendet: {error}"))?
}

#[tauri::command]
pub async fn ai_search_messages(
    app: AppHandle,
    input: AiSearchInput,
) -> Result<AiSearchResponse, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let query = input.query.trim();
        if query.is_empty() {
            return Err("Bitte einen Suchbegriff eingeben.".into());
        }
        let limit = input.limit.unwrap_or(12).clamp(1, 50);
        let (settings, account_ids) = {
            let db = state.db.lock().map_err(|error| error.to_string())?;
            let settings = load_settings(&db)?;
            ensure_enabled(&settings)?;
            let account_ids = authorized_account_ids(&db, &settings, input.account_id.as_deref())?;
            (settings, account_ids)
        };
        let semantic = provider_for_settings(&settings).ok().and_then(|provider| {
            provider
                .embed(
                    &EmbeddingRequest {
                        input: UntrustedText::new("search_query", query),
                        purpose: EmbeddingPurpose::RetrievalQuery,
                        output_dimension: Some(768),
                        truncate: true,
                    },
                    &CancellationFlag::new(),
                )
                .ok()
                .map(|embedding| SemanticQuery {
                    model: embedding.model,
                    vector: embedding.vector,
                    weight: 0.35,
                })
        });
        let mut candidates = {
            let db = state.db.lock().map_err(|error| error.to_string())?;
            let mut candidates = search_messages(
                &db,
                &SearchRequest {
                    query: query.into(),
                    account_ids: account_ids.clone(),
                    limit: (limit * 4).min(100),
                    semantic,
                },
            )?;
            if candidates.is_empty() {
                candidates = fallback_candidates(&db, query, &account_ids, (limit * 5).min(100))?;
            }
            candidates
        };
        if candidates.is_empty() {
            return Ok(AiSearchResponse {
                query: query.into(),
                hits: Vec::new(),
                summary: None,
            });
        }

        let provider = provider_for_settings(&settings)?;
        let security_candidates = candidates
            .iter()
            .map(|item| FinderCandidate {
                id: item.message_id.clone(),
                account_id: item.account_id.clone(),
                sender: item.sender.clone(),
                subject: item.subject.clone(),
                snippet: item.snippet.clone(),
            })
            .collect::<Vec<_>>();
        let finder = build_finder_rerank_prompt(
            query,
            &security_candidates,
            &security_options(&settings, false),
        )
        .map_err(safe_error)?;
        let response = generate_secure(provider.as_ref(), finder.request.clone(), 1200)?;
        let ranked = finder.validate_output(&response.text).map_err(safe_error)?;
        let scores = ranked
            .into_iter()
            .map(|rank| (rank.candidate_id, rank.score as f32 / 100.0))
            .collect::<HashMap<_, _>>();
        candidates.sort_by(|left, right| {
            scores
                .get(&right.message_id)
                .unwrap_or(&right.score)
                .partial_cmp(scores.get(&left.message_id).unwrap_or(&left.score))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let hits = candidates
            .into_iter()
            .filter(|item| scores.contains_key(&item.message_id))
            .take(limit)
            .map(|item| {
                let score = scores.get(&item.message_id).copied().unwrap_or(item.score);
                AiSearchHit {
                    message_id: item.message_id,
                    account_id: item.account_id,
                    sender_email: extract_email(&item.sender),
                    sender: item.sender,
                    subject: item.subject,
                    date: item.date,
                    snippet: item.snippet,
                    evidence: vec![item.reason],
                    score,
                }
            })
            .collect::<Vec<_>>();
        Ok(AiSearchResponse {
            query: query.into(),
            summary: Some(format!("{} passende Nachrichten gefunden.", hits.len())),
            hits,
        })
    })
    .await
    .map_err(|error| format!("KI-Suchworker wurde unerwartet beendet: {error}"))?
}

#[tauri::command]
pub fn ai_index_status(
    state: State<'_, AppState>,
    input: AiIndexInput,
) -> Result<AiIndexStatus, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    index_status(&db, input.account_id.as_deref())
}

#[tauri::command]
pub fn ai_index_start(
    app: AppHandle,
    state: State<'_, AppState>,
    input: AiIndexInput,
) -> Result<AiIndexStatus, String> {
    let account_id = normalize_optional(input.account_id);
    let job_id = {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        let settings = load_settings(&db)?;
        ensure_enabled(&settings)?;
        if let Some(ref account_id) = account_id {
            let allowed = authorized_account_ids(&db, &settings, Some(account_id))?;
            if allowed.is_empty() {
                return Err("Dieses Konto ist nicht fuer den lokalen KI-Index freigegeben.".into());
            }
        }
        create_index_job(&db, account_id.as_deref(), DEFAULT_BATCH_SIZE)?
    };
    let app_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || loop {
        let progress = {
            let state = app_handle.state::<AppState>();
            let mut db = match state.db.lock() {
                Ok(db) => db,
                Err(_) => return,
            };
            run_index_batch(&mut db, &job_id)
        };
        match progress {
            Ok(progress) if progress.state_is_terminal() => {
                let state = app_handle.state::<AppState>();
                let _ = embed_missing_batch(&state, 12);
                return;
            }
            Ok(progress) if progress.batch_processed == 0 => return,
            Ok(_) => {
                let state = app_handle.state::<AppState>();
                let _ = embed_missing_batch(&state, 8);
                std::thread::yield_now();
            }
            Err(_) => return,
        }
    });
    let db = state.db.lock().map_err(|error| error.to_string())?;
    index_status(&db, account_id.as_deref())
}

#[tauri::command]
pub fn ai_index_cancel(
    state: State<'_, AppState>,
    input: AiIndexInput,
) -> Result<AiIndexStatus, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    if let Some(job_id) = latest_active_job(&db, input.account_id.as_deref())? {
        request_job_cancel(&db, &job_id)?;
    }
    index_status(&db, input.account_id.as_deref())
}

trait IndexProgressExt {
    fn state_is_terminal(&self) -> bool;
}

impl IndexProgressExt for IndexProgress {
    fn state_is_terminal(&self) -> bool {
        matches!(self.status.as_str(), "completed" | "cancelled" | "failed")
    }
}

fn load_settings(db: &Connection) -> Result<AiSettings, String> {
    let mut settings = db
        .query_row(
            "SELECT enabled, account_id, provider, generation_model, embedding_model, ollama_base_url
             FROM ai_app_settings WHERE id = 1",
            [],
            |row| {
                Ok(AiSettings {
                    enabled: row.get::<_, i64>(0)? != 0,
                    account_id: row.get(1)?,
                    provider: row.get(2)?,
                    generation_model: row.get(3)?,
                    embedding_model: row.get(4)?,
                    ollama_base_url: row.get(5)?,
                    gemini_key_configured: false,
                })
            },
        )
        .map_err(|error| error.to_string())?;
    if settings.provider == "gemini" {
        let normalized_model = normalize_gemini_model(&settings.generation_model);
        if normalized_model != settings.generation_model {
            db.execute(
                "UPDATE ai_app_settings
                 SET generation_model = ?1, updated_at = CURRENT_TIMESTAMP
                 WHERE id = 1",
                params![normalized_model],
            )
            .map_err(|error| error.to_string())?;
            settings.generation_model = normalized_model;
        }
    }
    settings.gemini_key_configured = read_gemini_secret().is_ok();
    Ok(settings)
}

fn validate_settings_input(input: &AiSettingsUpdateInput) -> Result<(), String> {
    if !matches!(input.provider.as_str(), "gemini" | "ollama") {
        return Err("Unbekannter KI-Anbieter.".into());
    }
    for (label, model) in [
        ("Generierungsmodell", input.generation_model.trim()),
        ("Embeddingmodell", input.embedding_model.trim()),
    ] {
        if model.is_empty()
            || model.len() > 160
            || model.bytes().any(|byte| byte.is_ascii_control())
        {
            return Err(format!("{label} ist ungueltig."));
        }
    }
    Ok(())
}

fn ensure_enabled(settings: &AiSettings) -> Result<(), String> {
    if settings.enabled {
        Ok(())
    } else {
        Err("KI ist global deaktiviert. Aktiviere sie zuerst in den Einstellungen.".into())
    }
}

fn ensure_account_exists(db: &Connection, account_id: &str) -> Result<(), String> {
    let exists = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM accounts WHERE id = ?1)",
            params![account_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| error.to_string())?;
    if exists == 0 {
        Err("Das ausgewaehlte E-Mail-Konto existiert nicht.".into())
    } else {
        Ok(())
    }
}

fn provider_for_settings(settings: &AiSettings) -> Result<Box<dyn AiProvider>, String> {
    ensure_enabled(settings)?;
    let config = match settings.provider.as_str() {
        "gemini" => ProviderConfig::Gemini(GeminiConfig::new(
            read_gemini_secret()?,
            settings.generation_model.clone(),
            settings.embedding_model.clone(),
        )),
        "ollama" => ProviderConfig::Ollama(
            OllamaConfig::new(
                settings.generation_model.clone(),
                settings.embedding_model.clone(),
            )
            .with_base_url(
                settings
                    .ollama_base_url
                    .clone()
                    .unwrap_or_else(|| "http://127.0.0.1:11434".into()),
            ),
        ),
        _ => return Err("Unbekannter KI-Anbieter.".into()),
    };
    create_provider(config).map_err(safe_error)
}

fn embed_missing_batch(state: &State<'_, AppState>, limit: usize) -> Result<usize, String> {
    let (settings, messages) = {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        let settings = load_settings(&db)?;
        if !settings.enabled {
            return Ok(0);
        }
        let mut statement = db
            .prepare(
                "SELECT m.id, m.subject, m.from_addr, m.body_text
                 FROM messages m
                 JOIN ai_message_state state ON state.message_id = m.id
                 JOIN ai_account_settings account_settings ON account_settings.account_id = m.account_id
                 WHERE state.status = 'ready' AND account_settings.semantic_enabled = 1
                   AND NOT EXISTS (
                     SELECT 1 FROM ai_embeddings embedding
                     WHERE embedding.message_id = m.id AND embedding.model = ?1
                   )
                 ORDER BY COALESCE(m.date, m.created_at) DESC LIMIT ?2",
            )
            .map_err(|error| error.to_string())?;
        let messages = statement
            .query_map(
                params![settings.embedding_model, limit.clamp(1, 24)],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        (settings, messages)
    };
    if messages.is_empty() {
        return Ok(0);
    }
    let provider = provider_for_settings(&settings)?;
    let mut stored = 0;
    for (message_id, subject, sender, body) in messages {
        let source = format!("Betreff: {subject}\nAbsender: {sender}\n\n{body}");
        let Some(chunk) = chunk_text(&source, 1_600, 0).into_iter().next() else {
            continue;
        };
        let embedding = match provider.embed(
            &EmbeddingRequest {
                input: UntrustedText::new("mail_search_document", chunk.clone()),
                purpose: EmbeddingPurpose::RetrievalDocument,
                output_dimension: Some(768),
                truncate: true,
            },
            &CancellationFlag::new(),
        ) {
            Ok(embedding) => embedding,
            Err(_) => break,
        };
        let db = state.db.lock().map_err(|error| error.to_string())?;
        store_embedding(
            &db,
            &message_id,
            &embedding.model,
            0,
            &hash_message_content("", "", "", &chunk),
            &embedding.vector,
        )?;
        stored += 1;
    }
    Ok(stored)
}

fn read_gemini_secret() -> Result<String, String> {
    for service in KEYRING_SERVICES {
        if let Ok(entry) = keyring::Entry::new(service, GEMINI_SECRET_USER) {
            if let Ok(secret) = entry.get_password() {
                if !secret.trim().is_empty() {
                    return Ok(secret);
                }
            }
        }
    }
    Err("Kein Gemini API-Key im Credential Manager gespeichert.".into())
}

fn generate_secure(
    provider: &dyn AiProvider,
    prompt: PromptRequest,
    max_output_tokens: u32,
) -> Result<super::models::GenerationResponse, String> {
    if prompt.allow_tools {
        return Err("KI-Tools sind fuer Mailinhalte nicht erlaubt.".into());
    }
    provider
        .generate(
            &GenerationRequest {
                instruction: prompt.system_prompt,
                prompt: "Verarbeite ausschliesslich den beigefuegten, untrusted Datenblock und liefere das geforderte JSON.".into(),
                untrusted_data: vec![UntrustedText::new("secured_mail_context", prompt.user_prompt)],
                options: GenerationOptions {
                    temperature: Some(0.2),
                    max_output_tokens: Some(max_output_tokens),
                },
            },
            &CancellationFlag::new(),
        )
        .map_err(safe_error)
}

fn load_action_context(
    state: &State<'_, AppState>,
    input: &AiMessageActionInput,
) -> Result<(AiSettings, MailPromptInput), String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let settings = load_settings(&db)?;
    ensure_enabled(&settings)?;
    authorize_account(&settings, &input.account_id)?;
    let mut mail = db
        .query_row(
            "SELECT account_id, id, from_addr, to_addrs_json, subject, body_text
             FROM messages WHERE id = ?1 AND account_id = ?2",
            params![input.message_id, input.account_id],
            |row| {
                let recipients_json = row.get::<_, String>(3)?;
                Ok(MailPromptInput {
                    account_id: row.get(0)?,
                    message_id: row.get(1)?,
                    sender: row.get(2)?,
                    recipients: serde_json::from_str(&recipients_json).unwrap_or_default(),
                    subject: row.get(4)?,
                    body: row.get(5)?,
                    attachments: Vec::new(),
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Nachricht wurde im ausgewaehlten Konto nicht gefunden.".to_string())?;
    if input.include_attachments {
        let mut statement = db
            .prepare(
                "SELECT COALESCE(filename, 'Anhang'), COALESCE(mime_type, 'application/octet-stream'), storage_ref
                 FROM attachments WHERE message_id = ?1 ORDER BY created_at LIMIT 20",
            )
            .map_err(|error| error.to_string())?;
        mail.attachments = statement
            .query_map(params![input.message_id], |row| {
                let filename = row.get::<_, String>(0)?;
                let mime_type = row.get::<_, String>(1)?;
                let storage_ref = row.get::<_, Option<String>>(2)?;
                let extracted_text = storage_ref
                    .as_deref()
                    .and_then(|path| {
                        read_safe_text_attachment(&state.attachment_dir, path, &mime_type)
                    })
                    .unwrap_or_default();
                Ok(PromptAttachment {
                    filename,
                    mime_type,
                    extracted_text,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
    }
    Ok((settings, mail))
}

fn read_safe_text_attachment(
    root: &std::path::Path,
    stored: &str,
    mime_type: &str,
) -> Option<String> {
    if !mime_type.starts_with("text/") {
        return None;
    }
    let candidate = std::path::PathBuf::from(stored);
    let path = if candidate.is_absolute() {
        candidate
    } else {
        root.join(candidate)
    };
    let root = fs::canonicalize(root).ok()?;
    let path = fs::canonicalize(path).ok()?;
    if !path.starts_with(root) {
        return None;
    }
    let metadata = fs::metadata(&path).ok()?;
    if metadata.len() > 256 * 1024 {
        return None;
    }
    fs::read_to_string(path).ok()
}

fn authorize_account(settings: &AiSettings, account_id: &str) -> Result<(), String> {
    if settings
        .account_id
        .as_deref()
        .map(|selected| selected != account_id)
        .unwrap_or(false)
    {
        Err("Dieses Konto ist fuer KI-Funktionen nicht freigegeben.".into())
    } else {
        Ok(())
    }
}

fn authorized_account_ids(
    db: &Connection,
    settings: &AiSettings,
    requested: Option<&str>,
) -> Result<Vec<String>, String> {
    let requested = requested.map(str::trim).filter(|value| !value.is_empty());
    if let Some(requested) = requested {
        authorize_account(settings, requested)?;
        ensure_account_exists(db, requested)?;
        return Ok(vec![requested.to_string()]);
    }
    if let Some(ref selected) = settings.account_id {
        return Ok(vec![selected.clone()]);
    }
    db.prepare("SELECT id FROM accounts ORDER BY created_at")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(|error| error.to_string())
}

fn security_options(settings: &AiSettings, include_attachments: bool) -> PromptSecurityOptions {
    PromptSecurityOptions {
        target: if settings.provider == "gemini" {
            InferenceTarget::Cloud
        } else {
            InferenceTarget::Local
        },
        minimize_pii: settings.provider == "gemini",
        include_attachments,
        max_mail_chars: 24_000,
        max_attachment_chars: if include_attachments { 8_000 } else { 0 },
    }
}

fn fallback_candidates(
    db: &Connection,
    query: &str,
    account_ids: &[String],
    limit: usize,
) -> Result<Vec<MessageSearchResult>, String> {
    let tokens = query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| token.chars().count() >= 3)
        .take(8)
        .map(|token| format!("%{}%", token.to_lowercase()))
        .collect::<Vec<_>>();
    if tokens.is_empty() || account_ids.is_empty() {
        return Ok(Vec::new());
    }
    let account_placeholders = (0..account_ids.len())
        .map(|index| format!("?{}", index + 1))
        .collect::<Vec<_>>();
    let token_offset = account_ids.len();
    let token_clauses = tokens
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let parameter = token_offset + index + 1;
            format!("lower(subject || ' ' || from_addr || ' ' || body_text) LIKE ?{parameter}")
        })
        .collect::<Vec<_>>();
    let limit_parameter = account_ids.len() + tokens.len() + 1;
    let sql = format!(
        "SELECT id, account_id, COALESCE(date, created_at), from_addr, subject,
                substr(body_text, 1, 320)
         FROM messages WHERE account_id IN ({}) AND ({})
         ORDER BY COALESCE(date, created_at) DESC LIMIT ?{}",
        account_placeholders.join(","),
        token_clauses.join(" OR "),
        limit_parameter
    );
    let mut values = account_ids.to_vec();
    values.extend(tokens);
    values.push(limit.to_string());
    let mut statement = db.prepare(&sql).map_err(|error| error.to_string())?;
    let results = statement
        .query_map(rusqlite::params_from_iter(values), |row| {
            Ok(MessageSearchResult {
                message_id: row.get(0)?,
                account_id: row.get(1)?,
                date: row.get(2)?,
                sender: row.get(3)?,
                subject: row.get(4)?,
                snippet: row.get(5)?,
                reason: "Lokaler Texttreffer, von der KI nach Relevanz geordnet".into(),
                metadata_score: 0.4,
                fts_score: 0.4,
                semantic_score: None,
                score: 0.4,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(results)
}

fn index_status(db: &Connection, account_id: Option<&str>) -> Result<AiIndexStatus, String> {
    let job = db
        .query_row(
            "SELECT status, processed_count, total_count, last_error, updated_at, account_id
             FROM ai_jobs
             WHERE ((?1 IS NULL AND account_id IS NULL) OR account_id = ?1)
             ORDER BY created_at DESC LIMIT 1",
            params![account_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)? as usize,
                    row.get::<_, i64>(2)? as usize,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((raw_state, processed, total, last_error, updated_at, scoped_account)) = job else {
        return Ok(AiIndexStatus {
            state: "not_started".into(),
            account_id: account_id.map(str::to_string),
            processed: 0,
            total: 0,
            progress: 0.0,
            message: "Noch kein lokaler Nachrichtenindex vorhanden.".into(),
            last_error: None,
            updated_at: None,
        });
    };
    let state = match raw_state.as_str() {
        "queued" | "running" => "running",
        "completed" => "complete",
        "cancelled" => "cancelled",
        "failed" => "error",
        _ => "idle",
    };
    let progress = if total == 0 {
        0.0
    } else {
        processed as f32 / total as f32 * 100.0
    };
    Ok(AiIndexStatus {
        state: state.into(),
        account_id: scoped_account,
        processed,
        total,
        progress: progress.clamp(0.0, 100.0),
        message: match state {
            "running" => format!("{processed} von {total} Nachrichten indexiert."),
            "complete" => format!("{processed} Nachrichten lokal indexiert."),
            "cancelled" => "Indexierung wurde abgebrochen.".into(),
            "error" => "Indexierung ist fehlgeschlagen.".into(),
            _ => "Lokaler Index ist bereit.".into(),
        },
        last_error,
        updated_at,
    })
}

fn latest_active_job(db: &Connection, account_id: Option<&str>) -> Result<Option<String>, String> {
    db.query_row(
        "SELECT id FROM ai_jobs WHERE status IN ('queued', 'running')
         AND ((?1 IS NULL AND account_id IS NULL) OR account_id = ?1)
         ORDER BY created_at DESC LIMIT 1",
        params![account_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|error| error.to_string())
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && value != "all")
}

fn extract_email(sender: &str) -> String {
    sender
        .split('<')
        .nth(1)
        .and_then(|value| value.split('>').next())
        .unwrap_or(sender)
        .trim()
        .to_string()
}

fn now_label() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

fn safe_error(error: impl std::fmt::Display) -> String {
    redact_inline_secrets(&error.to_string())
}
