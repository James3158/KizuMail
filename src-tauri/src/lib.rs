use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::{Message, SmtpTransport, Transport};
use mailparse::{dateparse, parse_mail, MailHeaderMap, ParsedMail};
use rusqlite::{params, params_from_iter, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_notification::NotificationExt;

mod ai;

const INITIAL_MIGRATION: &str = include_str!("../migrations/0001_initial.sql");
const KEYRING_SERVICE: &str = "KizuMail";
const KEYRING_SERVICES: &[&str] = &["KizuMail", "app.kizumail.desktop", "kizumail"];
const KEYRING_CHUNK_MANIFEST_PREFIX: &str = "kizumail-chunked-v1:";
const KEYRING_WINDOWS_BLOB_BYTE_LIMIT: usize = 2560;
const KEYRING_CHUNK_UTF16_LIMIT: usize = 1000;
const KEYRING_CHUNK_MAX_COUNT: usize = 64;
const IMAP_NETWORK_TIMEOUT: Duration = Duration::from_secs(20);
const UNSUBSCRIBE_HTTP_TIMEOUT: Duration = Duration::from_secs(15);
const BACKGROUND_IDLE_TIMEOUT: Duration = Duration::from_secs(55);
const BACKGROUND_POLL_INTERVAL: Duration = Duration::from_secs(60);
const BACKGROUND_FORCE_SYNC_INTERVAL: Duration = Duration::from_secs(5 * 60);
const BACKGROUND_BACKFILL_INTERVAL: Duration = Duration::from_secs(10 * 60);
static OAUTH_RESOURCE_DIR: OnceLock<PathBuf> = OnceLock::new();

struct AppState {
    db: Mutex<Connection>,
    db_path: PathBuf,
    password_cache: Mutex<HashMap<String, String>>,
    account_sync_guards: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    attachment_dir: PathBuf,
}

#[derive(Clone, Serialize)]
struct MessagesChangedEvent {
    #[serde(rename = "accountId")]
    account_id: String,
    #[serde(rename = "messageCount")]
    message_count: usize,
}

#[derive(Clone, Deserialize, Serialize)]
struct DesktopPreferences {
    #[serde(rename = "autoSync")]
    auto_sync: bool,
    notifications: bool,
    #[serde(rename = "privacyMode")]
    privacy_mode: bool,
    #[serde(rename = "launchAtLogin")]
    launch_at_login: bool,
    #[serde(rename = "minimizeToTray")]
    minimize_to_tray: bool,
    initialized: bool,
}

impl Default for DesktopPreferences {
    fn default() -> Self {
        Self {
            auto_sync: true,
            notifications: false,
            privacy_mode: true,
            launch_at_login: false,
            minimize_to_tray: true,
            initialized: false,
        }
    }
}

#[derive(Deserialize)]
struct DesktopPreferencesInput {
    #[serde(rename = "autoSync")]
    auto_sync: bool,
    notifications: bool,
    #[serde(rename = "privacyMode")]
    privacy_mode: bool,
    #[serde(rename = "launchAtLogin")]
    launch_at_login: bool,
    #[serde(rename = "minimizeToTray")]
    minimize_to_tray: bool,
}

#[derive(Serialize)]
struct Account {
    id: String,
    provider: String,
    email: String,
    #[serde(rename = "displayName")]
    display_name: String,
    #[serde(rename = "authType")]
    auth_type: String,
    #[serde(rename = "syncEnabled")]
    sync_enabled: bool,
    #[serde(rename = "lastSyncAt")]
    last_sync_at: Option<String>,
    #[serde(rename = "credentialStatus")]
    credential_status: String,
    #[serde(rename = "authStatus")]
    auth_status: String,
}

#[derive(Deserialize)]
struct CreateImapAccountInput {
    #[serde(rename = "displayName")]
    display_name: String,
    email: String,
    password: String,
    #[serde(rename = "imapHost")]
    imap_host: String,
    #[serde(rename = "imapPort")]
    imap_port: u16,
    #[serde(rename = "smtpHost")]
    smtp_host: String,
    #[serde(rename = "smtpPort")]
    smtp_port: u16,
}

#[derive(Deserialize)]
struct UpdateAccountPasswordInput {
    #[serde(rename = "accountId")]
    account_id: String,
    password: String,
}

#[derive(Deserialize)]
struct NewsletterActionInput {
    #[serde(rename = "subscriptionId")]
    subscription_id: String,
}

#[derive(Deserialize)]
struct SenderActionInput {
    #[serde(rename = "messageId")]
    message_id: String,
}

#[derive(Deserialize)]
struct MoveMessageInput {
    #[serde(rename = "messageId")]
    message_id: String,
    role: String,
}

#[derive(Deserialize)]
struct MarkMessageReadInput {
    #[serde(rename = "messageId")]
    message_id: String,
    unread: bool,
}

#[derive(Deserialize)]
struct MarkMessageFlaggedInput {
    #[serde(rename = "messageId")]
    message_id: String,
    flagged: bool,
}

#[derive(Deserialize)]
struct SendEmailInput {
    #[serde(rename = "accountId")]
    account_id: String,
    to: String,
    subject: String,
    body: String,
}

#[derive(Deserialize)]
struct OAuthStartInput {
    provider: String,
    email: Option<String>,
    #[serde(rename = "clientId")]
    client_id: Option<String>,
    #[serde(rename = "redirectUri")]
    redirect_uri: Option<String>,
}

#[derive(Deserialize)]
struct OAuthCompleteInput {
    provider: String,
    email: String,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    #[serde(rename = "clientId")]
    client_id: String,
    code: String,
    #[serde(rename = "codeVerifier")]
    code_verifier: String,
    #[serde(rename = "redirectUri")]
    redirect_uri: String,
}

#[derive(Deserialize)]
struct OAuthConnectInput {
    provider: String,
    email: String,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    #[serde(rename = "clientId")]
    client_id: Option<String>,
    #[serde(rename = "redirectUri")]
    redirect_uri: Option<String>,
}

#[derive(Deserialize)]
struct AccountIdInput {
    #[serde(rename = "accountId")]
    account_id: String,
}

#[derive(Serialize)]
struct ImapConnectionTestResult {
    ok: bool,
    summary: String,
    details: String,
}

#[derive(Serialize)]
struct SyncResult {
    ok: bool,
    #[serde(rename = "accountId")]
    account_id: String,
    #[serde(rename = "folderCount")]
    folder_count: usize,
    #[serde(rename = "messageCount")]
    message_count: usize,
    #[serde(rename = "newMessages")]
    new_messages: Vec<NewMailNotification>,
    summary: String,
    details: String,
}

#[derive(Clone, Serialize)]
struct NewMailNotification {
    #[serde(rename = "messageId")]
    message_id: String,
    #[serde(rename = "accountId")]
    account_id: String,
    sender: String,
    subject: String,
}

#[derive(Deserialize)]
struct MessageListInput {
    limit: Option<u32>,
    offset: Option<u32>,
    sort: Option<String>,
    #[serde(rename = "accountId")]
    account_id: Option<String>,
    folder: Option<String>,
    #[serde(rename = "folderRole")]
    folder_role: Option<String>,
    unread: Option<bool>,
    flagged: Option<bool>,
    #[serde(rename = "hasAttachments")]
    has_attachments: Option<bool>,
    #[serde(rename = "dateFrom")]
    date_from: Option<String>,
    #[serde(rename = "dateTo")]
    date_to: Option<String>,
}

#[derive(Deserialize)]
struct SyncBackfillInput {
    #[serde(rename = "accountId")]
    account_id: Option<String>,
    #[serde(rename = "folderId")]
    folder_id: Option<String>,
    #[serde(rename = "batchSize")]
    batch_size: Option<u32>,
}

#[derive(Serialize)]
struct MessageListResult {
    messages: Vec<MessageSummary>,
    #[serde(rename = "nextCursor")]
    next_cursor: Option<String>,
    #[serde(rename = "hasMore")]
    has_more: bool,
    #[serde(rename = "syncStatus")]
    sync_status: SyncOverview,
}

#[derive(Serialize, Default)]
struct SyncOverview {
    running: bool,
    summary: String,
    #[serde(rename = "backfillAvailable")]
    backfill_available: bool,
}

#[derive(Serialize)]
struct SyncStatus {
    #[serde(rename = "accountId")]
    account_id: String,
    #[serde(rename = "accountEmail")]
    account_email: String,
    #[serde(rename = "folderId")]
    folder_id: String,
    folder: String,
    role: Option<String>,
    #[serde(rename = "lastSyncedAt")]
    last_synced_at: Option<String>,
    #[serde(rename = "lastError")]
    last_error: Option<String>,
    #[serde(rename = "backfillComplete")]
    backfill_complete: bool,
    #[serde(rename = "highestUid")]
    highest_uid: Option<u32>,
    #[serde(rename = "lowestUid")]
    lowest_uid: Option<u32>,
}

const SYNC_BACKFILL_VERSION: u8 = 2;

#[derive(Serialize, Deserialize, Clone)]
struct SyncCursor {
    #[serde(rename = "uidValidity")]
    uid_validity: Option<String>,
    #[serde(rename = "highestUid")]
    highest_uid: Option<u32>,
    #[serde(rename = "lowestUid")]
    lowest_uid: Option<u32>,
    #[serde(rename = "backfillComplete")]
    backfill_complete: bool,
    #[serde(rename = "backfillVersion", default)]
    backfill_version: u8,
}

impl Default for SyncCursor {
    fn default() -> Self {
        Self {
            uid_validity: None,
            highest_uid: None,
            lowest_uid: None,
            backfill_complete: false,
            backfill_version: SYNC_BACKFILL_VERSION,
        }
    }
}

#[derive(Serialize)]
struct MessageActionResult {
    ok: bool,
    summary: String,
}

#[derive(Serialize)]
struct NewsletterActionResult {
    ok: bool,
    method: String,
    target: String,
    summary: String,
}

#[derive(Serialize)]
struct OAuthStartResult {
    ok: bool,
    provider: String,
    #[serde(rename = "authorizationUrl")]
    authorization_url: String,
    state: String,
    #[serde(rename = "codeVerifier")]
    code_verifier: String,
    #[serde(rename = "redirectUri")]
    redirect_uri: String,
    summary: String,
}

#[derive(Serialize)]
struct OAuthCompleteResult {
    ok: bool,
    account: Account,
    summary: String,
}

#[derive(Serialize)]
struct OAuthProviderStatus {
    provider: String,
    label: String,
    #[serde(rename = "clientIdConfigured")]
    client_id_configured: bool,
    summary: String,
    #[serde(rename = "requiresOwnRegistration")]
    requires_own_registration: bool,
    #[serde(rename = "endUserReady")]
    end_user_ready: bool,
}

#[derive(Serialize)]
struct CredentialRepairResult {
    ok: bool,
    repaired: u32,
    missing: u32,
    summary: String,
}

#[derive(Serialize)]
struct SecurityAuditResult {
    ok: bool,
    summary: String,
    checks: Vec<SecurityCheck>,
}

#[derive(Serialize)]
struct SecurityCheck {
    label: String,
    ok: bool,
    details: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct StoredOAuthToken {
    provider: String,
    #[serde(rename = "clientId")]
    client_id: String,
    #[serde(rename = "tokenEndpoint")]
    token_endpoint: String,
    #[serde(rename = "accessToken")]
    access_token: String,
    #[serde(rename = "refreshToken")]
    refresh_token: Option<String>,
    #[serde(rename = "expiresAt")]
    expires_at: i64,
}

struct AccountConnection {
    id: String,
    provider: String,
    email: String,
    auth_type: String,
    secret_ref: Option<String>,
    imap_host: Option<String>,
    imap_port: Option<u16>,
    smtp_host: Option<String>,
    smtp_port: Option<u16>,
}

struct ProviderConfig {
    provider: &'static str,
    imap_host: &'static str,
    imap_port: u16,
    smtp_host: &'static str,
    smtp_port: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DiscoveredFolder {
    name: String,
    role: Option<String>,
    role_source: Option<String>,
    selectable: bool,
}

struct ParsedMessage {
    uid: String,
    subject: String,
    from_name: String,
    from_email: String,
    to_json: String,
    date: String,
    flags_json: String,
    body_text: String,
    body_html: String,
    raw_headers_json: String,
    attachments: Vec<ParsedAttachment>,
    has_attachments: bool,
    has_newsletter_signals: bool,
    unsubscribe_target: Option<String>,
    unsubscribe_one_click: bool,
}

struct ParsedAttachment {
    filename: String,
    mime_type: String,
    size: usize,
    content_id: String,
    is_inline: bool,
    bytes: Vec<u8>,
}

struct MessageContext {
    account_id: String,
    folder_path: String,
    uid: String,
    sender_email: String,
}

struct OAuth2Authenticator {
    user: String,
    access_token: String,
}

impl imap::Authenticator for OAuth2Authenticator {
    type Response = String;

    fn process(&self, _data: &[u8]) -> Self::Response {
        format!(
            "user={}\x01auth=Bearer {}\x01\x01",
            self.user, self.access_token
        )
    }
}

#[derive(Serialize)]
struct MessageSummary {
    id: String,
    #[serde(rename = "accountId")]
    account_id: String,
    sender: String,
    #[serde(rename = "senderEmail")]
    sender_email: String,
    subject: String,
    preview: String,
    #[serde(rename = "bodyText")]
    body_text: String,
    #[serde(rename = "bodyHtml")]
    body_html: Option<String>,
    date: String,
    folder: String,
    #[serde(rename = "folderRole")]
    folder_role: Option<String>,
    unread: bool,
    flagged: bool,
    #[serde(rename = "hasAttachments")]
    has_attachments: bool,
    attachments: Vec<AttachmentSummary>,
    #[serde(rename = "hasNewsletterSignals")]
    has_newsletter_signals: bool,
    #[serde(rename = "renderModeAvailable")]
    render_mode_available: RenderModeAvailability,
    #[serde(rename = "renderWarnings")]
    render_warnings: Vec<String>,
    #[serde(skip_serializing)]
    dedupe_key: String,
}

#[derive(Serialize)]
struct RenderModeAvailability {
    readable: bool,
    original: bool,
}

#[derive(Serialize)]
struct AttachmentSummary {
    filename: String,
    #[serde(rename = "mimeType")]
    mime_type: String,
    size: i64,
    #[serde(rename = "contentId")]
    content_id: Option<String>,
    #[serde(rename = "storageRef")]
    storage_ref: Option<String>,
    inline: bool,
    disposition: String,
    #[serde(rename = "previewKind")]
    preview_kind: String,
    #[serde(rename = "downloadPath")]
    download_path: Option<String>,
}

#[derive(Serialize)]
struct NewsletterSubscription {
    id: String,
    sender: String,
    #[serde(rename = "senderEmail")]
    sender_email: String,
    domain: String,
    #[serde(rename = "messageCount")]
    message_count: u32,
    #[serde(rename = "lastMessageAt")]
    last_message_at: String,
    #[serde(rename = "unsubscribeMethod")]
    unsubscribe_method: String,
    #[serde(rename = "confidenceScore")]
    confidence_score: u8,
    #[serde(rename = "riskScore")]
    risk_score: u8,
    #[serde(rename = "riskLevel")]
    risk_level: String,
    evidence: Vec<String>,
}

#[derive(Serialize)]
struct SenderStats {
    sender: String,
    #[serde(rename = "senderEmail")]
    sender_email: String,
    domain: String,
    #[serde(rename = "firstMessageAt")]
    first_message_at: Option<String>,
    #[serde(rename = "lastMessageAt")]
    last_message_at: Option<String>,
    #[serde(rename = "messageCount")]
    message_count: u32,
    newsletter: bool,
    trusted: bool,
    blocked: bool,
}

#[derive(Serialize)]
struct AuditEvent {
    id: String,
    action: String,
    entity: String,
    summary: String,
    #[serde(rename = "createdAt")]
    created_at: String,
}

#[tauri::command]
fn accounts_list(state: State<'_, AppState>) -> Result<Vec<Account>, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let mut statement = db
        .prepare(
            "SELECT id, provider, email, display_name, auth_type, sync_enabled, last_sync_at, secret_ref
             FROM accounts
             WHERE email NOT LIKE '%@example.com'
             ORDER BY created_at ASC",
        )
        .map_err(|error| error.to_string())?;

    let accounts = statement
        .query_map([], |row| {
            let id = row.get::<_, String>(0)?;
            let provider = row.get::<_, String>(1)?;
            let email = row.get::<_, String>(2)?;
            let auth_type = row.get::<_, String>(4)?;
            let secret_ref = row.get::<_, Option<String>>(7).unwrap_or(None);
            let connection = AccountConnection {
                id: id.clone(),
                provider: provider.clone(),
                email: email.clone(),
                auth_type: auth_type.clone(),
                secret_ref,
                imap_host: None,
                imap_port: None,
                smtp_host: None,
                smtp_port: None,
            };
            let credential_status = credential_status_for_account(&state, &connection);
            Ok(Account {
                id,
                provider,
                email,
                display_name: row.get(3)?,
                auth_type: auth_type.clone(),
                sync_enabled: row.get::<_, i64>(5)? == 1,
                last_sync_at: row.get(6)?,
                auth_status: auth_status_from_credential(&auth_type, &credential_status),
                credential_status,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    Ok(accounts)
}

#[tauri::command]
fn accounts_create_imap(
    state: State<'_, AppState>,
    input: CreateImapAccountInput,
) -> Result<Account, String> {
    let email = input.email.trim().to_lowercase();
    let display_name = if input.display_name.trim().is_empty() {
        email.split('@').next().unwrap_or("Konto").to_string()
    } else {
        input.display_name.trim().to_string()
    };
    let password = input.password.as_str();
    let provider_config = provider_config_for_email(&email);
    let imap_host = if input.imap_host.trim().is_empty() {
        provider_config
            .as_ref()
            .map(|config| config.imap_host)
            .unwrap_or("")
    } else {
        input.imap_host.trim()
    };
    let smtp_host = if input.smtp_host.trim().is_empty() {
        provider_config
            .as_ref()
            .map(|config| config.smtp_host)
            .unwrap_or("")
    } else {
        input.smtp_host.trim()
    };
    let imap_port = if input.imap_port == 0 {
        provider_config
            .as_ref()
            .map(|config| config.imap_port)
            .unwrap_or(993)
    } else {
        input.imap_port
    };
    let smtp_port = if input.smtp_port == 0 {
        provider_config
            .as_ref()
            .map(|config| config.smtp_port)
            .unwrap_or(587)
    } else {
        input.smtp_port
    };
    let provider = provider_config
        .as_ref()
        .map(|config| config.provider)
        .unwrap_or("imap");

    if !email.contains('@') {
        return Err("Bitte eine gültige E-Mail-Adresse eingeben.".into());
    }

    if imap_host.is_empty() || smtp_host.is_empty() {
        return Err("Der Anbieter wurde nicht automatisch erkannt. Bitte die erweiterten Serverdaten ausfüllen.".into());
    }

    if password.is_empty() {
        return Err("Passwort oder App-Passwort ist erforderlich.".into());
    }

    if imap_port == 0 || smtp_port == 0 {
        return Err("IMAP- und SMTP-Port müssen größer als 0 sein.".into());
    }

    let id = format!("acct-{}", uuid::Uuid::new_v4());
    let secret_user = format!("{id}:imap");
    let secret_ref = format!("keyring://{KEYRING_SERVICE}/{secret_user}");
    let metadata = serde_json::json!({
        "imapHost": imap_host,
        "imapPort": imap_port,
        "smtpHost": smtp_host,
        "smtpPort": smtp_port,
        "summary": "IMAP account created. Password stored in OS credential storage."
    })
    .to_string();

    store_password_entry(&secret_user, password)?;
    let _ = store_password_alias(&email, password);
    cache_password(&state, &id, password)?;
    cache_password(&state, &email, password)?;

    let db = state.db.lock().map_err(|error| error.to_string())?;
    if let Err(error) = db.execute(
        "INSERT INTO accounts (id, provider, email, display_name, auth_type, secret_ref, sync_enabled, imap_host, imap_port, smtp_host, smtp_port)
         VALUES (?1, ?2, ?3, ?4, 'password', ?5, 1, ?6, ?7, ?8, ?9)",
        params![id, provider, email, display_name, secret_ref, imap_host, imap_port, smtp_host, smtp_port],
    ) {
        let _ = delete_password_entry(&secret_user);
        return Err(if error.to_string().contains("UNIQUE") {
            "Dieses Konto existiert bereits.".to_string()
        } else {
            error.to_string()
        });
    }

    db.execute(
        "INSERT INTO audit_log (id, action, entity_type, entity_id, metadata_json)
         VALUES (?1, 'account.create', 'account', ?2, ?3)",
        params![format!("audit-{}", uuid::Uuid::new_v4()), id, metadata],
    )
    .map_err(|error| {
        let _ = delete_password_entry(&secret_user);
        if error.to_string().contains("UNIQUE") {
            "Dieses Konto existiert bereits.".to_string()
        } else {
            error.to_string()
        }
    })?;

    Ok(Account {
        id,
        provider: provider.into(),
        email,
        display_name: display_name.into(),
        auth_type: "password".into(),
        sync_enabled: true,
        last_sync_at: None,
        credential_status: "ok".into(),
        auth_status: "connected".into(),
    })
}

#[tauri::command]
fn accounts_update_password(
    state: State<'_, AppState>,
    input: UpdateAccountPasswordInput,
) -> Result<Account, String> {
    let account = load_account_connection(&state, &input.account_id)?;
    if account.auth_type == "oauth" {
        return Err("Dieses Konto nutzt OAuth. Bitte über Einstellungen > OAuth neu verbinden statt ein Passwort zu speichern.".into());
    }
    if input.password.trim().is_empty() {
        return Err("Bitte ein Passwort oder App-Passwort eingeben.".into());
    }

    cache_password(&state, &account.id, &input.password)?;
    cache_password(&state, &account.email, &input.password)?;
    let keyring_status = match store_password_for_account(&account, &input.password) {
        Ok(()) => "Password refreshed in OS credential storage.",
        Err(_) => {
            "Password refreshed for this app session. Windows Credential Manager write failed."
        }
    };

    let db = state.db.lock().map_err(|error| error.to_string())?;
    db.execute(
        "UPDATE accounts
         SET updated_at = CURRENT_TIMESTAMP
         WHERE id = ?1",
        params![account.id],
    )
    .map_err(|error| error.to_string())?;

    db.execute(
        "INSERT INTO audit_log (id, action, entity_type, entity_id, metadata_json)
         VALUES (?1, 'account.password.update', 'account', ?2, ?3)",
        params![
            format!("audit-{}", uuid::Uuid::new_v4()),
            account.id,
            serde_json::json!({
                "summary": keyring_status
            })
            .to_string()
        ],
    )
    .map_err(|error| error.to_string())?;

    load_public_account(&db, &input.account_id)
}

#[tauri::command]
fn accounts_repair_credentials(
    state: State<'_, AppState>,
) -> Result<CredentialRepairResult, String> {
    let accounts = load_account_connections(&state)?;
    let mut repaired = 0_u32;
    let mut missing = 0_u32;
    for account in accounts {
        match load_password(&state, &account) {
            Ok(secret) => {
                if store_password_for_account(&account, &secret).is_ok() {
                    repaired += 1;
                }
            }
            Err(_) => missing += 1,
        }
    }
    Ok(CredentialRepairResult {
        ok: missing == 0,
        repaired,
        missing,
        summary: if missing == 0 {
            format!("{repaired} Konto/Konten geprüft und Credential-Aliasse repariert.")
        } else {
            format!("{repaired} Konto/Konten repariert, {missing} brauchen erneute Anmeldung.")
        },
    })
}

#[tauri::command]
fn accounts_security_audit(state: State<'_, AppState>) -> Result<SecurityAuditResult, String> {
    let accounts = load_account_connections(&state)?;
    let mut checks = Vec::new();
    let missing_credentials = accounts
        .iter()
        .filter(|account| credential_status_for_account(&state, account) == "missing")
        .count();
    checks.push(SecurityCheck {
        label: "Windows Credential Manager".into(),
        ok: missing_credentials == 0,
        details: if missing_credentials == 0 {
            "Alle lokalen Konten haben einen wiederauffindbaren Credential-Eintrag.".into()
        } else {
            format!("{missing_credentials} Konto/Konten brauchen eine erneute sichere Anmeldung.")
        },
    });
    checks.push(SecurityCheck {
        label: "OAuth Tokens".into(),
        ok: true,
        details: "OAuth-Tokens werden als Credential gespeichert; Access-Tokens werden bei Bedarf erneuert.".into(),
    });
    checks.push(SecurityCheck {
        label: "Microsoft Login".into(),
        ok: configured_oauth_client_id("microsoft").is_some(),
        details: if configured_oauth_client_id("microsoft").is_some() {
            "Microsoft OAuth ist für diese Build-Version konfiguriert.".into()
        } else {
            "Ohne Microsoft-App-Registrierung ist Hotmail/Outlook-OAuth vorbereitet, aber nicht als Endnutzer-Login aktiv.".into()
        },
    });
    checks.push(SecurityCheck {
        label: "Lokale Daten".into(),
        ok: true,
        details: "Mails und Anhänge bleiben lokal in der Tauri-App-Datenablage; die App nutzt keinen externen KizuMail-Server.".into(),
    });
    let ok = checks.iter().all(|check| check.ok);
    Ok(SecurityAuditResult {
        ok,
        summary: if ok {
            "Sicherheitsprüfung ohne kritische Hinweise abgeschlossen.".into()
        } else {
            "Sicherheitsprüfung abgeschlossen. Einige Punkte brauchen Aufmerksamkeit.".into()
        },
        checks,
    })
}

#[tauri::command]
fn accounts_remove(
    state: State<'_, AppState>,
    input: AccountIdInput,
) -> Result<MessageActionResult, String> {
    let account = load_account_connection(&state, &input.account_id)?;
    delete_password_entries(&account);
    {
        let mut cache = state
            .password_cache
            .lock()
            .map_err(|error| error.to_string())?;
        cache.remove(&account.id.trim().to_lowercase());
        cache.remove(&account.email.trim().to_lowercase());
        cache.remove(&email_oauth_secret_user(&account.email));
    }

    let db = state.db.lock().map_err(|error| error.to_string())?;
    db.execute(
        "DELETE FROM accounts WHERE id = ?1",
        params![input.account_id],
    )
    .map_err(|error| error.to_string())?;

    db.execute(
        "INSERT INTO audit_log (id, action, entity_type, entity_id, metadata_json)
         VALUES (?1, 'account.remove', 'account', ?2, ?3)",
        params![
            format!("audit-{}", uuid::Uuid::new_v4()),
            account.id,
            serde_json::json!({
                "summary": format!("Removed account {} and cleared local credentials.", account.email)
            })
            .to_string()
        ],
    )
    .ok();

    Ok(MessageActionResult {
        ok: true,
        summary: "Konto wurde lokal entfernt und Credentials wurden bereinigt.".into(),
    })
}

#[tauri::command]
fn accounts_test_imap_connection(
    input: CreateImapAccountInput,
) -> Result<ImapConnectionTestResult, String> {
    let email = input.email.trim().to_lowercase();
    let provider_config = provider_config_for_email(&email);
    let host = if input.imap_host.trim().is_empty() {
        provider_config
            .as_ref()
            .map(|config| config.imap_host.to_string())
            .unwrap_or_default()
    } else {
        input.imap_host.trim().to_string()
    };
    let port = if input.imap_port == 0 {
        provider_config
            .as_ref()
            .map(|config| config.imap_port)
            .unwrap_or(993)
    } else {
        input.imap_port
    };

    if host.is_empty() {
        return Err("IMAP-Host ist erforderlich.".into());
    }

    if port == 0 {
        return Err("IMAP-Port muss größer als 0 sein.".into());
    }

    if email.is_empty() || input.password.is_empty() {
        return Err("E-Mail und Passwort sind für den Login-Test erforderlich.".into());
    }

    let mut session = login_imap(&host, port, &email, &input.password)?;
    let capabilities = session
        .capabilities()
        .map(|caps| {
            caps.iter()
                .take(8)
                .map(|cap| format!("{cap:?}"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let _ = session.logout();

    Ok(ImapConnectionTestResult {
        ok: true,
        summary: "IMAP-Login erfolgreich".into(),
        details: format!(
            "{}:{} hat den Login akzeptiert. Erkannte Server-Funktionen: {}",
            host,
            port,
            if capabilities.is_empty() {
                "keine Capability-Liste gelesen".into()
            } else {
                capabilities.join(", ")
            }
        ),
    })
}

fn test_imap_transport(host: &str, port: u16) -> Result<ImapConnectionTestResult, String> {
    let host = host.trim().to_string();

    let timeout = Duration::from_secs(8);
    let address = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|error| format!("DNS-Auflösung fehlgeschlagen: {error}"))?
        .next()
        .ok_or_else(|| "DNS-Auflösung lieferte keine Adresse.".to_string())?;

    let started_at = Instant::now();
    let stream = TcpStream::connect_timeout(&address, timeout)
        .map_err(|error| format!("TCP-Verbindung fehlgeschlagen: {error}"))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| error.to_string())?;

    if port == 993 {
        let connector = native_tls::TlsConnector::new()
            .map_err(|error| format!("TLS-Initialisierung fehlgeschlagen: {error}"))?;
        connector
            .connect(host.as_str(), stream)
            .map_err(|error| format!("TLS-Handshake fehlgeschlagen: {error}"))?;
        return Ok(ImapConnectionTestResult {
            ok: true,
            summary: "IMAP TLS erreichbar".into(),
            details: format!(
                "{}:{} antwortet per TLS. Dauer: {} ms. Login wurde noch nicht versucht.",
                host,
                port,
                started_at.elapsed().as_millis()
            ),
        });
    }

    let mut stream = stream;
    let mut buffer = [0_u8; 160];
    let greeting = match stream.read(&mut buffer) {
        Ok(read) if read > 0 => String::from_utf8_lossy(&buffer[..read]).trim().to_string(),
        Ok(_) => "Keine Begrüßung gelesen.".into(),
        Err(error) => format!("TCP ok, Begrüßung nicht gelesen: {error}"),
    };

    Ok(ImapConnectionTestResult {
        ok: true,
        summary: "IMAP TCP erreichbar".into(),
        details: format!(
            "{}:{} ist erreichbar. Gelesene Antwort: {}. STARTTLS/Login sind noch nicht implementiert.",
            host, port, greeting
        ),
    })
}

#[tauri::command]
async fn accounts_sync_now(
    app: AppHandle,
    account_id: String,
    limit: Option<u32>,
) -> Result<SyncResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        sync_account_delta_guarded(&state, &account_id, limit.unwrap_or(250).clamp(1, 1000))
    })
    .await
    .map_err(|error| format!("Sync-Worker wurde unerwartet beendet: {error}"))?
}

#[tauri::command]
fn desktop_preferences_get(state: State<'_, AppState>) -> Result<DesktopPreferences, String> {
    load_desktop_preferences(&state)
}

#[tauri::command]
fn desktop_preferences_update(
    state: State<'_, AppState>,
    input: DesktopPreferencesInput,
) -> Result<DesktopPreferences, String> {
    let preferences = DesktopPreferences {
        auto_sync: input.auto_sync,
        notifications: input.notifications,
        privacy_mode: input.privacy_mode,
        launch_at_login: input.launch_at_login,
        minimize_to_tray: input.minimize_to_tray,
        initialized: true,
    };
    save_desktop_preferences(&state, &preferences)?;
    Ok(preferences)
}

#[tauri::command]
async fn sync_delta(app: AppHandle, account_id: Option<String>) -> Result<Vec<SyncResult>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let account_ids = account_ids_for_sync(&state, account_id.as_deref())?;
        Ok(account_ids
            .into_iter()
            .map(|id| match sync_account_delta_guarded(&state, &id, 250) {
                Ok(result) => result,
                Err(error) => sync_error_result(&id, "Delta-Sync fehlgeschlagen", error),
            })
            .collect())
    })
    .await
    .map_err(|error| format!("Sync-Worker wurde unerwartet beendet: {error}"))?
}

#[tauri::command]
async fn sync_backfill(
    app: AppHandle,
    input: Option<SyncBackfillInput>,
) -> Result<Vec<SyncResult>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let input = input.unwrap_or(SyncBackfillInput {
            account_id: None,
            folder_id: None,
            batch_size: Some(250),
        });
        let batch_size = input.batch_size.unwrap_or(250).clamp(25, 500);
        let account_ids = account_ids_for_sync(&state, input.account_id.as_deref())?;
        Ok(account_ids
            .into_iter()
            .map(|id| {
                match sync_account_backfill_guarded(
                    &state,
                    &id,
                    input.folder_id.as_deref(),
                    batch_size,
                ) {
                    Ok(result) => result,
                    Err(error) => sync_error_result(&id, "Backfill fehlgeschlagen", error),
                }
            })
            .collect())
    })
    .await
    .map_err(|error| format!("Backfill-Worker wurde unerwartet beendet: {error}"))?
}

fn account_sync_lock(
    state: &State<'_, AppState>,
    account_id: &str,
) -> Result<Arc<Mutex<()>>, String> {
    let mut guards = state
        .account_sync_guards
        .lock()
        .map_err(|error| error.to_string())?;
    Ok(guards
        .entry(account_id.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone())
}

fn sync_account_delta_guarded(
    state: &State<'_, AppState>,
    account_id: &str,
    limit: u32,
) -> Result<SyncResult, String> {
    let sync_lock = account_sync_lock(state, account_id)?;
    let _guard = sync_lock.try_lock().map_err(|_| {
        format!("Für dieses Konto läuft bereits eine Synchronisation: {account_id}")
    })?;
    sync_account_delta(state, account_id, limit)
}

fn sync_account_backfill_guarded(
    state: &State<'_, AppState>,
    account_id: &str,
    folder_id: Option<&str>,
    batch_size: u32,
) -> Result<SyncResult, String> {
    let sync_lock = account_sync_lock(state, account_id)?;
    let _guard = sync_lock.try_lock().map_err(|_| {
        format!("Für dieses Konto läuft bereits eine Synchronisation: {account_id}")
    })?;
    sync_account_backfill(state, account_id, folder_id, batch_size)
}

fn sync_error_result(account_id: &str, summary: &str, error: String) -> SyncResult {
    SyncResult {
        ok: false,
        account_id: account_id.to_string(),
        folder_count: 0,
        message_count: 0,
        new_messages: Vec::new(),
        summary: summary.to_string(),
        details: error,
    }
}

#[tauri::command]
async fn sync_status(app: AppHandle) -> Result<Vec<SyncStatus>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let db = open_read_database(&state)?;
        let mut statement = db
            .prepare(
                "SELECT a.id, a.email, f.id, f.display_name, f.role, f.last_synced_at,
                    ss.last_error, ss.cursor_json
             FROM folders f
             JOIN accounts a ON a.id = f.account_id
             LEFT JOIN sync_state ss ON ss.folder_id = f.id
             WHERE a.email NOT LIKE '%@example.com'
             ORDER BY lower(a.email), CASE f.role
                WHEN 'inbox' THEN 0
                WHEN 'archive' THEN 1
                WHEN 'sent' THEN 2
                WHEN 'spam' THEN 3
                WHEN 'trash' THEN 4
                ELSE 9
             END, lower(f.display_name)",
            )
            .map_err(|error| error.to_string())?;
        let statuses = statement
            .query_map([], |row| {
                let cursor_json = row
                    .get::<_, Option<String>>(7)?
                    .unwrap_or_else(|| "{}".into());
                let cursor = parse_sync_cursor(&cursor_json);
                Ok(SyncStatus {
                    account_id: row.get(0)?,
                    account_email: row.get(1)?,
                    folder_id: row.get(2)?,
                    folder: row.get(3)?,
                    role: row.get(4)?,
                    last_synced_at: row.get(5)?,
                    last_error: row.get(6)?,
                    backfill_complete: cursor.backfill_complete,
                    highest_uid: cursor.highest_uid,
                    lowest_uid: cursor.lowest_uid,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        Ok(statuses)
    })
    .await
    .map_err(|error| format!("Sync-Status-Worker wurde unerwartet beendet: {error}"))?
}

fn sync_account_delta(
    state: &State<'_, AppState>,
    account_id: &str,
    initial_limit: u32,
) -> Result<SyncResult, String> {
    let account = load_account_connection(&state, &account_id)?;
    let config = connection_config(&account)?;
    let login_secret = load_login_secret(&state, &account)?;
    let fetch_limit = initial_limit.clamp(1, 1000);
    let imap_host = config
        .imap_host
        .as_deref()
        .ok_or_else(|| "IMAP-Host fehlt.".to_string())?;
    let imap_port = config
        .imap_port
        .ok_or_else(|| "IMAP-Port fehlt.".to_string())?;

    let mut session = login_imap_account(imap_host, imap_port, &account, &login_secret)?;
    let folders = session
        .list(None, Some("*"))
        .map_err(|error| format!("Ordner konnten nicht gelesen werden: {error}"))?;
    let discovered_folders = folders
        .iter()
        .map(|folder| discovered_folder(folder.name(), folder.attributes()))
        .collect::<Vec<_>>();
    let folder_names = discovered_folders
        .iter()
        .map(|folder| folder.name.clone())
        .collect::<Vec<_>>();
    {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        upsert_discovered_folders(&db, &account.id, &discovered_folders)?;
    }
    let sync_folders = syncable_folders(&discovered_folders);
    let inbox_folder = sync_folders
        .iter()
        .find(|folder| folder.role.as_deref() == Some("inbox"))
        .map(|folder| folder.name.clone())
        .unwrap_or_else(|| "INBOX".into());
    let mut saved_count = 0_usize;
    let mut new_messages = Vec::new();
    let mut synced_folder_count = 0_usize;
    let mut sync_errors = Vec::new();
    for folder in &sync_folders {
        let folder_name = &folder.name;
        let folder_id = {
            let db = state.db.lock().map_err(|error| error.to_string())?;
            upsert_discovered_folder(&db, &account.id, folder)?
        };
        let mailbox = match session.select(folder_name) {
            Ok(mailbox) => mailbox,
            Err(error) => {
                let db = state
                    .db
                    .lock()
                    .map_err(|lock_error| lock_error.to_string())?;
                record_sync_error(
                    &db,
                    &account.id,
                    &folder_id,
                    &format!("Ordnerauswahl fehlgeschlagen: {error}"),
                )?;
                continue;
            }
        };
        let uid_validity = mailbox.uid_validity.map(|value| value.to_string());
        let uid_next = mailbox.uid_next.map(|value| value.to_string());
        let mut cursor = {
            let db = state.db.lock().map_err(|error| error.to_string())?;
            update_folder_sync_metadata(
                &db,
                &folder_id,
                uid_validity.as_deref(),
                uid_next.as_deref(),
            )?;
            load_sync_cursor(&db, &account.id, &folder_id)?
        };
        let is_initial_folder_sync = cursor.highest_uid.is_none();
        if cursor.uid_validity.is_some() && cursor.uid_validity != uid_validity {
            cursor = SyncCursor {
                uid_validity: uid_validity.clone(),
                ..SyncCursor::default()
            };
        } else if cursor.uid_validity.is_none() {
            cursor.uid_validity = uid_validity.clone();
        }
        let fetched_messages = match fetch_delta_messages(
            &mut session,
            mailbox.exists,
            cursor.highest_uid,
            fetch_limit,
        ) {
            Ok(messages) => messages,
            Err(error) => {
                let summary =
                    format!("Nachrichten konnten nicht aus {folder_name} geladen werden: {error}");
                if let Ok(db) = state.db.lock() {
                    let _ = record_sync_error(&db, &account.id, &folder_id, &summary);
                }
                sync_errors.push(summary);
                continue;
            }
        };
        let auto_moved_uids = if folder.role.as_deref() == Some("spam") {
            let trusted_senders = {
                let db = state.db.lock().map_err(|error| error.to_string())?;
                trusted_sender_emails_locked(&db, &account.id)?
            };
            let candidates = fetched_messages
                .iter()
                .filter(|message| {
                    trusted_senders.contains(&message.from_email.trim().to_ascii_lowercase())
                })
                .collect::<Vec<_>>();
            let mut moved = HashSet::new();
            for message in candidates {
                if let Err(error) = session.uid_copy(&message.uid, &inbox_folder) {
                    sync_errors.push(format!(
                        "Vertrauenswürdige Nachricht von {} konnte nicht in den Posteingang kopiert werden: {error}",
                        message.from_email
                    ));
                    continue;
                }
                if let Err(error) = session.uid_store(&message.uid, "+FLAGS.SILENT (\\Deleted)") {
                    sync_errors.push(format!(
                        "Vertrauenswürdige Nachricht von {} konnte nicht aus Spam entfernt werden: {error}",
                        message.from_email
                    ));
                    continue;
                }
                moved.insert(message.uid.clone());
            }
            if !moved.is_empty() {
                if let Err(error) = session.expunge() {
                    sync_errors.push(format!(
                        "Automatische Kein-Spam-Korrektur konnte nicht abgeschlossen werden: {error}"
                    ));
                    moved.clear();
                }
            }
            moved
        } else {
            HashSet::new()
        };

        for chunk in fetched_messages.chunks(40) {
            let mut db = state.db.lock().map_err(|error| error.to_string())?;
            let transaction = db.transaction().map_err(|error| error.to_string())?;
            for message in chunk {
                if auto_moved_uids.contains(&message.uid) {
                    continue;
                }
                upsert_sender(&transaction, message)?;
                let upserted = upsert_message(
                    &transaction,
                    &state.attachment_dir,
                    &account.id,
                    &folder_id,
                    message,
                )?;
                if message.has_newsletter_signals {
                    upsert_newsletter_candidate(&transaction, &account.id, message)?;
                }
                if upserted.inserted
                    && !is_initial_folder_sync
                    && folder.role.as_deref() == Some("inbox")
                    && flags_json_is_unread(&message.flags_json)
                {
                    new_messages.push(NewMailNotification {
                        message_id: upserted.message_id,
                        account_id: account.id.clone(),
                        sender: message.from_name.clone(),
                        subject: message.subject.clone(),
                    });
                }
                saved_count += 1;
            }
            update_cursor_from_messages(&mut cursor, chunk);
            save_sync_cursor(&transaction, &account.id, &folder_id, &cursor, None)?;
            transaction.commit().map_err(|error| error.to_string())?;
            drop(db);
            thread::yield_now();
        }
        if fetched_messages.is_empty() {
            let db = state.db.lock().map_err(|error| error.to_string())?;
            save_sync_cursor(&db, &account.id, &folder_id, &cursor, None)?;
        }
        synced_folder_count += 1;
    }
    let _ = session.logout();

    let db = state.db.lock().map_err(|error| error.to_string())?;
    db.execute(
        "UPDATE accounts
         SET sync_enabled = 1,
             last_sync_at = CASE WHEN ?2 = 0 THEN CURRENT_TIMESTAMP ELSE last_sync_at END,
             updated_at = CURRENT_TIMESTAMP
         WHERE id = ?1",
        params![account.id, sync_errors.len() as i64],
    )
    .map_err(|error| error.to_string())?;

    db.execute(
        "INSERT INTO audit_log (id, action, entity_type, entity_id, metadata_json)
         VALUES (?1, 'sync.inbox', 'account', ?2, ?3)",
        params![
            format!("audit-{}", uuid::Uuid::new_v4()),
            account.id,
            serde_json::json!({
                "summary": format!("Synced {saved_count} messages from {imap_host}"),
                "imapHost": imap_host,
                "folderCount": folder_names.len(),
                "syncedFolders": sync_folders.iter().map(|folder| folder.name.clone()).collect::<Vec<_>>(),
                "messageCount": saved_count,
                "errorCount": sync_errors.len(),
                "errors": sync_errors.clone()
            })
            .to_string()
        ],
    )
    .map_err(|error| error.to_string())?;

    Ok(SyncResult {
        ok: sync_errors.is_empty(),
        account_id: account_id.to_string(),
        folder_count: synced_folder_count,
        message_count: saved_count,
        new_messages,
        summary: if sync_errors.is_empty() {
            "Inbox-Sync abgeschlossen".into()
        } else {
            "Inbox-Sync teilweise abgeschlossen".into()
        },
        details: format!(
            "{} von {} Ordnern verarbeitet, {} neue/aktualisierte Nachrichten lokal gespeichert.{}",
            synced_folder_count,
            sync_folders.len(),
            saved_count,
            if sync_errors.is_empty() {
                String::new()
            } else {
                format!(" Fehler: {}", sync_errors.join(" | "))
            }
        ),
    })
}

fn sync_account_backfill(
    state: &State<'_, AppState>,
    account_id: &str,
    only_folder_id: Option<&str>,
    batch_size: u32,
) -> Result<SyncResult, String> {
    let account = load_account_connection(&state, account_id)?;
    let config = connection_config(&account)?;
    let login_secret = load_login_secret(&state, &account)?;
    let imap_host = config
        .imap_host
        .as_deref()
        .ok_or_else(|| "IMAP-Host fehlt.".to_string())?;
    let imap_port = config
        .imap_port
        .ok_or_else(|| "IMAP-Port fehlt.".to_string())?;
    let mut session = login_imap_account(imap_host, imap_port, &account, &login_secret)?;
    let folders = session
        .list(None, Some("*"))
        .map_err(|error| format!("Ordner konnten nicht gelesen werden: {error}"))?;
    let discovered_folders = folders
        .iter()
        .map(|folder| discovered_folder(folder.name(), folder.attributes()))
        .collect::<Vec<_>>();
    {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        upsert_discovered_folders(&db, &account.id, &discovered_folders)?;
    }
    let mut saved_count = 0_usize;
    let mut synced_folder_count = 0_usize;
    let mut sync_errors = Vec::new();
    let sync_folders = syncable_folders(&discovered_folders);
    for folder in &sync_folders {
        let folder_name = &folder.name;
        let folder_id = {
            let db = state.db.lock().map_err(|error| error.to_string())?;
            upsert_discovered_folder(&db, &account.id, folder)?
        };
        if only_folder_id.is_some_and(|target| target != folder_id) {
            continue;
        }
        let mailbox = match session.select(folder_name) {
            Ok(mailbox) => mailbox,
            Err(error) => {
                let summary = format!("Ordnerauswahl fehlgeschlagen: {error}");
                if let Ok(db) = state.db.lock() {
                    let _ = record_sync_error(&db, &account.id, &folder_id, &summary);
                }
                sync_errors.push(summary);
                continue;
            }
        };
        let uid_validity = mailbox.uid_validity.map(|value| value.to_string());
        let uid_next = mailbox.uid_next.map(|value| value.to_string());
        let mut cursor = {
            let db = state.db.lock().map_err(|error| error.to_string())?;
            update_folder_sync_metadata(
                &db,
                &folder_id,
                uid_validity.as_deref(),
                uid_next.as_deref(),
            )?;
            load_sync_cursor(&db, &account.id, &folder_id)?
        };
        if cursor.uid_validity.is_some() && cursor.uid_validity != uid_validity {
            cursor = SyncCursor {
                uid_validity: uid_validity.clone(),
                ..SyncCursor::default()
            };
        } else if cursor.uid_validity.is_none() {
            cursor.uid_validity = uid_validity.clone();
        }
        if cursor.highest_uid.is_none() {
            let initial = match fetch_delta_messages(&mut session, mailbox.exists, None, batch_size)
            {
                Ok(messages) => messages,
                Err(error) => {
                    let summary =
                        format!("Initialer Backfill aus {folder_name} fehlgeschlagen: {error}");
                    if let Ok(db) = state.db.lock() {
                        let _ = record_sync_error(&db, &account.id, &folder_id, &summary);
                    }
                    sync_errors.push(summary);
                    continue;
                }
            };
            for chunk in initial.chunks(40) {
                let mut db = state.db.lock().map_err(|error| error.to_string())?;
                let transaction = db.transaction().map_err(|error| error.to_string())?;
                for message in chunk {
                    upsert_sender(&transaction, message)?;
                    upsert_message(
                        &transaction,
                        &state.attachment_dir,
                        &account.id,
                        &folder_id,
                        message,
                    )?;
                    if message.has_newsletter_signals {
                        upsert_newsletter_candidate(&transaction, &account.id, message)?;
                    }
                    saved_count += 1;
                }
                update_cursor_from_messages(&mut cursor, chunk);
                save_sync_cursor(&transaction, &account.id, &folder_id, &cursor, None)?;
                transaction.commit().map_err(|error| error.to_string())?;
                drop(db);
                thread::yield_now();
            }
            if initial.is_empty() {
                let db = state.db.lock().map_err(|error| error.to_string())?;
                save_sync_cursor(&db, &account.id, &folder_id, &cursor, None)?;
            }
            synced_folder_count += 1;
            continue;
        }
        if cursor.backfill_complete {
            synced_folder_count += 1;
            continue;
        }
        let backfill_batch =
            match fetch_backfill_messages(&mut session, cursor.lowest_uid, batch_size) {
                Ok(batch) => batch,
                Err(error) => {
                    let summary = format!(
                    "Ältere Nachrichten konnten nicht aus {folder_name} geladen werden: {error}"
                );
                    if let Ok(db) = state.db.lock() {
                        let _ = record_sync_error(&db, &account.id, &folder_id, &summary);
                    }
                    sync_errors.push(summary);
                    continue;
                }
            };
        for chunk in backfill_batch.messages.chunks(40) {
            let mut db = state.db.lock().map_err(|error| error.to_string())?;
            let transaction = db.transaction().map_err(|error| error.to_string())?;
            for message in chunk {
                upsert_sender(&transaction, message)?;
                upsert_message(
                    &transaction,
                    &state.attachment_dir,
                    &account.id,
                    &folder_id,
                    message,
                )?;
                if message.has_newsletter_signals {
                    upsert_newsletter_candidate(&transaction, &account.id, message)?;
                }
                saved_count += 1;
            }
            update_cursor_from_messages(&mut cursor, chunk);
            save_sync_cursor(&transaction, &account.id, &folder_id, &cursor, None)?;
            transaction.commit().map_err(|error| error.to_string())?;
            drop(db);
            thread::yield_now();
        }
        if let Some(lowest_requested_uid) = backfill_batch.lowest_requested_uid {
            cursor.lowest_uid = Some(cursor.lowest_uid.map_or(lowest_requested_uid, |current| {
                current.min(lowest_requested_uid)
            }));
        }
        cursor.backfill_complete = !backfill_batch.has_more;
        {
            let db = state.db.lock().map_err(|error| error.to_string())?;
            save_sync_cursor(&db, &account.id, &folder_id, &cursor, None)?;
        }
        synced_folder_count += 1;
    }
    let _ = session.logout();
    let db = state.db.lock().map_err(|error| error.to_string())?;
    db.execute(
        "UPDATE accounts
         SET sync_enabled = 1,
             last_sync_at = CASE WHEN ?2 = 0 THEN CURRENT_TIMESTAMP ELSE last_sync_at END,
             updated_at = CURRENT_TIMESTAMP
         WHERE id = ?1",
        params![account.id, sync_errors.len() as i64],
    )
    .map_err(|error| error.to_string())?;
    Ok(SyncResult {
        ok: sync_errors.is_empty(),
        account_id: account_id.to_string(),
        folder_count: synced_folder_count,
        message_count: saved_count,
        new_messages: Vec::new(),
        summary: if sync_errors.is_empty() {
            "Backfill abgeschlossen".into()
        } else {
            "Backfill teilweise abgeschlossen".into()
        },
        details: if sync_errors.is_empty() {
            format!("{saved_count} ältere Nachrichten paketweise nachgeladen.")
        } else {
            format!(
                "{saved_count} ältere Nachrichten paketweise nachgeladen. Fehler: {}",
                sync_errors.join(" | ")
            )
        },
    })
}

#[tauri::command]
async fn messages_list(
    app: AppHandle,
    input: Option<MessageListInput>,
) -> Result<MessageListResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let db = open_read_database(&state)?;
        let started_at = Instant::now();
        let result = messages_list_from_db(&db, input);
        if started_at.elapsed() > Duration::from_millis(50) {
            eprintln!(
                "KizuMail performance: messages_list took {} ms",
                started_at.elapsed().as_millis()
            );
        }
        result
    })
    .await
    .map_err(|error| format!("Nachrichtenliste-Worker wurde unerwartet beendet: {error}"))?
}

#[tauri::command]
async fn messages_get(app: AppHandle, message_id: String) -> Result<MessageSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let db = open_read_database(&state)?;
        message_get_from_db(&db, &message_id)
    })
    .await
    .map_err(|error| format!("Nachrichtendetail-Worker wurde unerwartet beendet: {error}"))?
}

fn messages_list_from_db(
    db: &Connection,
    input: Option<MessageListInput>,
) -> Result<MessageListResult, String> {
    let input = input.unwrap_or(MessageListInput {
        limit: Some(250),
        offset: Some(0),
        sort: Some("newest".into()),
        account_id: None,
        folder: None,
        folder_role: None,
        unread: None,
        flagged: None,
        has_attachments: None,
        date_from: None,
        date_to: None,
    });
    let limit = input.limit.unwrap_or(250).clamp(1, 1000);
    let offset = input.offset.unwrap_or(0).min(100_000);
    let order_by = message_order_by(input.sort.as_deref());
    let fair_order_by = fair_message_order_by(input.sort.as_deref());
    let mut where_clauses = vec!["a.email NOT LIKE '%@example.com'".to_string()];
    let mut bind_values = vec![(limit + 1).to_string(), offset.to_string()];
    let explicit_account_id = input
        .account_id
        .as_ref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_string());
    if let Some(account_id) = explicit_account_id.as_ref() {
        where_clauses.push(format!("m.account_id = ?{}", bind_values.len() + 1));
        bind_values.push(account_id.clone());
    }
    if let Some(folder) = input
        .folder
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        where_clauses.push(format!(
            "lower(f.display_name) LIKE lower(?{})",
            bind_values.len() + 1
        ));
        bind_values.push(format!("%{}%", folder.trim()));
    }
    if let Some(folder_role) = input
        .folder_role
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        where_clauses.push(format!("f.role = ?{}", bind_values.len() + 1));
        bind_values.push(folder_role.trim().to_ascii_lowercase());
    }
    if let Some(unread) = input.unread {
        where_clauses.push(if unread {
            "instr(m.flags_json, '\\Seen') = 0".into()
        } else {
            "instr(m.flags_json, '\\Seen') > 0".into()
        });
    }
    if let Some(flagged) = input.flagged {
        where_clauses.push(if flagged {
            "instr(m.flags_json, '\\Flagged') > 0".into()
        } else {
            "instr(m.flags_json, '\\Flagged') = 0".into()
        });
    }
    if let Some(has_attachments) = input.has_attachments {
        where_clauses.push(if has_attachments {
            "m.has_attachments = 1".into()
        } else {
            "m.has_attachments = 0".into()
        });
    }
    if let Some(date_from) = input
        .date_from
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        where_clauses.push(format!(
            "COALESCE(datetime(m.date), datetime(m.created_at)) >= datetime(?{})",
            bind_values.len() + 1
        ));
        bind_values.push(date_from.trim().to_string());
    }
    if let Some(date_to) = input
        .date_to
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        where_clauses.push(format!(
            "COALESCE(datetime(m.date), datetime(m.created_at)) <= datetime(?{})",
            bind_values.len() + 1
        ));
        bind_values.push(date_to.trim().to_string());
    }
    let where_sql = where_clauses.join(" AND ");
    let query = if explicit_account_id.is_some() {
        format!(
            "SELECT m.id,
                    m.account_id,
                    COALESCE(s.display_name, m.from_addr) AS sender,
                    m.from_addr,
                    m.subject,
                    substr(m.body_text, 1, 180) AS preview,
                    '' AS body_text,
                    NULL AS body_html_sanitized,
                    COALESCE(m.date, m.created_at) AS date,
                    f.display_name AS folder,
                    CASE WHEN instr(m.flags_json, '\\Seen') = 0 THEN 1 ELSE 0 END AS unread,
                    CASE WHEN instr(m.flags_json, '\\Flagged') > 0 THEN 1 ELSE 0 END AS flagged,
                    m.has_attachments,
                    CASE WHEN ns.id IS NULL THEN 0 ELSE 1 END AS has_newsletter_signals,
                    '[]' AS attachments_json,
                    m.raw_headers_json,
                    m.created_at AS created_at,
                    COUNT(*) OVER () AS total_available,
                    f.role AS folder_role
             FROM messages m
             JOIN folders f ON f.id = m.folder_id
             JOIN accounts a ON a.id = m.account_id
             LEFT JOIN senders s ON lower(s.normalized_email) = lower(m.from_addr)
                    LEFT JOIN newsletter_subscriptions ns ON ns.sender_id = s.id AND ns.account_id = m.account_id
             WHERE {where_sql}
             ORDER BY {order_by}
             LIMIT ?1 OFFSET ?2"
        )
    } else {
        let fair_target_parameter = bind_values.len() + 1;
        bind_values.push(fair_page_target(limit, offset).to_string());
        let fair_row_columns = "id, account_id, sender, from_addr, subject, preview, body_text,
                                 body_html_sanitized, date, folder, unread, flagged,
                                 has_attachments, has_newsletter_signals, attachments_json,
                                 raw_headers_json, created_at, total_available, folder_role,
                                 account_rank";
        format!(
            "WITH eligible AS (
                 SELECT m.id,
                        m.account_id,
                        COALESCE(s.display_name, m.from_addr) AS sender,
                        m.from_addr,
                        m.subject,
                        substr(m.body_text, 1, 180) AS preview,
                         '' AS body_text,
                         NULL AS body_html_sanitized,
                        COALESCE(m.date, m.created_at) AS date,
                        f.display_name AS folder,
                        CASE WHEN instr(m.flags_json, '\\Seen') = 0 THEN 1 ELSE 0 END AS unread,
                        CASE WHEN instr(m.flags_json, '\\Flagged') > 0 THEN 1 ELSE 0 END AS flagged,
                        m.has_attachments,
                        CASE WHEN ns.id IS NULL THEN 0 ELSE 1 END AS has_newsletter_signals,
                         '[]' AS attachments_json,
                        m.raw_headers_json,
                        m.created_at,
                        COUNT(*) OVER () AS total_available,
                        f.role AS folder_role
                 FROM messages m
                 JOIN folders f ON f.id = m.folder_id
                 JOIN accounts a ON a.id = m.account_id
                 LEFT JOIN senders s ON lower(s.normalized_email) = lower(m.from_addr)
                 LEFT JOIN newsletter_subscriptions ns
                   ON ns.sender_id = s.id AND ns.account_id = m.account_id
                 WHERE {where_sql}
             ),
             ranked AS (
                 SELECT eligible.*,
                        ROW_NUMBER() OVER (
                            PARTITION BY account_id
                            ORDER BY {fair_order_by}, id
                        ) AS account_rank
                 FROM eligible
             ),
             account_count AS (
                 SELECT COUNT(DISTINCT account_id) AS count
                 FROM eligible
             ),
             quota AS (
                 SELECT CASE
                          WHEN count = 0 THEN 0
                          ELSE (?{fair_target_parameter} + count - 1) / count
                        END AS per_account_quota
                 FROM account_count
             ),
             base AS (
                 SELECT {fair_row_columns}
                 FROM ranked
                 CROSS JOIN quota
                 WHERE account_rank <= per_account_quota
             ),
             extra_ranked AS (
                 SELECT ranked.*,
                        ROW_NUMBER() OVER (ORDER BY {fair_order_by}, id) AS extra_rank
                 FROM ranked
                 CROSS JOIN quota
                 WHERE account_rank > per_account_quota
             ),
             fair AS (
                 SELECT {fair_row_columns}
                 FROM base
                 UNION ALL
                 SELECT {fair_row_columns}
                 FROM extra_ranked
                 WHERE extra_rank <= (?{fair_target_parameter} - (SELECT COUNT(*) FROM base))
             )
             SELECT m.id,
                    m.account_id,
                    m.sender,
                    m.from_addr,
                    m.subject,
                    m.preview,
                    m.body_text,
                    m.body_html_sanitized,
                    m.date,
                    m.folder,
                    m.unread,
                    m.flagged,
                    m.has_attachments,
                    m.has_newsletter_signals,
                    m.attachments_json,
                    m.raw_headers_json,
                    m.created_at,
                    m.total_available,
                    m.folder_role
             FROM fair m
             ORDER BY {fair_order_by}, m.id
             LIMIT ?1 OFFSET ?2"
        )
    };
    let mut statement = db.prepare(&query).map_err(|error| error.to_string())?;

    let mut total_available = None;
    let mut messages = statement
        .query_map(params_from_iter(bind_values.iter()), |row| {
            let attachments = parse_attachment_summaries(&row.get::<_, String>(14)?);
            let raw_headers_json = row.get::<_, String>(15)?;
            if total_available.is_none() {
                total_available = Some(row.get::<_, i64>(17)?.max(0) as usize);
            }
            Ok(MessageSummary {
                id: row.get(0)?,
                account_id: row.get(1)?,
                sender: row.get(2)?,
                sender_email: row.get(3)?,
                subject: row.get(4)?,
                preview: row.get(5)?,
                body_text: row.get(6)?,
                body_html: row.get(7)?,
                date: row.get(8)?,
                folder: row.get(9)?,
                folder_role: row.get(18)?,
                unread: row.get::<_, i64>(10)? == 1,
                flagged: row.get::<_, i64>(11)? == 1,
                has_attachments: row.get::<_, i64>(12)? == 1,
                has_newsletter_signals: row.get::<_, i64>(13)? == 1,
                render_mode_available: RenderModeAvailability {
                    readable: true,
                    original: row
                        .get::<_, Option<String>>(7)?
                        .map(|html| !html.trim().is_empty())
                        .unwrap_or(false),
                },
                render_warnings: Vec::new(),
                dedupe_key: message_dedupe_key(
                    &raw_headers_json,
                    row.get::<_, String>(3)?.as_str(),
                    row.get::<_, String>(4)?.as_str(),
                    row.get::<_, String>(8)?.as_str(),
                ),
                attachments,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    let has_more = total_available
        .map(|total| (offset as usize).saturating_add(limit as usize) < total)
        .unwrap_or_else(|| messages.len() > limit as usize);
    if has_more {
        messages.truncate(limit as usize);
    }
    let messages = dedupe_messages(messages);
    Ok(MessageListResult {
        messages,
        next_cursor: if has_more {
            Some((offset + limit).to_string())
        } else {
            None
        },
        has_more,
        sync_status: sync_overview(&db),
    })
}

fn message_get_from_db(db: &Connection, message_id: &str) -> Result<MessageSummary, String> {
    let message = db
        .query_row(
            "SELECT m.id,
                    m.account_id,
                    COALESCE(s.display_name, m.from_addr) AS sender,
                    m.from_addr,
                    m.subject,
                    substr(m.body_text, 1, 180) AS preview,
                    m.body_text,
                    m.body_html_sanitized,
                    COALESCE(m.date, m.created_at) AS date,
                    f.display_name AS folder,
                    CASE WHEN instr(m.flags_json, '\\Seen') = 0 THEN 1 ELSE 0 END AS unread,
                    CASE WHEN instr(m.flags_json, '\\Flagged') > 0 THEN 1 ELSE 0 END AS flagged,
                    m.has_attachments,
                    CASE WHEN ns.id IS NULL THEN 0 ELSE 1 END AS has_newsletter_signals,
                    COALESCE((
                        SELECT json_group_array(json_object(
                            'filename', COALESCE(att.filename, 'Anhang'),
                            'mimeType', COALESCE(att.mime_type, 'application/octet-stream'),
                            'size', att.size,
                            'contentId', att.content_id,
                            'storageRef', att.storage_ref,
                            'inline', CASE WHEN att.scan_status = 'inline_cached' THEN 1 ELSE 0 END
                        ))
                        FROM attachments att
                        WHERE att.message_id = m.id
                    ), '[]') AS attachments_json,
                    m.raw_headers_json,
                    f.role AS folder_role
             FROM messages m
             JOIN folders f ON f.id = m.folder_id
             LEFT JOIN senders s ON lower(s.normalized_email) = lower(m.from_addr)
             LEFT JOIN newsletter_subscriptions ns
               ON ns.sender_id = s.id AND ns.account_id = m.account_id
             WHERE m.id = ?1",
            params![message_id],
            |row| {
                let attachments = parse_attachment_summaries(&row.get::<_, String>(14)?);
                let raw_headers_json = row.get::<_, String>(15)?;
                let sender_email = row.get::<_, String>(3)?;
                let subject = row.get::<_, String>(4)?;
                let date = row.get::<_, String>(8)?;
                let body_html = row.get::<_, Option<String>>(7)?;
                Ok(MessageSummary {
                    id: row.get(0)?,
                    account_id: row.get(1)?,
                    sender: row.get(2)?,
                    sender_email: sender_email.clone(),
                    subject: subject.clone(),
                    preview: row.get(5)?,
                    body_text: row.get(6)?,
                    body_html: body_html.clone(),
                    date: date.clone(),
                    folder: row.get(9)?,
                    folder_role: row.get(16)?,
                    unread: row.get::<_, i64>(10)? == 1,
                    flagged: row.get::<_, i64>(11)? == 1,
                    has_attachments: row.get::<_, i64>(12)? == 1 && !attachments.is_empty(),
                    attachments,
                    has_newsletter_signals: row.get::<_, i64>(13)? == 1,
                    render_mode_available: RenderModeAvailability {
                        readable: true,
                        original: body_html
                            .as_deref()
                            .is_some_and(|html| !html.trim().is_empty()),
                    },
                    render_warnings: Vec::new(),
                    dedupe_key: message_dedupe_key(
                        &raw_headers_json,
                        &sender_email,
                        &subject,
                        &date,
                    ),
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Nachricht wurde lokal nicht gefunden.".to_string())?;
    Ok(message)
}

fn fair_page_target(limit: u32, offset: u32) -> usize {
    (limit as usize)
        .saturating_add(offset as usize)
        .saturating_add(1)
}

fn parse_attachment_summaries(raw: &str) -> Vec<AttachmentSummary> {
    serde_json::from_str::<Vec<serde_json::Value>>(raw)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| {
            let filename = value
                .get("filename")
                .and_then(|item| item.as_str())
                .unwrap_or("Anhang")
                .to_string();
            let mime_type = value
                .get("mimeType")
                .and_then(|item| item.as_str())
                .unwrap_or("application/octet-stream")
                .to_string();
            let inline = value
                .get("inline")
                .and_then(|item| item.as_i64())
                .unwrap_or(0)
                == 1;
            let storage_ref = value
                .get("storageRef")
                .and_then(|item| item.as_str())
                .map(|item| item.to_string());
            let disposition =
                attachment_disposition(&filename, &mime_type, inline, storage_ref.is_some());
            if disposition == "body" {
                return None;
            }
            Some(AttachmentSummary {
                filename,
                preview_kind: attachment_preview_kind(&mime_type),
                mime_type,
                size: value
                    .get("size")
                    .and_then(|item| item.as_i64())
                    .unwrap_or(0),
                content_id: value
                    .get("contentId")
                    .and_then(|item| item.as_str())
                    .map(|item| item.to_string()),
                download_path: storage_ref.clone(),
                storage_ref,
                inline,
                disposition,
            })
        })
        .collect()
}

fn message_order_by(sort: Option<&str>) -> &'static str {
    match sort.unwrap_or("newest") {
        "oldest" => "COALESCE(datetime(m.date), datetime(m.created_at)) ASC, m.created_at ASC",
        "unread" => "unread DESC, COALESCE(datetime(m.date), datetime(m.created_at)) DESC",
        "sender" => "lower(sender) ASC, COALESCE(datetime(m.date), datetime(m.created_at)) DESC",
        "subject" => {
            "lower(m.subject) ASC, COALESCE(datetime(m.date), datetime(m.created_at)) DESC"
        }
        "attachments" => {
            "m.has_attachments DESC, COALESCE(datetime(m.date), datetime(m.created_at)) DESC"
        }
        _ => "COALESCE(datetime(m.date), datetime(m.created_at)) DESC, m.created_at DESC",
    }
}

fn fair_message_order_by(sort: Option<&str>) -> &'static str {
    match sort.unwrap_or("newest") {
        "oldest" => "COALESCE(datetime(date), datetime(created_at)) ASC, created_at ASC",
        "unread" => "unread DESC, COALESCE(datetime(date), datetime(created_at)) DESC",
        "sender" => "lower(sender) ASC, COALESCE(datetime(date), datetime(created_at)) DESC",
        "subject" => "lower(subject) ASC, COALESCE(datetime(date), datetime(created_at)) DESC",
        "attachments" => {
            "has_attachments DESC, COALESCE(datetime(date), datetime(created_at)) DESC"
        }
        _ => "COALESCE(datetime(date), datetime(created_at)) DESC, created_at DESC",
    }
}

fn dedupe_messages(messages: Vec<MessageSummary>) -> Vec<MessageSummary> {
    let mut seen = HashSet::new();
    let mut output = Vec::with_capacity(messages.len());
    for message in messages {
        if seen.insert(message.dedupe_key.clone()) {
            output.push(message);
        }
    }
    output
}

fn message_dedupe_key(
    raw_headers_json: &str,
    from_addr: &str,
    subject: &str,
    date: &str,
) -> String {
    let message_id = serde_json::from_str::<serde_json::Value>(raw_headers_json)
        .ok()
        .and_then(|value| {
            value
                .get("messageId")
                .and_then(|item| item.as_str())
                .map(str::to_string)
        })
        .map(|value| {
            value
                .trim()
                .trim_matches('<')
                .trim_matches('>')
                .to_ascii_lowercase()
        })
        .filter(|value| !value.is_empty());
    message_id.unwrap_or_else(|| {
        format!(
            "{}|{}|{}",
            from_addr.trim().to_ascii_lowercase(),
            subject.trim().to_ascii_lowercase(),
            date.trim().to_ascii_lowercase()
        )
    })
}

fn account_ids_for_sync(
    state: &State<'_, AppState>,
    account_id: Option<&str>,
) -> Result<Vec<String>, String> {
    if let Some(account_id) = account_id.filter(|value| !value.trim().is_empty()) {
        return Ok(vec![account_id.to_string()]);
    }
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let mut statement = db
        .prepare("SELECT id FROM accounts WHERE sync_enabled = 1 AND email NOT LIKE '%@example.com' ORDER BY created_at")
        .map_err(|error| error.to_string())?;
    let account_ids = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(account_ids)
}

fn fetch_delta_messages(
    session: &mut imap::Session<native_tls::TlsStream<TcpStream>>,
    exists: u32,
    highest_uid: Option<u32>,
    initial_limit: u32,
) -> Result<Vec<ParsedMessage>, String> {
    if exists == 0 {
        return Ok(Vec::new());
    }
    let fetches = if let Some(highest_uid) = highest_uid {
        let next_uid = highest_uid.saturating_add(1);
        if next_uid <= highest_uid {
            return Ok(Vec::new());
        }
        session
            .uid_fetch(format!("{next_uid}:*"), "(UID FLAGS BODY.PEEK[])")
            .map_err(|error| error.to_string())?
    } else {
        let start = exists.saturating_sub(initial_limit) + 1;
        session
            .fetch(format!("{start}:*"), "(UID FLAGS BODY.PEEK[])")
            .map_err(|error| error.to_string())?
    };
    Ok(fetches.iter().filter_map(parse_fetch).collect())
}

fn fetch_backfill_messages(
    session: &mut imap::Session<native_tls::TlsStream<TcpStream>>,
    lowest_uid: Option<u32>,
    batch_size: u32,
) -> Result<BackfillBatch, String> {
    let Some(lowest_uid) = lowest_uid else {
        return Ok(BackfillBatch::default());
    };
    if lowest_uid <= 1 {
        return Ok(BackfillBatch::default());
    }
    let end = lowest_uid - 1;
    let available_uids = session
        .uid_search(format!("UID 1:{end}"))
        .map_err(|error| error.to_string())?
        .into_iter()
        .collect::<Vec<_>>();
    let (selected_uids, has_more) = select_backfill_uids(available_uids, lowest_uid, batch_size);
    if selected_uids.is_empty() {
        return Ok(BackfillBatch::default());
    }
    let lowest_requested_uid = selected_uids.first().copied();
    let sequence_set = selected_uids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let fetches = session
        .uid_fetch(sequence_set, "(UID FLAGS BODY.PEEK[])")
        .map_err(|error| error.to_string())?;
    Ok(BackfillBatch {
        messages: fetches.iter().filter_map(parse_fetch).collect(),
        lowest_requested_uid,
        has_more,
    })
}

#[derive(Default)]
struct BackfillBatch {
    messages: Vec<ParsedMessage>,
    lowest_requested_uid: Option<u32>,
    has_more: bool,
}

fn select_backfill_uids(
    mut available_uids: Vec<u32>,
    lowest_uid: u32,
    batch_size: u32,
) -> (Vec<u32>, bool) {
    available_uids.retain(|uid| *uid < lowest_uid);
    available_uids.sort_unstable();
    available_uids.dedup();
    let take = batch_size.max(1) as usize;
    let split_at = available_uids.len().saturating_sub(take);
    let selected = available_uids.split_off(split_at);
    (selected, split_at > 0)
}

fn update_cursor_from_messages(cursor: &mut SyncCursor, messages: &[ParsedMessage]) {
    for uid in messages
        .iter()
        .filter_map(|message| message.uid.parse::<u32>().ok())
    {
        cursor.highest_uid = Some(cursor.highest_uid.map_or(uid, |current| current.max(uid)));
        cursor.lowest_uid = Some(cursor.lowest_uid.map_or(uid, |current| current.min(uid)));
    }
}

fn load_sync_cursor(
    db: &Connection,
    account_id: &str,
    folder_id: &str,
) -> Result<SyncCursor, String> {
    let raw = db
        .query_row(
            "SELECT cursor_json FROM sync_state WHERE account_id = ?1 AND folder_id = ?2",
            params![account_id, folder_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    Ok(raw.as_deref().map(parse_sync_cursor).unwrap_or_default())
}

fn parse_sync_cursor(raw: &str) -> SyncCursor {
    let mut cursor = serde_json::from_str::<SyncCursor>(raw).unwrap_or_default();
    if cursor.backfill_version < SYNC_BACKFILL_VERSION {
        cursor.backfill_complete = false;
        cursor.backfill_version = SYNC_BACKFILL_VERSION;
    }
    cursor
}

fn save_sync_cursor(
    db: &Connection,
    account_id: &str,
    folder_id: &str,
    cursor: &SyncCursor,
    last_error: Option<&str>,
) -> Result<(), String> {
    let cursor_json = serde_json::to_string(cursor).map_err(|error| error.to_string())?;
    db.execute(
        "INSERT INTO sync_state (id, account_id, folder_id, cursor_json, last_error, backoff_until, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, NULL, CURRENT_TIMESTAMP)
         ON CONFLICT(account_id, folder_id) DO UPDATE SET
           cursor_json = excluded.cursor_json,
           last_error = excluded.last_error,
           backoff_until = excluded.backoff_until,
           updated_at = CURRENT_TIMESTAMP",
        params![
            format!("sync-{}", uuid::Uuid::new_v4()),
            account_id,
            folder_id,
            cursor_json,
            last_error
        ],
    )
    .map_err(|error| error.to_string())?;
    if last_error.is_none() {
        db.execute(
            "UPDATE folders SET last_synced_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
            params![folder_id],
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn record_sync_error(
    db: &Connection,
    account_id: &str,
    folder_id: &str,
    error: &str,
) -> Result<(), String> {
    let cursor = load_sync_cursor(db, account_id, folder_id)?;
    save_sync_cursor(db, account_id, folder_id, &cursor, Some(error))
}

fn update_folder_sync_metadata(
    db: &Connection,
    folder_id: &str,
    uid_validity: Option<&str>,
    uid_next: Option<&str>,
) -> Result<(), String> {
    db.execute(
        "UPDATE folders
         SET uid_validity = COALESCE(?1, uid_validity),
             uid_next = COALESCE(?2, uid_next),
             updated_at = CURRENT_TIMESTAMP
         WHERE id = ?3",
        params![uid_validity, uid_next, folder_id],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn sync_overview(db: &Connection) -> SyncOverview {
    let incomplete = db
        .query_row(
            "SELECT COUNT(*)
             FROM sync_state
             WHERE json_extract(cursor_json, '$.backfillComplete') IS NOT 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0);
    let errors = db
        .query_row(
            "SELECT COUNT(*) FROM sync_state WHERE last_error IS NOT NULL AND trim(last_error) <> ''",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0);
    SyncOverview {
        running: false,
        summary: if errors > 0 {
            format!("{errors} Sync-Hinweis(e), ältere Mails können weiter nachgeladen werden.")
        } else if incomplete > 0 {
            "Smart Sync aktiv. Ältere Mails sind paketweise verfügbar.".into()
        } else {
            "Alle bekannten Ordner sind lokal bis zum aktuellen Cursor synchronisiert.".into()
        },
        backfill_available: incomplete > 0,
    }
}

fn attachment_disposition(
    filename: &str,
    mime_type: &str,
    inline: bool,
    has_storage: bool,
) -> String {
    let filename = filename.trim().to_ascii_lowercase();
    let mime_type = mime_type.trim().to_ascii_lowercase();
    if mime_type.starts_with("multipart/")
        || ((filename.is_empty() || filename == "anhang" || filename == "attachment")
            && (mime_type == "text/html" || mime_type == "text/plain"))
    {
        return "body".into();
    }
    if inline && has_storage {
        return "inline".into();
    }
    "attachment".into()
}

fn attachment_preview_kind(mime_type: &str) -> String {
    let mime_type = mime_type.to_ascii_lowercase();
    if mime_type.starts_with("image/") {
        "image".into()
    } else if mime_type.starts_with("video/") {
        "video".into()
    } else if mime_type == "application/pdf" {
        "pdf".into()
    } else {
        "file".into()
    }
}

fn update_local_message_flag(
    state: &State<'_, AppState>,
    message_id: &str,
    flag: &str,
    enabled: bool,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let current_json = db
        .query_row(
            "SELECT flags_json FROM messages WHERE id = ?1",
            params![message_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Nachricht wurde nicht gefunden.".to_string())?;
    let mut flags = serde_json::from_str::<Vec<String>>(&current_json).unwrap_or_default();
    if enabled {
        if !flags.iter().any(|item| item.eq_ignore_ascii_case(flag)) {
            flags.push(flag.to_string());
        }
    } else {
        flags.retain(|item| !item.eq_ignore_ascii_case(flag));
    }
    let next_json = serde_json::to_string(&flags).map_err(|error| error.to_string())?;
    db.execute(
        "UPDATE messages SET flags_json = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
        params![next_json, message_id],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
async fn messages_move_local(
    app: AppHandle,
    input: MoveMessageInput,
) -> Result<MessageActionResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        messages_move_local_blocking(state, input)
    })
    .await
    .map_err(|error| format!("Mail-Aktion wurde unerwartet beendet: {error}"))?
}

fn messages_move_local_blocking(
    state: State<'_, AppState>,
    input: MoveMessageInput,
) -> Result<MessageActionResult, String> {
    let message_context = load_message_context(&state, &input.message_id)?;
    let account = load_account_connection(&state, &message_context.account_id)?;
    let config = connection_config(&account)?;
    let login_secret = load_login_secret(&state, &account)?;
    let target_folder = match input.role.as_str() {
        "archive" => {
            find_existing_folder(&state, &message_context.account_id, &["archive", "archiv"])
                .unwrap_or_else(|| "Archive".into())
        }
        "trash" => find_existing_folder(
            &state,
            &message_context.account_id,
            &["trash", "papierkorb", "deleted"],
        )
        .unwrap_or_else(|| "Trash".into()),
        "spam" => find_existing_folder(&state, &message_context.account_id, &["spam", "junk"])
            .unwrap_or_else(|| "Junk".into()),
        "inbox" => find_existing_folder(
            &state,
            &message_context.account_id,
            &["inbox", "posteingang"],
        )
        .unwrap_or_else(|| "INBOX".into()),
        _ => return Err("Unbekanntes Verschiebeziel.".into()),
    };
    let imap_host = config
        .imap_host
        .as_deref()
        .ok_or_else(|| "IMAP-Host fehlt.".to_string())?;
    let imap_port = config
        .imap_port
        .ok_or_else(|| "IMAP-Port fehlt.".to_string())?;

    if !message_context
        .folder_path
        .eq_ignore_ascii_case(&target_folder)
    {
        let mut session = login_imap_account(imap_host, imap_port, &account, &login_secret)?;
        session
            .select(&message_context.folder_path)
            .map_err(|error| format!("Quellordner konnte nicht geöffnet werden: {error}"))?;
        session
            .uid_copy(&message_context.uid, &target_folder)
            .map_err(|error| {
                format!("Nachricht konnte serverseitig nicht kopiert werden: {error}")
            })?;
        session
            .uid_store(&message_context.uid, "+FLAGS.SILENT (\\Deleted)")
            .map_err(|error| {
                format!("Nachricht konnte serverseitig nicht als gelöscht markiert werden: {error}")
            })?;
        session.expunge().map_err(|error| {
            format!("Serverseitiges Entfernen aus dem Quellordner ist fehlgeschlagen: {error}")
        })?;
        let _ = session.logout();
    }

    let mut db = state.db.lock().map_err(|error| error.to_string())?;
    let transaction = db.transaction().map_err(|error| error.to_string())?;
    let folder_id = upsert_folder(&transaction, &message_context.account_id, &target_folder)?;
    transaction
        .execute(
            "UPDATE messages SET folder_id = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
            params![folder_id, input.message_id],
        )
        .map_err(|error| error.to_string())?;
    if input.role == "inbox" {
        set_sender_trusted_locked(
            &transaction,
            &message_context.account_id,
            &message_context.sender_email,
            true,
        )?;
        transaction
            .execute(
                "DELETE FROM blocked_senders WHERE account_id = ?1 AND lower(sender_email) = lower(?2)",
                params![&message_context.account_id, &message_context.sender_email],
            )
            .map_err(|error| error.to_string())?;
    } else if input.role == "spam" {
        set_sender_trusted_locked(
            &transaction,
            &message_context.account_id,
            &message_context.sender_email,
            false,
        )?;
    }
    transaction.commit().map_err(|error| error.to_string())?;

    Ok(MessageActionResult {
        ok: true,
        summary: if input.role == "inbox" {
            "Nachricht in den Posteingang verschoben. Künftige Nachrichten dieses Absenders werden automatisch aus Spam zurückgeführt.".into()
        } else {
            format!("Nachricht serverseitig und lokal nach {target_folder} verschoben.")
        },
    })
}

#[tauri::command]
async fn messages_mark_read(
    app: AppHandle,
    input: MarkMessageReadInput,
) -> Result<MessageActionResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        messages_mark_read_blocking(state, input)
    })
    .await
    .map_err(|error| format!("Gelesen-Status-Worker wurde unerwartet beendet: {error}"))?
}

fn messages_mark_read_blocking(
    state: State<'_, AppState>,
    input: MarkMessageReadInput,
) -> Result<MessageActionResult, String> {
    let message_context = load_message_context(&state, &input.message_id)?;
    let account = load_account_connection(&state, &message_context.account_id)?;
    let config = connection_config(&account)?;
    let login_secret = load_login_secret(&state, &account)?;
    let imap_host = config
        .imap_host
        .as_deref()
        .ok_or_else(|| "IMAP-Host fehlt.".to_string())?;
    let imap_port = config
        .imap_port
        .ok_or_else(|| "IMAP-Port fehlt.".to_string())?;
    let mut session = login_imap_account(imap_host, imap_port, &account, &login_secret)?;
    session
        .select(&message_context.folder_path)
        .map_err(|error| format!("Ordner konnte nicht geöffnet werden: {error}"))?;
    let operation = if input.unread {
        "-FLAGS.SILENT (\\Seen)"
    } else {
        "+FLAGS.SILENT (\\Seen)"
    };
    session
        .uid_store(&message_context.uid, operation)
        .map_err(|error| {
            format!("Gelesen-Status konnte serverseitig nicht geändert werden: {error}")
        })?;
    let _ = session.logout();
    update_local_message_flag(&state, &input.message_id, "\\Seen", !input.unread)?;

    Ok(MessageActionResult {
        ok: true,
        summary: if input.unread {
            "Nachricht serverseitig und lokal als ungelesen markiert.".into()
        } else {
            "Nachricht serverseitig und lokal als gelesen markiert.".into()
        },
    })
}

#[tauri::command]
async fn messages_mark_flagged(
    app: AppHandle,
    input: MarkMessageFlaggedInput,
) -> Result<MessageActionResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        messages_mark_flagged_blocking(state, input)
    })
    .await
    .map_err(|error| format!("Markierungs-Worker wurde unerwartet beendet: {error}"))?
}

fn messages_mark_flagged_blocking(
    state: State<'_, AppState>,
    input: MarkMessageFlaggedInput,
) -> Result<MessageActionResult, String> {
    let message_context = load_message_context(&state, &input.message_id)?;
    let account = load_account_connection(&state, &message_context.account_id)?;
    let config = connection_config(&account)?;
    let login_secret = load_login_secret(&state, &account)?;
    let imap_host = config
        .imap_host
        .as_deref()
        .ok_or_else(|| "IMAP-Host fehlt.".to_string())?;
    let imap_port = config
        .imap_port
        .ok_or_else(|| "IMAP-Port fehlt.".to_string())?;
    let mut session = login_imap_account(imap_host, imap_port, &account, &login_secret)?;
    session
        .select(&message_context.folder_path)
        .map_err(|error| format!("Ordner konnte nicht geöffnet werden: {error}"))?;
    let operation = if input.flagged {
        "+FLAGS.SILENT (\\Flagged)"
    } else {
        "-FLAGS.SILENT (\\Flagged)"
    };
    session
        .uid_store(&message_context.uid, operation)
        .map_err(|error| {
            format!("Markierung konnte serverseitig nicht geändert werden: {error}")
        })?;
    let _ = session.logout();

    update_local_message_flag(&state, &input.message_id, "\\Flagged", input.flagged)?;

    Ok(MessageActionResult {
        ok: true,
        summary: if input.flagged {
            "Nachricht als wichtig markiert.".into()
        } else {
            "Wichtig-Markierung entfernt.".into()
        },
    })
}

#[tauri::command]
fn sender_stats(
    state: State<'_, AppState>,
    input: SenderActionInput,
) -> Result<SenderStats, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let (account_id, sender, sender_email): (String, String, String) = db
        .query_row(
            "SELECT m.account_id, COALESCE(s.display_name, m.from_addr), m.from_addr
             FROM messages m
             LEFT JOIN senders s ON lower(s.normalized_email) = lower(m.from_addr)
             WHERE m.id = ?1",
            params![input.message_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Nachricht wurde nicht gefunden.".to_string())?;
    let sender_email_normalized = sender_email.trim().to_ascii_lowercase();
    let domain = sender_domain(&sender_email_normalized);
    let (message_count, first_message_at, last_message_at): (i64, Option<String>, Option<String>) =
        db.query_row(
            "SELECT COUNT(*),
                    MIN(datetime(COALESCE(date, created_at))),
                    MAX(datetime(COALESCE(date, created_at)))
             FROM messages
             WHERE account_id = ?1 AND lower(from_addr) = lower(?2)",
            params![&account_id, &sender_email_normalized],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| error.to_string())?;
    let newsletter = db
        .query_row(
            "SELECT COUNT(*)
             FROM newsletter_subscriptions ns
             JOIN senders s ON s.id = ns.sender_id
             WHERE ns.account_id = ?1 AND lower(s.normalized_email) = lower(?2)",
            params![&account_id, &sender_email_normalized],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| error.to_string())?
        > 0;
    let blocked = db
        .query_row(
            "SELECT COUNT(*) FROM blocked_senders WHERE account_id = ?1 AND lower(sender_email) = lower(?2)",
            params![&account_id, &sender_email_normalized],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| error.to_string())?
        > 0;
    let trusted = sender_is_trusted_locked(&db, &account_id, &sender_email_normalized)?;

    Ok(SenderStats {
        sender,
        sender_email: sender_email_normalized,
        domain,
        first_message_at,
        last_message_at,
        message_count: message_count.max(0) as u32,
        newsletter,
        trusted: trusted && !blocked,
        blocked,
    })
}

#[tauri::command]
fn sender_block(
    state: State<'_, AppState>,
    input: SenderActionInput,
) -> Result<MessageActionResult, String> {
    set_sender_blocked(state, input.message_id, true)
}

#[tauri::command]
fn sender_unblock(
    state: State<'_, AppState>,
    input: SenderActionInput,
) -> Result<MessageActionResult, String> {
    set_sender_blocked(state, input.message_id, false)
}

fn set_sender_blocked(
    state: State<'_, AppState>,
    message_id: String,
    blocked: bool,
) -> Result<MessageActionResult, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let (account_id, sender_email): (String, String) = db
        .query_row(
            "SELECT account_id, from_addr FROM messages WHERE id = ?1",
            params![message_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Nachricht wurde nicht gefunden.".to_string())?;
    let sender_email = sender_email.trim().to_ascii_lowercase();
    let domain = sender_domain(&sender_email);
    if blocked {
        set_sender_trusted_locked(&db, &account_id, &sender_email, false)?;
        db.execute(
            "INSERT INTO blocked_senders (id, account_id, sender_email, domain)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(account_id, sender_email) DO UPDATE SET domain = excluded.domain",
            params![
                format!("blocked-{}", uuid::Uuid::new_v4()),
                &account_id,
                &sender_email,
                &domain
            ],
        )
        .map_err(|error| error.to_string())?;
        let spam_folder = find_existing_folder_locked(&db, &account_id, &["spam", "junk"])
            .unwrap_or_else(|| "Spam".into());
        let spam_folder_id = upsert_folder(&db, &account_id, &spam_folder)?;
        db.execute(
            "UPDATE messages SET folder_id = ?1, updated_at = CURRENT_TIMESTAMP
             WHERE account_id = ?2 AND lower(from_addr) = lower(?3)",
            params![spam_folder_id, &account_id, &sender_email],
        )
        .map_err(|error| error.to_string())?;
        audit(
            &db,
            "sender.block",
            "sender",
            Some(&sender_email),
            serde_json::json!({ "domain": domain }),
        )?;
        Ok(MessageActionResult {
            ok: true,
            summary: "Absender lokal blockiert und bestehende Mails in Spam verschoben.".into(),
        })
    } else {
        db.execute(
            "DELETE FROM blocked_senders WHERE account_id = ?1 AND lower(sender_email) = lower(?2)",
            params![&account_id, &sender_email],
        )
        .map_err(|error| error.to_string())?;
        audit(
            &db,
            "sender.unblock",
            "sender",
            Some(&sender_email),
            serde_json::json!({ "domain": domain }),
        )?;
        Ok(MessageActionResult {
            ok: true,
            summary: "Blockierung für diesen Absender aufgehoben.".into(),
        })
    }
}

#[tauri::command]
fn smtp_send_email(
    state: State<'_, AppState>,
    input: SendEmailInput,
) -> Result<MessageActionResult, String> {
    let account = load_account_connection(&state, &input.account_id)?;
    let config = connection_config(&account)?;
    let login_secret = load_login_secret(&state, &account)?;
    let smtp_host = config
        .smtp_host
        .as_deref()
        .ok_or_else(|| "SMTP-Host fehlt.".to_string())?;
    let smtp_port = config
        .smtp_port
        .ok_or_else(|| "SMTP-Port fehlt.".to_string())?;

    if input.to.trim().is_empty() || input.subject.trim().is_empty() {
        return Err("Empfänger und Betreff sind erforderlich.".into());
    }

    let from: Mailbox = account
        .email
        .parse()
        .map_err(|error| format!("Absenderadresse ist ungültig: {error}"))?;
    let to: Mailbox = input
        .to
        .trim()
        .parse()
        .map_err(|error| format!("Empfängeradresse ist ungültig: {error}"))?;
    let email = Message::builder()
        .from(from)
        .to(to)
        .subject(input.subject.trim())
        .body(input.body)
        .map_err(|error| format!("E-Mail konnte nicht gebaut werden: {error}"))?;
    let credentials = Credentials::new(account.email.clone(), login_secret);
    let mut builder = if smtp_port == 465 {
        SmtpTransport::relay(smtp_host)
            .map_err(|error| {
                format!("SMTP TLS-Verbindung konnte nicht vorbereitet werden: {error}")
            })?
            .port(smtp_port)
    } else {
        SmtpTransport::starttls_relay(smtp_host)
            .map_err(|error| {
                format!("SMTP STARTTLS-Verbindung konnte nicht vorbereitet werden: {error}")
            })?
            .port(smtp_port)
    };
    builder = builder.credentials(credentials);
    if account.auth_type == "oauth" {
        builder = builder.authentication(vec![Mechanism::Xoauth2]);
    }
    let mailer = builder.build();

    mailer
        .send(&email)
        .map_err(|error| format!("SMTP-Versand fehlgeschlagen: {error}"))?;

    Ok(MessageActionResult {
        ok: true,
        summary: "E-Mail wurde per SMTP gesendet.".into(),
    })
}

#[tauri::command]
fn oauth_provider_config() -> Vec<OAuthProviderStatus> {
    vec![
        oauth_provider_status("gmail", "Gmail / Google"),
        oauth_provider_status("microsoft", "Outlook / Hotmail / Microsoft"),
    ]
}

#[tauri::command]
fn oauth_start(input: OAuthStartInput) -> Result<OAuthStartResult, String> {
    let provider = normalize_oauth_provider(&input.provider)?;
    let client_id = resolve_oauth_client_id(&provider, input.client_id.as_deref())?;
    let redirect_uri = input
        .redirect_uri
        .or_else(|| configured_oauth_redirect_uri(&provider))
        .unwrap_or_else(|| default_oauth_redirect_uri(53682));
    build_oauth_start(
        &provider,
        &client_id,
        input.email.unwrap_or_default(),
        redirect_uri,
    )
}

fn build_oauth_start(
    provider: &str,
    client_id: &str,
    login_hint: String,
    redirect_uri: String,
) -> Result<OAuthStartResult, String> {
    let state = format!("state-{}", uuid::Uuid::new_v4());
    let code_verifier = pkce_verifier();
    let code_challenge = pkce_challenge(&code_verifier);

    let (auth_endpoint, scopes) = match provider {
        "gmail" => (
            "https://accounts.google.com/o/oauth2/v2/auth",
            "https://mail.google.com/",
        ),
        "microsoft" => (
            "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
            "openid profile email offline_access https://outlook.office.com/IMAP.AccessAsUser.All https://outlook.office.com/SMTP.Send",
        ),
        _ => return Err("OAuth ist aktuell für Gmail und Microsoft/Outlook vorbereitet.".into()),
    };

    let mut query = vec![
        ("client_id", client_id.to_string()),
        ("redirect_uri", redirect_uri.clone()),
        ("response_type", "code".into()),
        ("scope", scopes.into()),
        ("state", state.clone()),
        ("code_challenge", code_challenge),
        ("code_challenge_method", "S256".into()),
    ];
    if provider == "gmail" {
        query.push(("access_type", "offline".into()));
        query.push(("prompt", "consent".into()));
    } else if provider == "microsoft" {
        query.push(("prompt", "select_account".into()));
    }
    if !login_hint.trim().is_empty() {
        query.push(("login_hint", login_hint));
    }

    let authorization_url = format!(
        "{}?{}",
        auth_endpoint,
        query
            .into_iter()
            .map(|(key, value)| format!("{}={}", url_component(key), url_component(&value)))
            .collect::<Vec<_>>()
            .join("&")
    );

    Ok(OAuthStartResult {
        ok: true,
        provider: provider.to_string(),
        authorization_url,
        state,
        code_verifier,
        redirect_uri,
        summary: "Anbieter-Login wurde vorbereitet. Nach der Bestätigung kehrt der Browser zu KizuMail zurück.".into(),
    })
}

#[tauri::command]
async fn oauth_connect_interactive(
    app: AppHandle,
    input: OAuthConnectInput,
) -> Result<OAuthCompleteResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let provider = normalize_oauth_provider(&input.provider)?;
        let email = input.email.trim().to_lowercase();
        if !email.contains('@') {
            return Err("Bitte zuerst die E-Mail-Adresse eingeben.".into());
        }
        let client_id = resolve_oauth_client_id(&provider, input.client_id.as_deref())?;
        let configured_redirect_uri = configured_oauth_redirect_uri(&provider);
        let redirect_override = input
            .redirect_uri
            .as_deref()
            .or(configured_redirect_uri.as_deref());
        let (listener, redirect_uri) = prepare_oauth_loopback(redirect_override)?;
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("OAuth-Rücksprung konnte nicht gestartet werden: {error}"))?;
        let start = build_oauth_start(&provider, &client_id, email.clone(), redirect_uri.clone())?;
        open::that(&start.authorization_url)
            .map_err(|error| format!("Browser konnte nicht geöffnet werden: {error}"))?;
        let code = wait_for_oauth_code(listener, &start.state)?;
        let token = exchange_oauth_code(
            &provider,
            &client_id,
            &code,
            &start.code_verifier,
            &redirect_uri,
        )?;
        create_oauth_account(
            &state,
            &provider,
            &email,
            input.display_name.as_deref(),
            token,
        )
    })
    .await
    .map_err(|error| format!("OAuth-Worker wurde unerwartet beendet: {error}"))?
}

#[tauri::command]
async fn oauth_complete(
    app: AppHandle,
    input: OAuthCompleteInput,
) -> Result<OAuthCompleteResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let provider = normalize_oauth_provider(&input.provider)?;
        let email = input.email.trim().to_lowercase();
        let client_id = input.client_id.trim();
        let code = input.code.trim();
        let code_verifier = input.code_verifier.trim();
        let redirect_uri = input.redirect_uri.trim();

        if !email.contains('@') {
            return Err("Bitte eine gültige E-Mail-Adresse eingeben.".into());
        }
        if client_id.is_empty()
            || code.is_empty()
            || code_verifier.is_empty()
            || redirect_uri.is_empty()
        {
            return Err(
                "OAuth-Code, Client-ID, Redirect-URI und Code-Verifier sind erforderlich.".into(),
            );
        }

        let token = exchange_oauth_code(&provider, client_id, code, code_verifier, redirect_uri)?;
        create_oauth_account(
            &state,
            &provider,
            &email,
            input.display_name.as_deref(),
            token,
        )
    })
    .await
    .map_err(|error| format!("OAuth-Worker wurde unerwartet beendet: {error}"))?
}

fn create_oauth_account(
    state: &State<'_, AppState>,
    provider: &str,
    email: &str,
    display_name: Option<&str>,
    token: StoredOAuthToken,
) -> Result<OAuthCompleteResult, String> {
    let provider_config = provider_config_for_oauth(provider, email)?;
    let display_name = display_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| email.split('@').next().unwrap_or("Konto").to_string());
    let existing_id = {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        db.query_row(
            "SELECT id FROM accounts WHERE email = ?1 AND provider = ?2",
            params![email, provider_config.provider],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
    };
    let account_existed = existing_id.is_some();
    let id = existing_id.unwrap_or_else(|| format!("acct-{}", uuid::Uuid::new_v4()));
    let secret_user = format!("{id}:oauth");
    let email_secret_user = email_oauth_secret_user(email);
    let secret_ref = format!("keyring://{KEYRING_SERVICE}/{secret_user}");
    let token_json = serde_json::to_string(&token).map_err(|error| error.to_string())?;

    store_password_entry(&secret_user, &token_json).map_err(|error| {
        format!(
            "Die Anmeldung war erfolgreich, aber der OAuth-Token konnte nicht sicher im Windows Credential Manager gespeichert werden. {error}"
        )
    })?;
    let credential_status = "OAuth token stored in OS credential storage.";
    if let Err(error) = store_password_entry(&email_secret_user, &token_json) {
        let _ = error;
    }
    cache_password(&state, &id, &token_json)?;
    cache_password(&state, &email_secret_user, &token_json)?;

    let db = state.db.lock().map_err(|error| error.to_string())?;
    if let Err(error) = db.execute(
        "INSERT INTO accounts (id, provider, email, display_name, auth_type, secret_ref, sync_enabled, imap_host, imap_port, smtp_host, smtp_port)
         VALUES (?1, ?2, ?3, ?4, 'oauth', ?5, 1, ?6, ?7, ?8, ?9)
         ON CONFLICT(email, provider) DO UPDATE SET
           display_name = excluded.display_name,
           auth_type = 'oauth',
           secret_ref = excluded.secret_ref,
           sync_enabled = 1,
           imap_host = excluded.imap_host,
           imap_port = excluded.imap_port,
           smtp_host = excluded.smtp_host,
           smtp_port = excluded.smtp_port,
           updated_at = CURRENT_TIMESTAMP",
        params![
            id,
            provider_config.provider,
            email,
            display_name,
            secret_ref,
            provider_config.imap_host,
            provider_config.imap_port,
            provider_config.smtp_host,
            provider_config.smtp_port
        ],
    ) {
        if !account_existed {
            let _ = delete_password_entry(&secret_user);
            let _ = delete_password_entry(&email_secret_user);
            if let Ok(mut cache) = state.password_cache.lock() {
                cache.remove(&id.trim().to_lowercase());
                cache.remove(&email_secret_user.trim().to_lowercase());
            }
        }
        return Err(error.to_string());
    }

    db.execute(
        "INSERT INTO audit_log (id, action, entity_type, entity_id, metadata_json)
         VALUES (?1, 'account.oauth.complete', 'account', ?2, ?3)",
        params![
            format!("audit-{}", uuid::Uuid::new_v4()),
            id,
            serde_json::json!({
                "summary": credential_status,
                "provider": provider_config.provider,
                "imapHost": provider_config.imap_host,
                "smtpHost": provider_config.smtp_host
            })
            .to_string()
        ],
    )
    .ok();

    let account = load_public_account(&db, &id)?;
    Ok(OAuthCompleteResult {
        ok: true,
        account,
        summary: "Konto verbunden und lokal aktualisiert. Der Sync startet im Hintergrund.".into(),
    })
}

#[tauri::command]
fn newsletter_list(state: State<'_, AppState>) -> Result<Vec<NewsletterSubscription>, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let mut statement = db
        .prepare(
            "SELECT ns.id,
                    COALESCE(s.display_name, s.normalized_email) AS sender,
                    s.normalized_email,
                    s.domain,
                    s.message_count,
                    s.last_seen_at,
                    ns.unsubscribe_type,
                    ns.confidence_score,
                    ns.risk_score,
                    CASE
                      WHEN ns.risk_score >= 70 THEN 'high'
                      WHEN ns.risk_score >= 35 THEN 'medium'
                      ELSE 'low'
                    END AS risk_level,
                    ns.evidence_json
             FROM newsletter_subscriptions ns
             JOIN senders s ON s.id = ns.sender_id
             JOIN accounts a ON a.id = ns.account_id
             WHERE a.email NOT LIKE '%@example.com'
             ORDER BY ns.risk_score ASC, s.message_count DESC",
        )
        .map_err(|error| error.to_string())?;

    let subscriptions = statement
        .query_map([], |row| {
            let evidence_json: String = row.get(10)?;
            let evidence = serde_json::from_str::<Vec<String>>(&evidence_json).unwrap_or_default();

            Ok(NewsletterSubscription {
                id: row.get(0)?,
                sender: row.get(1)?,
                sender_email: row.get(2)?,
                domain: row.get(3)?,
                message_count: row.get(4)?,
                last_message_at: row.get(5)?,
                unsubscribe_method: row.get(6)?,
                confidence_score: row.get(7)?,
                risk_score: row.get(8)?,
                risk_level: row.get(9)?,
                evidence,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    Ok(subscriptions)
}

#[tauri::command]
fn newsletter_unsubscribe_preview(
    state: State<'_, AppState>,
    input: NewsletterActionInput,
) -> Result<NewsletterActionResult, String> {
    let (_id, method, target) = load_unsubscribe_target(&state, &input.subscription_id)?;
    Ok(NewsletterActionResult {
        ok: true,
        method: method.clone(),
        target: target.clone(),
        summary: unsubscribe_preview_summary(&method, &target),
    })
}

#[tauri::command]
fn newsletter_unsubscribe_execute(
    state: State<'_, AppState>,
    input: NewsletterActionInput,
) -> Result<NewsletterActionResult, String> {
    let (subscription_id, method, target) =
        load_unsubscribe_target(&state, &input.subscription_id)?;
    if target.trim().is_empty() {
        return Err("Für diesen Newsletter wurde kein Abmeldeziel gefunden.".into());
    }

    let summary = match method.as_str() {
        "http_post" => {
            let target = validate_http_unsubscribe_target(&target)?;
            let response = unsubscribe_http_agent()
                .post(&target)
                .set("User-Agent", "KizuMail/0.1 unsubscribe")
                .set("Content-Type", "application/x-www-form-urlencoded")
                .send_string("List-Unsubscribe=One-Click")
                .map_err(|error| {
                    format!("Ein-Klick-Abmeldung wurde vom Anbieter abgelehnt: {error}")
                })?;
            format!(
                "Ein-Klick-Abmeldung gesendet. Serverstatus: {}.",
                response.status()
            )
        }
        "http_get" | "html_link" => {
            let target = validate_http_unsubscribe_target(&target)?;
            let response = unsubscribe_http_agent()
                .get(&target)
                .set("User-Agent", "KizuMail/0.1 unsubscribe")
                .call()
                .map_err(|error| format!("Abmelde-Link konnte nicht aufgerufen werden: {error}"))?;
            if !(200..300).contains(&response.status()) {
                return Err(format!(
                    "Abmelde-Link wurde mit HTTP {} beantwortet; die Abmeldung wurde nicht als erfolgreich gewertet.",
                    response.status()
                ));
            }
            format!(
                "Abmelde-Link wurde aufgerufen. Serverstatus: {}.",
                response.status()
            )
        }
        "mailto" => {
            let target = validate_mailto_unsubscribe_target(&target)?;
            open::that(&target).map_err(|error| {
                format!("Mailto-Abmeldung konnte nicht geöffnet werden: {error}")
            })?;
            "Mailto-Abmeldung wurde im Standard-Mailprogramm geöffnet.".into()
        }
        _ => {
            return Err(format!(
                "Unsicheres oder nicht unterstütztes Abmeldeverfahren '{method}'. Erwartet wird mailto oder http(s)."
            ));
        }
    };

    let db = state.db.lock().map_err(|error| error.to_string())?;
    db.execute(
        "UPDATE newsletter_subscriptions SET status = 'unsubscribe_requested', updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
        params![subscription_id],
    )
    .map_err(|error| error.to_string())?;
    db.execute(
        "INSERT INTO unsubscribe_actions (id, subscription_id, method, target, executed_at, result_status, result_summary)
         VALUES (?1, ?2, ?3, ?4, CURRENT_TIMESTAMP, 'requested', ?5)",
        params![
            format!("unsub-{}", uuid::Uuid::new_v4()),
            subscription_id,
            method,
            target,
            summary
        ],
    )
    .map_err(|error| error.to_string())?;

    Ok(NewsletterActionResult {
        ok: true,
        method: "executed".into(),
        target: String::new(),
        summary,
    })
}

fn load_unsubscribe_target(
    state: &State<'_, AppState>,
    subscription_id: &str,
) -> Result<(String, String, String), String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let (id, raw_method, raw_target): (String, String, Option<String>) = db
        .query_row(
            "SELECT id, unsubscribe_type, unsubscribe_target FROM newsletter_subscriptions WHERE id = ?1",
            params![subscription_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Newsletter wurde nicht gefunden.".to_string())?;
    let raw_target = raw_target.unwrap_or_default();
    let (method, target) = parse_unsubscribe_target(&raw_method, &raw_target);
    Ok((id, method, target))
}

fn unsubscribe_preview_summary(method: &str, target: &str) -> String {
    match method {
        "http_post" => {
            format!("KizuMail kann die standardisierte Ein-Klick-Abmeldung senden: {target}")
        }
        "http_get" | "html_link" => {
            format!("KizuMail kann diesen Abmelde-Link nach Bestätigung aufrufen: {target}")
        }
        "mailto" => format!("KizuMail öffnet eine vorbereitete Abmelde-E-Mail: {target}"),
        _ => format!("KizuMail öffnet dieses Abmeldeziel zur manuellen Prüfung: {target}"),
    }
}

fn unsubscribe_http_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(UNSUBSCRIBE_HTTP_TIMEOUT)
        .timeout_read(UNSUBSCRIBE_HTTP_TIMEOUT)
        .timeout_write(UNSUBSCRIBE_HTTP_TIMEOUT)
        .redirects(5)
        .build()
}

fn parse_unsubscribe_target(raw_method: &str, raw_target: &str) -> (String, String) {
    let candidates = raw_target
        .split(|character| character == ',' || character == '\n' || character == '\r')
        .map(|entry| {
            entry
                .trim()
                .trim_matches(|character| character == '<' || character == '>')
                .trim()
                .to_string()
        })
        .filter(|entry| !entry.is_empty())
        .collect::<Vec<_>>();
    for scheme in ["https://", "http://"] {
        if let Some(http) = candidates
            .iter()
            .find(|entry| starts_with_ascii_case_insensitive(entry, scheme))
        {
            return ("http_get".into(), canonicalize_scheme(http, scheme));
        }
    }
    if let Some(mailto) = candidates
        .iter()
        .find(|entry| starts_with_ascii_case_insensitive(entry, "mailto:"))
    {
        return ("mailto".into(), canonicalize_scheme(mailto, "mailto:"));
    }
    let method = raw_method.trim().to_ascii_lowercase();
    if method == "mailto" && raw_target.trim().contains('@') {
        return (method, format!("mailto:{}", raw_target.trim()));
    }
    (method, raw_target.trim().to_string())
}

fn starts_with_ascii_case_insensitive(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

fn canonicalize_scheme(value: &str, scheme: &str) -> String {
    format!("{}{}", scheme, &value[scheme.len()..])
}

fn validate_http_unsubscribe_target(target: &str) -> Result<String, String> {
    let target = target.trim();
    let scheme = if starts_with_ascii_case_insensitive(target, "https://") {
        "https://"
    } else if starts_with_ascii_case_insensitive(target, "http://") {
        "http://"
    } else {
        return Err(
            "Unsicheres Abmeldeziel: Es sind ausschließlich http:// oder https:// erlaubt.".into(),
        );
    };
    if target
        .chars()
        .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err("Abmelde-Link enthält ungültige Leer- oder Steuerzeichen.".into());
    }
    if target.contains('#') {
        return Err("Abmelde-Link enthält ein Fragment, das der Server nicht erhält.".into());
    }
    let authority = target[scheme.len()..]
        .split(['/', '?'])
        .next()
        .unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return Err(
            "Abmelde-Link enthält keinen gültigen Host oder verbotene Zugangsdaten.".into(),
        );
    }
    Ok(canonicalize_scheme(target, scheme))
}

fn validate_mailto_unsubscribe_target(target: &str) -> Result<String, String> {
    if !starts_with_ascii_case_insensitive(target.trim(), "mailto:") {
        return Err("Ungültiges mailto-Abmeldeziel: Das Ziel muss mit mailto: beginnen.".into());
    }
    let target = canonicalize_scheme(target.trim(), "mailto:");
    let address_part = target["mailto:".len()..]
        .split('?')
        .next()
        .unwrap_or_default();
    if address_part.is_empty() || address_part.starts_with("//") {
        return Err("Ungültiges mailto-Abmeldeziel: Empfänger fehlt.".into());
    }
    if address_part
        .chars()
        .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(
            "Ungültiges mailto-Abmeldeziel: Empfänger enthält Leer- oder Steuerzeichen.".into(),
        );
    }
    if !address_part.contains('@') && !address_part.to_ascii_lowercase().contains("%40") {
        return Err("Ungültiges mailto-Abmeldeziel: Empfänger-Adresse fehlt.".into());
    }
    if target.contains('%') {
        let bytes = target.as_bytes();
        for index in 0..bytes.len() {
            if bytes[index] == b'%'
                && (index + 2 >= bytes.len()
                    || !bytes[index + 1].is_ascii_hexdigit()
                    || !bytes[index + 2].is_ascii_hexdigit())
            {
                return Err(
                    "Ungültiges mailto-Abmeldeziel: Prozentkodierung ist fehlerhaft.".into(),
                );
            }
        }
    }
    Ok(target)
}

#[tauri::command]
fn audit_list(state: State<'_, AppState>) -> Result<Vec<AuditEvent>, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let mut statement = db
        .prepare(
            "SELECT id, action, COALESCE(entity_id, entity_type), json_extract(metadata_json, '$.summary'), created_at
             FROM audit_log
             ORDER BY created_at DESC
             LIMIT 50",
        )
        .map_err(|error| error.to_string())?;

    let events = statement
        .query_map([], |row| {
            Ok(AuditEvent {
                id: row.get(0)?,
                action: row.get(1)?,
                entity: row.get(2)?,
                summary: row
                    .get::<_, Option<String>>(3)?
                    .unwrap_or_else(|| "Local action recorded.".into()),
                created_at: row.get(4)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    Ok(events)
}

fn provider_config_for_email(email: &str) -> Option<ProviderConfig> {
    let domain = email.split('@').nth(1)?.to_ascii_lowercase();
    let config = match domain.as_str() {
        "hotmail.com" | "hotmail.de" | "outlook.com" | "outlook.de" | "live.com" | "live.de"
        | "msn.com" => ProviderConfig {
            provider: "microsoft",
            imap_host: "outlook.office365.com",
            imap_port: 993,
            smtp_host: "smtp-mail.outlook.com",
            smtp_port: 587,
        },
        "gmail.com" | "googlemail.com" => ProviderConfig {
            provider: "gmail",
            imap_host: "imap.gmail.com",
            imap_port: 993,
            smtp_host: "smtp.gmail.com",
            smtp_port: 587,
        },
        "icloud.com" | "me.com" | "mac.com" => ProviderConfig {
            provider: "imap",
            imap_host: "imap.mail.me.com",
            imap_port: 993,
            smtp_host: "smtp.mail.me.com",
            smtp_port: 587,
        },
        "yahoo.com" | "yahoo.de" => ProviderConfig {
            provider: "imap",
            imap_host: "imap.mail.yahoo.com",
            imap_port: 993,
            smtp_host: "smtp.mail.yahoo.com",
            smtp_port: 587,
        },
        "gmx.de" | "gmx.net" => ProviderConfig {
            provider: "imap",
            imap_host: "imap.gmx.net",
            imap_port: 993,
            smtp_host: "mail.gmx.net",
            smtp_port: 587,
        },
        "web.de" => ProviderConfig {
            provider: "imap",
            imap_host: "imap.web.de",
            imap_port: 993,
            smtp_host: "smtp.web.de",
            smtp_port: 587,
        },
        _ => return None,
    };

    Some(config)
}

fn load_account_connection(
    state: &State<'_, AppState>,
    account_id: &str,
) -> Result<AccountConnection, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    db.query_row(
        "SELECT id, provider, email, auth_type, secret_ref, imap_host, imap_port, smtp_host, smtp_port
         FROM accounts
         WHERE id = ?1",
        params![account_id],
        |row| {
            Ok(AccountConnection {
                id: row.get(0)?,
                provider: row.get(1)?,
                email: row.get(2)?,
                auth_type: row.get(3)?,
                secret_ref: row.get(4)?,
                imap_host: row.get(5)?,
                imap_port: row.get::<_, Option<i64>>(6)?.map(|value| value as u16),
                smtp_host: row.get(7)?,
                smtp_port: row.get::<_, Option<i64>>(8)?.map(|value| value as u16),
            })
        },
    )
    .optional()
    .map_err(|error| error.to_string())?
    .ok_or_else(|| "Konto wurde nicht gefunden.".to_string())
}

fn load_account_connections(state: &State<'_, AppState>) -> Result<Vec<AccountConnection>, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let mut statement = db
        .prepare(
            "SELECT id, provider, email, auth_type, secret_ref, imap_host, imap_port, smtp_host, smtp_port
             FROM accounts
             WHERE email NOT LIKE '%@example.com'
             ORDER BY created_at ASC",
        )
        .map_err(|error| error.to_string())?;
    let accounts = statement
        .query_map([], |row| {
            Ok(AccountConnection {
                id: row.get(0)?,
                provider: row.get(1)?,
                email: row.get(2)?,
                auth_type: row.get(3)?,
                secret_ref: row.get(4)?,
                imap_host: row.get(5)?,
                imap_port: row.get::<_, Option<i64>>(6)?.map(|value| value as u16),
                smtp_host: row.get(7)?,
                smtp_port: row.get::<_, Option<i64>>(8)?.map(|value| value as u16),
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(accounts)
}

fn load_public_account(db: &Connection, account_id: &str) -> Result<Account, String> {
    db.query_row(
        "SELECT id, provider, email, display_name, auth_type, sync_enabled, last_sync_at
         FROM accounts
         WHERE id = ?1",
        params![account_id],
        |row| {
            Ok(Account {
                id: row.get(0)?,
                provider: row.get(1)?,
                email: row.get(2)?,
                display_name: row.get(3)?,
                auth_type: row.get(4)?,
                sync_enabled: row.get::<_, i64>(5)? == 1,
                last_sync_at: row.get(6)?,
                credential_status: "unknown".into(),
                auth_status: "connected".into(),
            })
        },
    )
    .optional()
    .map_err(|error| error.to_string())?
    .ok_or_else(|| "Konto wurde nicht gefunden.".to_string())
}

fn load_message_context(
    state: &State<'_, AppState>,
    message_id: &str,
) -> Result<MessageContext, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    db.query_row(
        "SELECT m.account_id, f.path, m.uid, m.from_addr
         FROM messages m
         JOIN folders f ON f.id = m.folder_id
         WHERE m.id = ?1",
        params![message_id],
        |row| {
            Ok(MessageContext {
                account_id: row.get(0)?,
                folder_path: row.get(1)?,
                uid: row.get(2)?,
                sender_email: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(|error| error.to_string())?
    .ok_or_else(|| "Nachricht wurde nicht gefunden.".to_string())
}

fn find_existing_folder(
    state: &State<'_, AppState>,
    account_id: &str,
    needles: &[&str],
) -> Option<String> {
    let db = state.db.lock().ok()?;
    let mut statement = db
        .prepare("SELECT path FROM folders WHERE account_id = ?1")
        .ok()?;
    let folders = statement
        .query_map(params![account_id], |row| row.get::<_, String>(0))
        .ok()?
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    folders.into_iter().find(|folder| {
        let normalized = folder.to_ascii_lowercase();
        needles.iter().any(|needle| normalized.contains(needle))
    })
}

fn find_existing_folder_locked(
    db: &Connection,
    account_id: &str,
    needles: &[&str],
) -> Option<String> {
    let mut statement = db
        .prepare("SELECT path FROM folders WHERE account_id = ?1")
        .ok()?;
    let folders = statement
        .query_map(params![account_id], |row| row.get::<_, String>(0))
        .ok()?
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    folders.into_iter().find(|folder| {
        let normalized = folder.to_ascii_lowercase();
        needles.iter().any(|needle| normalized.contains(needle))
    })
}

fn connection_config(account: &AccountConnection) -> Result<AccountConnection, String> {
    let provider_config = provider_config_for_email(&account.email);
    let imap_host = account
        .imap_host
        .as_ref()
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .or_else(|| {
            provider_config
                .as_ref()
                .map(|config| config.imap_host.to_string())
        });
    let smtp_host = account
        .smtp_host
        .as_ref()
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .or_else(|| {
            provider_config
                .as_ref()
                .map(|config| config.smtp_host.to_string())
        });

    Ok(AccountConnection {
        id: account.id.clone(),
        provider: account.provider.clone(),
        email: account.email.clone(),
        auth_type: account.auth_type.clone(),
        secret_ref: account.secret_ref.clone(),
        imap_host,
        imap_port: account
            .imap_port
            .or_else(|| provider_config.as_ref().map(|config| config.imap_port)),
        smtp_host,
        smtp_port: account
            .smtp_port
            .or_else(|| provider_config.as_ref().map(|config| config.smtp_port)),
    })
    .and_then(|config| {
        if config.imap_host.is_none() || config.imap_port.is_none() {
            Err("Für dieses Konto fehlen IMAP-Serverdaten. Bitte Konto neu verbinden oder erweiterte Daten eintragen.".into())
        } else {
            Ok(config)
        }
    })
}

fn load_login_secret(
    state: &State<'_, AppState>,
    account: &AccountConnection,
) -> Result<String, String> {
    let secret = load_password(state, account)?;
    if account.auth_type == "oauth" {
        return load_or_refresh_oauth_access_token(state, account, &secret);
    }
    Ok(secret)
}

fn load_password(
    state: &State<'_, AppState>,
    account: &AccountConnection,
) -> Result<String, String> {
    let candidates = credential_candidates(account);
    let mut last_error = None;
    for secret_user in candidates {
        for service in KEYRING_SERVICES {
            match load_password_entry(service, &secret_user) {
                Ok(password) => {
                    let _ = store_password_for_account(account, &password);
                    let _ = cache_password(state, &account.id, &password);
                    let _ = cache_password(state, &account.email, &password);
                    if account.auth_type == "oauth" {
                        let _ = cache_password(
                            state,
                            &email_oauth_secret_user(&account.email),
                            &password,
                        );
                    }
                    return Ok(password);
                }
                Err(error) => last_error = Some(error),
            }
        }
    }

    if let Some(password) = cached_password(state, &account.id)? {
        let _ = store_password_for_account(account, &password);
        return Ok(password);
    }

    if let Some(password) = cached_password(state, &account.email)? {
        let _ = store_password_for_account(account, &password);
        return Ok(password);
    }

    if let Some(password) = cached_password(state, &email_oauth_secret_user(&account.email))? {
        let _ = store_password_for_account(account, &password);
        return Ok(password);
    }

    Err(format!(
        "Anmeldedaten fehlen im Windows Credential Manager. Bitte {} einmal neu verbinden. Details: {}",
        account.email,
        last_error.unwrap_or_else(|| "kein Credential-Key gefunden".into())
    ))
}

fn credential_candidates(account: &AccountConnection) -> Vec<String> {
    let mut candidates = Vec::new();
    if let Some(secret_user) = account
        .secret_ref
        .as_ref()
        .and_then(|secret_ref| secret_ref.rsplit('/').next())
        .map(str::to_string)
    {
        candidates.push(secret_user);
    }
    candidates.push(secret_user_for_account(account));
    candidates.push(format!("{}:imap", account.id));
    candidates.push(format!("{}:oauth", account.id));
    candidates.push(email_secret_user(&account.email));
    candidates.push(email_oauth_secret_user(&account.email));
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|candidate| seen.insert(candidate.clone()))
        .collect()
}

fn credential_status_for_account(
    state: &State<'_, AppState>,
    account: &AccountConnection,
) -> String {
    for secret_user in credential_candidates(account) {
        for service in KEYRING_SERVICES {
            if load_password_entry(service, &secret_user).is_ok() {
                return "ok".into();
            }
        }
    }
    if cached_password(state, &account.id).ok().flatten().is_some()
        || cached_password(state, &account.email)
            .ok()
            .flatten()
            .is_some()
        || cached_password(state, &email_oauth_secret_user(&account.email))
            .ok()
            .flatten()
            .is_some()
    {
        return "session_only".into();
    }
    "missing".into()
}

fn auth_status_from_credential(auth_type: &str, credential_status: &str) -> String {
    match credential_status {
        "ok" => "connected".into(),
        "session_only" => "repairable".into(),
        _ if auth_type == "oauth" => "needs_reconnect".into(),
        _ => "needs_password".into(),
    }
}

fn load_or_refresh_oauth_access_token(
    state: &State<'_, AppState>,
    account: &AccountConnection,
    secret: &str,
) -> Result<String, String> {
    let mut token: StoredOAuthToken = serde_json::from_str(secret)
        .map_err(|_| "OAuth-Token ist beschädigt. Bitte Konto neu verbinden.".to_string())?;
    let now = current_unix_timestamp();
    if token.expires_at > now + 60 && !token.access_token.trim().is_empty() {
        return Ok(token.access_token);
    }

    let refresh_token = token
        .refresh_token
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "OAuth-Refresh-Token fehlt. Bitte Konto neu verbinden.".to_string())?;
    let refreshed = refresh_oauth_token(
        &token.provider,
        &token.client_id,
        &token.token_endpoint,
        &refresh_token,
    )?;
    token.access_token = refreshed.access_token;
    token.expires_at = refreshed.expires_at;
    if refreshed.refresh_token.is_some() {
        token.refresh_token = refreshed.refresh_token;
    }
    let token_json = serde_json::to_string(&token).map_err(|error| error.to_string())?;
    store_password_for_account(account, &token_json).map_err(|error| {
        format!(
            "Der erneuerte OAuth-Token konnte nicht sicher gespeichert werden. Bitte Konto erneut verbinden. {error}"
        )
    })?;
    cache_password(state, &account.id, &token_json)?;
    cache_password(state, &email_oauth_secret_user(&account.email), &token_json)?;
    Ok(token.access_token)
}

fn exchange_oauth_code(
    provider: &str,
    client_id: &str,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<StoredOAuthToken, String> {
    let token_endpoint = oauth_token_endpoint(provider)?;
    let client_secret = configured_oauth_client_secret(provider);
    let mut fields = vec![
        ("client_id", client_id),
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("code_verifier", code_verifier),
    ];
    if let Some(secret) = client_secret.as_deref() {
        fields.push(("client_secret", secret));
    }
    let response = send_oauth_form(token_endpoint, &fields, "OAuth-Token-Austausch")?;
    parse_oauth_token_response(provider, client_id, token_endpoint, response)
}

fn refresh_oauth_token(
    provider: &str,
    client_id: &str,
    token_endpoint: &str,
    refresh_token: &str,
) -> Result<StoredOAuthToken, String> {
    let client_secret = configured_oauth_client_secret(provider);
    let mut fields = vec![
        ("client_id", client_id),
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
    ];
    if let Some(secret) = client_secret.as_deref() {
        fields.push(("client_secret", secret));
    }
    let response = send_oauth_form(token_endpoint, &fields, "OAuth-Token-Refresh")?;
    parse_oauth_token_response(provider, client_id, token_endpoint, response)
}

fn send_oauth_form(
    token_endpoint: &str,
    fields: &[(&str, &str)],
    label: &str,
) -> Result<ureq::Response, String> {
    match ureq::post(token_endpoint)
        .set("Accept", "application/json")
        .send_form(fields)
    {
        Ok(response) => Ok(response),
        Err(ureq::Error::Status(code, response)) => {
            let body = response.into_string().unwrap_or_default();
            let detail = oauth_error_detail(&body);
            Err(format!("{label} fehlgeschlagen: HTTP {code}. {detail}")
                .trim()
                .to_string())
        }
        Err(error) => Err(format!("{label} fehlgeschlagen: {error}")),
    }
}

fn oauth_error_detail(body: &str) -> String {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
        let error = json
            .get("error")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let description = json
            .get("error_description")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let detail = format!("{error} {description}").trim().to_string();
        if !detail.is_empty() {
            return detail;
        }
    }
    if body.trim().is_empty() {
        "Der Anbieter hat keine Fehlerdetails gesendet.".into()
    } else {
        body.trim().to_string()
    }
}

fn parse_oauth_token_response(
    provider: &str,
    client_id: &str,
    token_endpoint: &str,
    response: ureq::Response,
) -> Result<StoredOAuthToken, String> {
    let json = response
        .into_json::<serde_json::Value>()
        .map_err(|error| format!("OAuth-Antwort konnte nicht gelesen werden: {error}"))?;
    if let Some(error) = json.get("error").and_then(|value| value.as_str()) {
        let description = json
            .get("error_description")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        return Err(
            format!("OAuth-Anbieter meldet Fehler: {error} {description}")
                .trim()
                .to_string(),
        );
    }
    let access_token = json
        .get("access_token")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "OAuth-Antwort enthielt keinen Access Token.".to_string())?
        .to_string();
    let refresh_token = json
        .get("refresh_token")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let expires_in = json
        .get("expires_in")
        .and_then(|value| value.as_i64())
        .unwrap_or(3600);
    Ok(StoredOAuthToken {
        provider: provider.to_string(),
        client_id: client_id.to_string(),
        token_endpoint: token_endpoint.to_string(),
        access_token,
        refresh_token,
        expires_at: current_unix_timestamp() + expires_in,
    })
}

fn normalize_oauth_provider(provider: &str) -> Result<String, String> {
    match provider.trim().to_lowercase().as_str() {
        "gmail" | "google" => Ok("gmail".into()),
        "microsoft" | "outlook" | "hotmail" => Ok("microsoft".into()),
        _ => Err("OAuth ist aktuell für Gmail und Microsoft/Outlook vorbereitet.".into()),
    }
}

fn oauth_token_endpoint(provider: &str) -> Result<&'static str, String> {
    match normalize_oauth_provider(provider)?.as_str() {
        "gmail" => Ok("https://oauth2.googleapis.com/token"),
        "microsoft" => Ok("https://login.microsoftonline.com/common/oauth2/v2.0/token"),
        _ => Err("OAuth-Anbieter wird nicht unterstützt.".into()),
    }
}

fn provider_config_for_oauth(provider: &str, email: &str) -> Result<ProviderConfig, String> {
    match normalize_oauth_provider(provider)?.as_str() {
        "gmail" => Ok(ProviderConfig {
            provider: "gmail",
            imap_host: "imap.gmail.com",
            imap_port: 993,
            smtp_host: "smtp.gmail.com",
            smtp_port: 587,
        }),
        "microsoft" => Ok(ProviderConfig {
            provider: provider_config_for_email(email)
                .as_ref()
                .map(|config| config.provider)
                .unwrap_or("microsoft"),
            imap_host: "outlook.office365.com",
            imap_port: 993,
            smtp_host: "smtp.office365.com",
            smtp_port: 587,
        }),
        _ => Err("OAuth-Anbieter wird nicht unterstützt.".into()),
    }
}

fn oauth_provider_status(provider: &str, label: &str) -> OAuthProviderStatus {
    let configured = configured_oauth_client_id(provider).is_some();
    let requires_own_registration = provider == "microsoft" && !configured;
    OAuthProviderStatus {
        provider: provider.to_string(),
        label: label.to_string(),
        client_id_configured: configured,
        requires_own_registration,
        end_user_ready: configured,
        summary: if configured {
            "Direkte Browser-Anmeldung ist vorbereitet.".into()
        } else if provider == "microsoft" {
            "Microsoft verlangt für Outlook/Hotmail OAuth eine App-Registrierung. Ohne Azure/Developer-Registrierung bleibt dieser Login vorbereitet, aber gesperrt.".into()
        } else {
            "In dieser Build-Version fehlt noch die öffentliche Anbieter-Registrierung.".into()
        },
    }
}

fn resolve_oauth_client_id(
    provider: &str,
    override_client_id: Option<&str>,
) -> Result<String, String> {
    if let Some(client_id) = override_client_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(client_id.to_string());
    }
    configured_oauth_client_id(provider).ok_or_else(|| {
        match provider {
            "gmail" => "Gmail-Login ist in dieser KizuMail-Build-Version noch nicht registriert. Nutze vorerst ein App-Passwort oder trage in den Entwickleroptionen eine Google Client-ID ein.".into(),
            "microsoft" => "Outlook/Hotmail-Login ist in dieser KizuMail-Build-Version noch nicht registriert. Nutze vorerst App-Passwort/IMAP, falls dein Konto es erlaubt, oder trage in den Entwickleroptionen eine Microsoft Client-ID ein.".into(),
            _ => "Dieser OAuth-Anbieter ist nicht registriert.".into(),
        }
    })
}

fn configured_oauth_client_id(provider: &str) -> Option<String> {
    let keys = match provider {
        "gmail" => ["KIZUMAIL_GMAIL_CLIENT_ID", "VITE_KIZUMAIL_GMAIL_CLIENT_ID"],
        "microsoft" => [
            "KIZUMAIL_MICROSOFT_CLIENT_ID",
            "VITE_KIZUMAIL_MICROSOFT_CLIENT_ID",
        ],
        _ => return None,
    };

    for key in keys {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }

    let compile_time = match provider {
        "gmail" => option_env!("KIZUMAIL_GMAIL_CLIENT_ID"),
        "microsoft" => option_env!("KIZUMAIL_MICROSOFT_CLIENT_ID"),
        _ => None,
    };
    compile_time
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| configured_oauth_client_id_from_file(provider))
}

fn configured_oauth_client_id_from_file(provider: &str) -> Option<String> {
    for path in oauth_client_config_paths() {
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        if let Some(value) = json.get(provider).and_then(|value| value.as_str()) {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
        if let Some(value) = json
            .get(provider)
            .and_then(|value| value.get("clientId"))
            .and_then(|value| value.as_str())
        {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
        if provider == "gmail" {
            if let Some(value) = json
                .get("installed")
                .or_else(|| json.get("web"))
                .and_then(|value| value.get("client_id"))
                .and_then(|value| value.as_str())
            {
                let value = value.trim();
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

fn configured_oauth_redirect_uri(provider: &str) -> Option<String> {
    for path in oauth_client_config_paths() {
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        let Some(value) = json
            .get(provider)
            .and_then(|value| {
                value
                    .get("redirectUri")
                    .or_else(|| value.get("redirect_uri"))
            })
            .and_then(|value| value.as_str())
        else {
            continue;
        };
        let value = value.trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

fn configured_oauth_client_secret(provider: &str) -> Option<String> {
    if provider == "microsoft" {
        return None;
    }
    let keys = match provider {
        "gmail" => [
            "KIZUMAIL_GMAIL_CLIENT_SECRET",
            "VITE_KIZUMAIL_GMAIL_CLIENT_SECRET",
        ],
        _ => return None,
    };

    for key in keys {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }

    for path in oauth_client_config_paths() {
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        if let Some(value) = json
            .get(provider)
            .and_then(|value| value.get("clientSecret"))
            .and_then(|value| value.as_str())
        {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
        if provider == "gmail" {
            if let Some(value) = json
                .get("installed")
                .or_else(|| json.get("web"))
                .and_then(|value| value.get("client_secret"))
                .and_then(|value| value.as_str())
            {
                let value = value.trim();
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

fn oauth_client_config_paths() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    if let Some(resource_dir) = OAUTH_RESOURCE_DIR.get() {
        paths.push(
            resource_dir
                .join("resources")
                .join("oauth-clients.local.json"),
        );
        paths.push(resource_dir.join("oauth-clients.local.json"));
    }
    if let Ok(current_dir) = std::env::current_dir() {
        paths.push(current_dir.join("oauth-clients.local.json"));
    }
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            paths.push(exe_dir.join("oauth-clients.local.json"));
        }
    }
    paths
}

fn default_oauth_redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}/oauth/callback")
}

fn prepare_oauth_loopback(
    override_redirect_uri: Option<&str>,
) -> Result<(TcpListener, String), String> {
    if let Some(redirect_uri) = override_redirect_uri
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let (host, port) = parse_loopback_redirect(redirect_uri)?;
        let listener = TcpListener::bind((host.as_str(), port)).map_err(|error| {
            format!("OAuth-Rücksprung konnte auf {host}:{port} nicht gestartet werden: {error}")
        })?;
        return Ok((listener, redirect_uri.to_string()));
    }

    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| {
        format!("OAuth-Rücksprung konnte lokal nicht vorbereitet werden: {error}")
    })?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("OAuth-Rücksprung-Port konnte nicht gelesen werden: {error}"))?
        .port();
    Ok((listener, default_oauth_redirect_uri(port)))
}

fn parse_loopback_redirect(redirect_uri: &str) -> Result<(String, u16), String> {
    let lower = redirect_uri.to_lowercase();
    if !lower.starts_with("http://") {
        return Err("OAuth-Redirect muss ein lokaler http:// Loopback-Redirect sein.".into());
    }

    let without_scheme = &redirect_uri["http://".len()..];
    let authority = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    let mut parts = authority.rsplitn(2, ':');
    let port = parts
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| "OAuth-Redirect braucht einen lokalen Port, zum Beispiel http://127.0.0.1:53682/oauth/callback.".to_string())?;
    let host = parts
        .next()
        .map(str::to_lowercase)
        .ok_or_else(|| "OAuth-Redirect braucht einen lokalen Host.".to_string())?;

    match host.as_str() {
        "127.0.0.1" | "localhost" => Ok((host, port)),
        _ => Err("OAuth-Redirect darf nur 127.0.0.1 oder localhost verwenden.".into()),
    }
}

fn wait_for_oauth_code(listener: TcpListener, expected_state: &str) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(180);
    while Instant::now() < deadline {
        match listener.accept() {
            Ok((mut stream, _addr)) => {
                let mut buffer = [0_u8; 4096];
                let read = stream.read(&mut buffer).map_err(|error| {
                    format!("OAuth-Rücksprung konnte nicht gelesen werden: {error}")
                })?;
                let request = String::from_utf8_lossy(&buffer[..read]);
                let first_line = request.lines().next().unwrap_or_default();
                let mut request_parts = first_line.split_whitespace();
                let method = request_parts.next().unwrap_or_default();
                let path = request_parts.next().unwrap_or_default();
                let callback_path = path.split('?').next().unwrap_or_default();
                if method != "GET" || callback_path != "/oauth/callback" {
                    let html = "<html><body><h2>Nicht gefunden</h2></body></html>";
                    let response = format!(
                        "HTTP/1.1 404 Not Found\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        html.len(),
                        html
                    );
                    let _ = stream.write_all(response.as_bytes());
                    continue;
                }
                let query = path.split_once('?').map(|(_, query)| query).unwrap_or("");
                let code = query_param(query, "code");
                let state = query_param(query, "state");
                let error = query_param(query, "error");
                let error_description = query_param(query, "error_description");
                let html = if code.is_some() && state.as_deref() == Some(expected_state) {
                    "<html><body><h2>KizuMail verbunden</h2><p>Du kannst dieses Fenster schließen.</p></body></html>"
                } else {
                    "<html><body><h2>KizuMail Anmeldung nicht abgeschlossen</h2><p>Bitte zurück zur App wechseln.</p></body></html>"
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    html.as_bytes().len(),
                    html
                );
                let _ = stream.write_all(response.as_bytes());

                if let Some(error) = error {
                    return Err(format!(
                        "Anbieter hat die Anmeldung abgebrochen: {}",
                        error_description
                            .filter(|value| !value.trim().is_empty())
                            .unwrap_or(error)
                    ));
                }
                if state.as_deref() != Some(expected_state) {
                    return Err("OAuth-Rücksprung hatte einen ungültigen Sicherheitsstatus.".into());
                }
                return code.ok_or_else(|| "OAuth-Rücksprung enthielt keinen Code.".to_string());
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(200));
            }
            Err(error) => return Err(format!("OAuth-Rücksprung fehlgeschlagen: {error}")),
        }
    }
    Err("OAuth-Anmeldung wurde nicht rechtzeitig abgeschlossen.".into())
}

fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|part| {
        let (candidate_key, value) = part.split_once('=')?;
        if candidate_key == key {
            Some(decode_url_component(value))
        } else {
            None
        }
    })
}

fn decode_url_component(value: &str) -> String {
    let mut output = Vec::new();
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                output.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = &value[index + 1..index + 3];
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    output.push(byte);
                    index += 3;
                } else {
                    output.push(bytes[index]);
                    index += 1;
                }
            }
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

fn cache_password(state: &State<'_, AppState>, key: &str, password: &str) -> Result<(), String> {
    let mut cache = state
        .password_cache
        .lock()
        .map_err(|error| error.to_string())?;
    cache.insert(key.trim().to_lowercase(), password.to_string());
    Ok(())
}

fn cached_password(state: &State<'_, AppState>, key: &str) -> Result<Option<String>, String> {
    let cache = state
        .password_cache
        .lock()
        .map_err(|error| error.to_string())?;
    Ok(cache.get(&key.trim().to_lowercase()).cloned())
}

fn store_password_for_account(account: &AccountConnection, password: &str) -> Result<(), String> {
    let secret_user = account
        .secret_ref
        .as_ref()
        .and_then(|secret_ref| secret_ref.rsplit('/').next())
        .map(str::to_string)
        .unwrap_or_else(|| secret_user_for_account(account));
    store_password_entry(&secret_user, password)?;
    if account.auth_type == "oauth" {
        let _ = store_password_entry(&email_oauth_secret_user(&account.email), password);
    } else {
        let _ = store_password_alias(&account.email, password);
    }
    Ok(())
}

fn store_password_alias(email: &str, password: &str) -> Result<(), String> {
    store_password_entry(&email_secret_user(email), password)
}

fn store_password_entry(secret_user: &str, password: &str) -> Result<(), String> {
    let mut stored = false;
    let mut last_error = None;
    for service in KEYRING_SERVICES {
        match store_password_entry_for_service(service, secret_user, password) {
            Ok(()) => stored = true,
            Err(error) => last_error = Some(error),
        }
    }
    if stored {
        Ok(())
    } else {
        Err(format!(
            "Anmeldedaten konnten nicht im Credential Manager gespeichert werden: {}",
            last_error.unwrap_or_else(|| "kein Credential-Service erreichbar".into())
        ))
    }
}

fn store_password_entry_for_service(
    service: &str,
    secret_user: &str,
    password: &str,
) -> Result<(), String> {
    let entry = keyring::Entry::new(service, secret_user)
        .map_err(|error| format!("Credential-Store konnte nicht geöffnet werden: {error}"))?;
    let previous_chunk_count = entry
        .get_password()
        .ok()
        .and_then(|value| credential_chunk_count(&value).ok().flatten())
        .unwrap_or(0);

    if credential_fits_single_entry(password) {
        entry
            .set_password(password)
            .map_err(|error| error.to_string())?;
        delete_password_chunks(service, secret_user, previous_chunk_count);
        return Ok(());
    }

    let chunks = split_credential_value(password);
    if chunks.is_empty() || chunks.len() > KEYRING_CHUNK_MAX_COUNT {
        return Err("OAuth-Token ist zu groß für die sichere lokale Speicherung.".into());
    }

    let mut stored_chunks = 0;
    for (index, chunk) in chunks.iter().enumerate() {
        let chunk_user = credential_chunk_user(secret_user, index);
        let chunk_entry = keyring::Entry::new(service, &chunk_user)
            .map_err(|error| format!("Credential-Teil konnte nicht geöffnet werden: {error}"))?;
        if let Err(error) = chunk_entry.set_password(chunk) {
            delete_password_chunks(service, secret_user, stored_chunks);
            return Err(format!(
                "Credential-Teil konnte nicht gespeichert werden: {error}"
            ));
        }
        stored_chunks += 1;
    }

    let manifest = format!("{KEYRING_CHUNK_MANIFEST_PREFIX}{}", chunks.len());
    if let Err(error) = entry.set_password(&manifest) {
        delete_password_chunks(service, secret_user, stored_chunks);
        return Err(format!(
            "Credential-Manifest konnte nicht gespeichert werden: {error}"
        ));
    }
    if previous_chunk_count > chunks.len() {
        for index in chunks.len()..previous_chunk_count {
            if let Ok(chunk_entry) =
                keyring::Entry::new(service, &credential_chunk_user(secret_user, index))
            {
                let _ = chunk_entry.delete_credential();
            }
        }
    }
    Ok(())
}

fn load_password_entry(service: &str, secret_user: &str) -> Result<String, String> {
    let entry = keyring::Entry::new(service, secret_user)
        .map_err(|error| format!("Credential-Store konnte nicht geöffnet werden: {error}"))?;
    let value = entry.get_password().map_err(|error| error.to_string())?;
    let Some(chunk_count) = credential_chunk_count(&value)? else {
        return Ok(value);
    };

    let mut password = String::new();
    for index in 0..chunk_count {
        let chunk_user = credential_chunk_user(secret_user, index);
        let chunk_entry = keyring::Entry::new(service, &chunk_user)
            .map_err(|error| format!("Credential-Teil konnte nicht geöffnet werden: {error}"))?;
        let chunk = chunk_entry
            .get_password()
            .map_err(|error| format!("Credential-Teil {index} fehlt: {error}"))?;
        password.push_str(&chunk);
    }
    Ok(password)
}

fn credential_chunk_count(value: &str) -> Result<Option<usize>, String> {
    let Some(raw_count) = value.strip_prefix(KEYRING_CHUNK_MANIFEST_PREFIX) else {
        return Ok(None);
    };
    let count = raw_count
        .parse::<usize>()
        .map_err(|_| "Credential-Manifest ist beschädigt.".to_string())?;
    if count == 0 || count > KEYRING_CHUNK_MAX_COUNT {
        return Err("Credential-Manifest enthält eine ungültige Teilanzahl.".into());
    }
    Ok(Some(count))
}

fn credential_utf16_byte_len(value: &str) -> usize {
    value.encode_utf16().count().saturating_mul(2)
}

fn credential_fits_single_entry(value: &str) -> bool {
    credential_utf16_byte_len(value) <= KEYRING_WINDOWS_BLOB_BYTE_LIMIT
}

fn split_credential_value(value: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_utf16_len = 0;
    for character in value.chars() {
        let character_len = character.len_utf16();
        if current_utf16_len + character_len > KEYRING_CHUNK_UTF16_LIMIT && !current.is_empty() {
            chunks.push(current);
            current = String::new();
            current_utf16_len = 0;
        }
        current.push(character);
        current_utf16_len += character_len;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn credential_chunk_user(secret_user: &str, index: usize) -> String {
    format!("{secret_user}:chunk:{index}")
}

fn delete_password_chunks(service: &str, secret_user: &str, count: usize) {
    for index in 0..count.min(KEYRING_CHUNK_MAX_COUNT) {
        if let Ok(entry) = keyring::Entry::new(service, &credential_chunk_user(secret_user, index))
        {
            let _ = entry.delete_credential();
        }
    }
}

fn email_secret_user(email: &str) -> String {
    format!("{}:imap", email.trim().to_lowercase())
}

fn email_oauth_secret_user(email: &str) -> String {
    format!("{}:oauth", email.trim().to_lowercase())
}

fn secret_user_for_account(account: &AccountConnection) -> String {
    if account.auth_type == "oauth" {
        format!("{}:oauth", account.id)
    } else {
        format!("{}:imap", account.id)
    }
}

fn delete_password_entries(account: &AccountConnection) {
    let mut candidates = Vec::new();
    if let Some(secret_user) = account
        .secret_ref
        .as_ref()
        .and_then(|secret_ref| secret_ref.rsplit('/').next())
        .map(str::to_string)
    {
        candidates.push(secret_user);
    }
    candidates.push(format!("{}:imap", account.id));
    candidates.push(format!("{}:oauth", account.id));
    candidates.push(email_secret_user(&account.email));
    candidates.push(email_oauth_secret_user(&account.email));

    for candidate in candidates {
        let _ = delete_password_entry(&candidate);
    }
}

fn delete_password_entry(secret_user: &str) -> Result<(), String> {
    let mut deleted = false;
    let mut last_error = None;
    for service in KEYRING_SERVICES {
        match keyring::Entry::new(service, secret_user) {
            Ok(entry) => {
                let chunk_count = entry
                    .get_password()
                    .ok()
                    .and_then(|value| credential_chunk_count(&value).ok().flatten())
                    .unwrap_or(0);
                delete_password_chunks(service, secret_user, chunk_count);
                match entry.delete_credential() {
                    Ok(()) => deleted = true,
                    Err(error) => last_error = Some(error.to_string()),
                }
            }
            Err(error) => last_error = Some(error.to_string()),
        }
    }
    if deleted {
        Ok(())
    } else {
        Err(format!(
            "Credential konnte nicht gelöscht werden: {}",
            last_error.unwrap_or_else(|| "kein Credential-Service erreichbar".into())
        ))
    }
}

fn current_unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn pkce_verifier() -> String {
    format!(
        "{}{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

fn url_component(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![byte as char]
            }
            _ => format!("%{byte:02X}").chars().collect::<Vec<_>>(),
        })
        .collect()
}

fn login_imap(
    host: &str,
    port: u16,
    email: &str,
    password: &str,
) -> Result<imap::Session<native_tls::TlsStream<TcpStream>>, String> {
    if port != 993 {
        let _ = test_imap_transport(host, port)?;
        return Err(
            "Aktuell ist für den Login nur IMAP über SSL/TLS auf Port 993 aktiviert.".into(),
        );
    }

    let tls = native_tls::TlsConnector::builder()
        .build()
        .map_err(|error| format!("TLS konnte nicht initialisiert werden: {error}"))?;
    let client = connect_imap_with_timeout(host, port, &tls)?;
    client
        .login(email, password)
        .map_err(|(error, _client)| format!("IMAP-Login fehlgeschlagen: {error}"))
}

fn login_imap_account(
    host: &str,
    port: u16,
    account: &AccountConnection,
    login_secret: &str,
) -> Result<imap::Session<native_tls::TlsStream<TcpStream>>, String> {
    if account.auth_type != "oauth" {
        return login_imap(host, port, &account.email, login_secret);
    }
    if port != 993 {
        let _ = test_imap_transport(host, port)?;
        return Err("Aktuell ist für OAuth-IMAP nur SSL/TLS auf Port 993 aktiviert.".into());
    }

    let tls = native_tls::TlsConnector::builder()
        .build()
        .map_err(|error| format!("TLS konnte nicht initialisiert werden: {error}"))?;
    let client = connect_imap_with_timeout(host, port, &tls)?;
    let authenticator = OAuth2Authenticator {
        user: account.email.clone(),
        access_token: login_secret.to_string(),
    };
    client
        .authenticate("XOAUTH2", &authenticator)
        .map_err(|(error, _client)| format!("IMAP-OAuth-Login fehlgeschlagen: {error}"))
}

fn connect_imap_with_timeout(
    host: &str,
    port: u16,
    tls: &native_tls::TlsConnector,
) -> Result<imap::Client<native_tls::TlsStream<TcpStream>>, String> {
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|error| format!("DNS-Auflösung für IMAP fehlgeschlagen: {error}"))?
        .collect::<Vec<SocketAddr>>();
    if addresses.is_empty() {
        return Err("DNS-Auflösung für IMAP lieferte keine Adresse.".into());
    }

    let mut errors = Vec::new();
    for address in addresses {
        let stream = match TcpStream::connect_timeout(&address, IMAP_NETWORK_TIMEOUT) {
            Ok(stream) => stream,
            Err(error) => {
                errors.push(format!("{address}: {error}"));
                continue;
            }
        };
        if let Err(error) = stream.set_read_timeout(Some(IMAP_NETWORK_TIMEOUT)) {
            errors.push(format!(
                "{address}: Lesetimeout konnte nicht gesetzt werden: {error}"
            ));
            continue;
        }
        if let Err(error) = stream.set_write_timeout(Some(IMAP_NETWORK_TIMEOUT)) {
            errors.push(format!(
                "{address}: Schreibtimeout konnte nicht gesetzt werden: {error}"
            ));
            continue;
        }
        let tls_stream = match tls.connect(host, stream) {
            Ok(stream) => stream,
            Err(error) => {
                errors.push(format!("{address}: TLS-Handshake fehlgeschlagen: {error}"));
                continue;
            }
        };
        let mut client = imap::Client::new(tls_stream);
        match client.read_greeting() {
            Ok(_) => return Ok(client),
            Err(error) => errors.push(format!("{address}: IMAP-Begrüßung fehlgeschlagen: {error}")),
        }
    }

    Err(format!(
        "IMAP-Verbindung zu {host}:{port} fehlgeschlagen (Timeout {} s). {}",
        IMAP_NETWORK_TIMEOUT.as_secs(),
        errors.join(" | ")
    ))
}

fn parse_fetch(fetch: &imap::types::Fetch) -> Option<ParsedMessage> {
    let body = fetch.body()?;
    let parsed = parse_mail(body).ok()?;
    let uid = fetch.uid.map(|value| value.to_string())?;
    let flags = fetch
        .flags()
        .iter()
        .map(|flag| flag.to_string())
        .collect::<Vec<_>>();
    let flags_json = serde_json::to_string(&flags).unwrap_or_else(|_| "[]".into());

    let subject = parsed
        .headers
        .get_first_value("Subject")
        .unwrap_or_else(|| "(Kein Betreff)".into());
    let from_raw = parsed.headers.get_first_value("From").unwrap_or_default();
    let (from_name, from_email) = parse_mailbox(&from_raw);
    let to_json = serde_json::to_string(&split_address_list(
        &parsed.headers.get_first_value("To").unwrap_or_default(),
    ))
    .unwrap_or_else(|_| "[]".into());
    let raw_date = parsed
        .headers
        .get_first_value("Date")
        .unwrap_or_else(current_timestamp);
    let date = canonical_message_date(&raw_date).unwrap_or_else(current_timestamp);
    let body_text = normalize_body_text(&extract_text_body(&parsed));
    let body_html = sanitize_email_html(&extract_html_body(&parsed));
    let attachments = extract_attachments(&parsed);
    let has_attachments = !attachments.is_empty();
    let list_unsubscribe = parsed.headers.get_first_value("List-Unsubscribe");
    let list_unsubscribe_post = parsed.headers.get_first_value("List-Unsubscribe-Post");
    let unsubscribe_one_click = list_unsubscribe_post.as_deref().is_some_and(|value| {
        value
            .to_ascii_lowercase()
            .contains("list-unsubscribe=one-click")
    });
    let raw_headers_json = serde_json::json!({
        "messageId": parsed.headers.get_first_value("Message-ID"),
        "listUnsubscribe": list_unsubscribe,
        "listUnsubscribePost": list_unsubscribe_post,
        "contentType": parsed.headers.get_first_value("Content-Type")
    })
    .to_string();
    let has_newsletter_signals = list_unsubscribe.is_some()
        || parsed.headers.get_first_value("List-ID").is_some()
        || parsed.headers.get_first_value("Precedence").is_some();

    Some(ParsedMessage {
        uid,
        subject,
        from_name,
        from_email,
        to_json,
        date,
        flags_json,
        body_text,
        body_html,
        raw_headers_json,
        attachments,
        has_attachments,
        has_newsletter_signals,
        unsubscribe_target: list_unsubscribe,
        unsubscribe_one_click,
    })
}

fn extract_text_body(parsed: &ParsedMail<'_>) -> String {
    if parsed.subparts.is_empty() {
        let mimetype = parsed.ctype.mimetype.to_ascii_lowercase();
        if mimetype == "text/plain" || mimetype == "text/html" {
            return parsed.get_body().unwrap_or_default();
        }
        return String::new();
    }

    let plain = parsed
        .subparts
        .iter()
        .find(|part| part.ctype.mimetype.eq_ignore_ascii_case("text/plain"))
        .map(extract_text_body)
        .unwrap_or_default();

    if !plain.trim().is_empty() {
        return plain;
    }

    parsed
        .subparts
        .iter()
        .map(extract_text_body)
        .find(|text| !text.trim().is_empty())
        .unwrap_or_default()
}

fn extract_html_body(parsed: &ParsedMail<'_>) -> String {
    if parsed.subparts.is_empty() {
        if parsed.ctype.mimetype.eq_ignore_ascii_case("text/html") {
            return parsed.get_body().unwrap_or_default();
        }
        return String::new();
    }

    parsed
        .subparts
        .iter()
        .map(extract_html_body)
        .find(|html| !html.trim().is_empty())
        .unwrap_or_default()
}

fn extract_attachments(parsed: &ParsedMail<'_>) -> Vec<ParsedAttachment> {
    let mut attachments = Vec::new();
    collect_attachments(parsed, &mut attachments);
    attachments
}

fn collect_attachments(parsed: &ParsedMail<'_>, attachments: &mut Vec<ParsedAttachment>) {
    let disposition = parsed.get_content_disposition();
    let filename = disposition
        .params
        .get("filename")
        .or_else(|| parsed.ctype.params.get("name"))
        .cloned()
        .unwrap_or_default();
    let content_id = parsed
        .headers
        .get_first_value("Content-ID")
        .unwrap_or_default();
    let content_location = parsed
        .headers
        .get_first_value("Content-Location")
        .unwrap_or_default();
    let content_reference = if content_id.trim().is_empty() {
        content_location
    } else {
        content_id
    };
    let mimetype = parsed.ctype.mimetype.to_ascii_lowercase();
    let disposition_kind = format!("{:?}", disposition.disposition).to_ascii_lowercase();
    let is_inline = disposition_kind.contains("inline") || !content_reference.trim().is_empty();
    let is_attachment = disposition_kind.contains("attachment")
        || is_inline
        || (!filename.trim().is_empty() && !mimetype.starts_with("text/"));

    if is_attachment && !is_body_like_mail_part(&filename, &mimetype, &disposition_kind, is_inline)
    {
        let body = parsed.get_body_raw().unwrap_or_default();
        attachments.push(ParsedAttachment {
            filename: if filename.trim().is_empty() {
                "Anhang".into()
            } else {
                filename
            },
            mime_type: parsed.ctype.mimetype.clone(),
            size: body.len(),
            content_id: content_reference,
            is_inline,
            bytes: body,
        });
    }

    for part in &parsed.subparts {
        collect_attachments(part, attachments);
    }
}

fn is_body_like_mail_part(
    filename: &str,
    mimetype: &str,
    disposition_kind: &str,
    is_inline: bool,
) -> bool {
    let filename = filename.trim().to_ascii_lowercase();
    let mimetype = mimetype.trim().to_ascii_lowercase();
    let has_filename = !filename.is_empty();
    if mimetype.starts_with("multipart/") {
        return true;
    }
    if !has_filename && (mimetype == "text/html" || mimetype == "text/plain") {
        return true;
    }
    if !has_filename && disposition_kind.contains("attachment") && mimetype.starts_with("text/") {
        return true;
    }
    if is_inline
        && !mimetype.starts_with("image/")
        && !mimetype.starts_with("video/")
        && mimetype != "application/pdf"
    {
        return true;
    }
    false
}

fn normalize_body_text(value: &str) -> String {
    let without_css = remove_css_blocks(value);
    let decoded = decode_basic_entities(&without_css);
    strip_html_tags(&decoded)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn strip_html_tags(value: &str) -> String {
    let mut output = String::new();
    let mut inside_tag = false;

    for character in value.chars() {
        match character {
            '<' => {
                inside_tag = true;
                output.push(' ');
            }
            '>' => {
                inside_tag = false;
                output.push(' ');
            }
            _ if !inside_tag => output.push(character),
            _ => {}
        }
    }

    output
}

fn remove_css_blocks(value: &str) -> String {
    let mut output = String::new();
    let mut rest = value;
    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(start) = lower.find("<style") else {
            output.push_str(rest);
            break;
        };
        output.push_str(&rest[..start]);
        let after_start = &rest[start..];
        let lower_after = after_start.to_ascii_lowercase();
        if let Some(end) = lower_after.find("</style>") {
            rest = &after_start[end + "</style>".len()..];
        } else {
            break;
        }
    }
    output
}

fn decode_basic_entities(value: &str) -> String {
    value
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

fn sanitize_email_html(value: &str) -> String {
    if value.trim().is_empty() {
        return String::new();
    }
    let mut html = value.to_string();
    for needle in ["script", "iframe", "object", "embed"] {
        html = remove_tag_blocks(&html, needle);
    }
    strip_event_handlers(&html)
}

fn remove_tag_blocks(value: &str, tag: &str) -> String {
    let mut output = String::new();
    let mut rest = value;
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(start) = lower.find(&open) else {
            output.push_str(rest);
            break;
        };
        output.push_str(&rest[..start]);
        let after_start = &rest[start..];
        let lower_after = after_start.to_ascii_lowercase();
        if let Some(end) = lower_after.find(&close) {
            rest = &after_start[end + close.len()..];
        } else {
            break;
        }
    }
    output
}

fn strip_event_handlers(value: &str) -> String {
    let mut output = String::new();
    let mut rest = value;
    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(pos) = lower.find(" on") else {
            output.push_str(rest);
            break;
        };
        output.push_str(&rest[..pos]);
        let after = &rest[pos + 1..];
        if let Some(eq_pos) = after.find('=') {
            let after_eq = &after[eq_pos + 1..];
            let quote = after_eq.chars().next().unwrap_or(' ');
            if quote == '"' || quote == '\'' {
                if let Some(end) = after_eq[1..].find(quote) {
                    rest = &after_eq[end + 2..];
                    continue;
                }
            }
        }
        output.push(' ');
        rest = after;
    }
    output
}

fn parse_mailbox(raw: &str) -> (String, String) {
    if let (Some(start), Some(end)) = (raw.find('<'), raw.find('>')) {
        let name = raw[..start].trim().trim_matches('"').to_string();
        let email = raw[start + 1..end].trim().to_lowercase();
        return (
            if name.is_empty() {
                email.split('@').next().unwrap_or("Unbekannt").to_string()
            } else {
                name
            },
            email,
        );
    }

    let email = raw.trim().trim_matches('"').to_lowercase();
    let name = email.split('@').next().unwrap_or("Unbekannt").to_string();
    (name, email)
}

fn split_address_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|entry| parse_mailbox(entry).1)
        .filter(|entry| !entry.is_empty())
        .collect()
}

fn sender_domain(email: &str) -> String {
    email
        .split('@')
        .nth(1)
        .unwrap_or("unknown.local")
        .to_ascii_lowercase()
}

fn current_timestamp() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}

fn canonical_message_date(value: &str) -> Option<String> {
    let value = value.trim();
    let has_plausible_year = value
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| part.len() == 4)
        .filter_map(|part| part.parse::<i32>().ok())
        .any(|year| (1900..=3000).contains(&year));
    if !has_plausible_year {
        return None;
    }
    let unix_timestamp = dateparse(value).ok()?;
    time::OffsetDateTime::from_unix_timestamp(unix_timestamp)
        .ok()?
        .format(&time::format_description::well_known::Rfc3339)
        .ok()
}

fn folder_role(name: &str) -> Option<&'static str> {
    let normalized = name
        .replace('\\', "/")
        .split('/')
        .next_back()
        .unwrap_or(name)
        .trim()
        .to_ascii_lowercase();
    if normalized == "inbox" || normalized == "posteingang" {
        Some("inbox")
    } else if normalized.contains("sent") || normalized.contains("gesendet") {
        Some("sent")
    } else if normalized.contains("draft")
        || normalized.contains("entwurf")
        || normalized.contains("entw&apw-rfe")
    {
        Some("drafts")
    } else if normalized.contains("spam") || normalized.contains("junk") {
        Some("spam")
    } else if normalized.contains("trash")
        || normalized.contains("papierkorb")
        || normalized.contains("deleted")
    {
        Some("trash")
    } else if normalized.contains("archive")
        || normalized.contains("archiv")
        || normalized.contains("all mail")
        || normalized.contains("alle mails")
        || normalized.contains("alle nachrichten")
    {
        Some("archive")
    } else {
        None
    }
}

fn folder_role_from_attributes(
    attributes: &[imap::types::NameAttribute<'_>],
) -> Option<&'static str> {
    attributes.iter().find_map(|attribute| {
        let imap::types::NameAttribute::Custom(value) = attribute else {
            return None;
        };
        match value.trim().to_ascii_lowercase().as_str() {
            "\\inbox" => Some("inbox"),
            "\\sent" => Some("sent"),
            "\\drafts" => Some("drafts"),
            "\\archive" | "\\all" => Some("archive"),
            "\\junk" | "\\spam" => Some("spam"),
            "\\trash" => Some("trash"),
            _ => None,
        }
    })
}

fn discovered_folder(
    name: &str,
    attributes: &[imap::types::NameAttribute<'_>],
) -> DiscoveredFolder {
    let special_use_role = folder_role_from_attributes(attributes);
    let heuristic_role = folder_role(name);
    DiscoveredFolder {
        name: name.to_string(),
        role: special_use_role.or(heuristic_role).map(str::to_string),
        role_source: special_use_role
            .map(|_| "special_use".to_string())
            .or_else(|| heuristic_role.map(|_| "heuristic".to_string())),
        selectable: !attributes
            .iter()
            .any(|attribute| matches!(attribute, imap::types::NameAttribute::NoSelect)),
    }
}

fn syncable_folders(discovered: &[DiscoveredFolder]) -> Vec<DiscoveredFolder> {
    let mut folders = discovered
        .iter()
        .filter(|folder| folder.selectable && folder.role.is_some())
        .cloned()
        .collect::<Vec<_>>();
    if folders.is_empty() {
        folders.extend(
            discovered
                .iter()
                .find(|folder| folder.selectable && folder.name.eq_ignore_ascii_case("INBOX"))
                .cloned()
                .or_else(|| {
                    Some(DiscoveredFolder {
                        name: "INBOX".into(),
                        role: Some("inbox".into()),
                        role_source: Some("heuristic".into()),
                        selectable: true,
                    })
                }),
        );
    }
    folders.sort_by_key(|folder| folder_sync_priority(&folder.name));
    folders.dedup_by(|left, right| left.name.eq_ignore_ascii_case(&right.name));
    folders
}

fn folder_sync_priority(name: &str) -> u8 {
    match folder_role(name) {
        Some("inbox") => 0,
        Some("archive") => 1,
        Some("sent") => 2,
        Some("spam") => 3,
        Some("trash") => 4,
        Some("drafts") => 5,
        _ => 9,
    }
}

fn upsert_discovered_folders(
    db: &Connection,
    account_id: &str,
    folders: &[DiscoveredFolder],
) -> Result<(), String> {
    for folder in folders {
        let _ = upsert_discovered_folder(db, account_id, folder)?;
    }

    Ok(())
}

fn upsert_folder(db: &Connection, account_id: &str, folder_name: &str) -> Result<String, String> {
    let role = folder_role(folder_name).map(str::to_string);
    let folder = DiscoveredFolder {
        name: folder_name.to_string(),
        role_source: role.as_ref().map(|_| "heuristic".to_string()),
        role,
        selectable: true,
    };
    upsert_discovered_folder(db, account_id, &folder)
}

fn upsert_discovered_folder(
    db: &Connection,
    account_id: &str,
    folder: &DiscoveredFolder,
) -> Result<String, String> {
    if let Some(id) = db
        .query_row(
            "SELECT id FROM folders WHERE account_id = ?1 AND path = ?2",
            params![account_id, folder.name],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
    {
        db.execute(
            "UPDATE folders
             SET display_name = ?3,
                 role = CASE
                   WHEN ?5 = 'special_use' THEN ?4
                   WHEN role_source IS NULL OR role_source = 'heuristic' THEN ?4
                   ELSE role
                 END,
                 role_source = CASE
                   WHEN ?5 = 'special_use' THEN ?5
                   WHEN role_source IS NULL OR role_source = 'heuristic' THEN ?5
                   ELSE role_source
                 END,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1 AND account_id = ?2",
            params![id, account_id, folder.name, folder.role, folder.role_source],
        )
        .map_err(|error| error.to_string())?;
        return Ok(id);
    }

    let id = format!("folder-{}", uuid::Uuid::new_v4());
    db.execute(
        "INSERT INTO folders (id, account_id, path, display_name, role, role_source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            id,
            account_id,
            folder.name,
            folder.name,
            folder.role,
            folder.role_source
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(id)
}

fn upsert_sender(db: &Connection, message: &ParsedMessage) -> Result<(), String> {
    let domain = sender_domain(&message.from_email);
    db.execute(
        "INSERT INTO senders (id, normalized_email, domain, display_name, first_seen_at, last_seen_at, message_count)
         VALUES (?1, ?2, ?3, ?4, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, 1)
         ON CONFLICT(normalized_email) DO UPDATE SET
           display_name = excluded.display_name,
           last_seen_at = CURRENT_TIMESTAMP,
           message_count = message_count + 1",
        params![
            format!("sender-{}", uuid::Uuid::new_v4()),
            message.from_email,
            domain,
            message.from_name
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

struct UpsertMessageResult {
    message_id: String,
    inserted: bool,
}

fn upsert_message(
    db: &Connection,
    attachment_dir: &Path,
    account_id: &str,
    folder_id: &str,
    message: &ParsedMessage,
) -> Result<UpsertMessageResult, String> {
    let effective_folder_id = blocked_sender_folder_id(db, account_id, &message.from_email)?
        .unwrap_or_else(|| folder_id.to_string());
    let existing_message_id = db
        .query_row(
            "SELECT id FROM messages WHERE account_id = ?1 AND folder_id = ?2 AND uid = ?3",
            params![account_id, &effective_folder_id, message.uid],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let inserted = existing_message_id.is_none();
    let message_id = existing_message_id.unwrap_or_else(|| format!("msg-{}", uuid::Uuid::new_v4()));
    db.execute(
        "INSERT INTO messages (
           id, account_id, folder_id, uid, subject, from_addr, to_addrs_json, date,
           flags_json, body_text, body_html_sanitized, raw_headers_json, has_attachments
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT(account_id, folder_id, uid) DO UPDATE SET
           subject = excluded.subject,
           from_addr = excluded.from_addr,
           to_addrs_json = excluded.to_addrs_json,
           date = excluded.date,
           flags_json = excluded.flags_json,
           body_text = excluded.body_text,
           body_html_sanitized = excluded.body_html_sanitized,
           raw_headers_json = excluded.raw_headers_json,
           has_attachments = excluded.has_attachments,
           updated_at = CURRENT_TIMESTAMP",
        params![
            &message_id,
            account_id,
            &effective_folder_id,
            message.uid,
            message.subject,
            message.from_email,
            message.to_json,
            message.date,
            message.flags_json,
            message.body_text,
            if message.body_html.trim().is_empty() {
                None::<String>
            } else {
                Some(message.body_html.clone())
            },
            message.raw_headers_json,
            if message.has_attachments { 1 } else { 0 }
        ],
    )
    .map_err(|error| error.to_string())?;
    upsert_attachments(db, attachment_dir, &message_id, &message.attachments)?;
    Ok(UpsertMessageResult {
        message_id,
        inserted,
    })
}

fn flags_json_is_unread(flags_json: &str) -> bool {
    serde_json::from_str::<Vec<String>>(flags_json)
        .unwrap_or_default()
        .iter()
        .all(|flag| !flag.eq_ignore_ascii_case("\\Seen"))
}

fn blocked_sender_folder_id(
    db: &Connection,
    account_id: &str,
    sender_email: &str,
) -> Result<Option<String>, String> {
    let normalized_sender = sender_email.trim().to_ascii_lowercase();
    let blocked = db
        .query_row(
            "SELECT COUNT(*) FROM blocked_senders WHERE account_id = ?1 AND lower(sender_email) = lower(?2)",
            params![account_id, normalized_sender],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| error.to_string())?
        > 0;
    if !blocked {
        return Ok(None);
    }
    let spam_folder = find_existing_folder_locked(db, account_id, &["spam", "junk"])
        .unwrap_or_else(|| "Spam".into());
    upsert_folder(db, account_id, &spam_folder).map(Some)
}

fn sender_is_trusted_locked(
    db: &Connection,
    account_id: &str,
    sender_email: &str,
) -> Result<bool, String> {
    db.query_row(
        "SELECT COUNT(*) FROM trusted_senders WHERE account_id = ?1 AND lower(sender_email) = lower(?2)",
        params![account_id, sender_email.trim()],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| count > 0)
    .map_err(|error| error.to_string())
}

fn trusted_sender_emails_locked(
    db: &Connection,
    account_id: &str,
) -> Result<HashSet<String>, String> {
    let mut statement = db
        .prepare("SELECT lower(sender_email) FROM trusted_senders WHERE account_id = ?1")
        .map_err(|error| error.to_string())?;
    let senders = statement
        .query_map(params![account_id], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(senders)
}

fn set_sender_trusted_locked(
    db: &Connection,
    account_id: &str,
    sender_email: &str,
    trusted: bool,
) -> Result<(), String> {
    let sender_email = sender_email.trim().to_ascii_lowercase();
    if trusted {
        db.execute(
            "INSERT INTO trusted_senders (id, account_id, sender_email)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(account_id, sender_email) DO NOTHING",
            params![
                format!("trusted-{}", uuid::Uuid::new_v4()),
                account_id,
                sender_email
            ],
        )
        .map_err(|error| error.to_string())?;
    } else {
        db.execute(
            "DELETE FROM trusted_senders WHERE account_id = ?1 AND lower(sender_email) = lower(?2)",
            params![account_id, sender_email],
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn upsert_attachments(
    db: &Connection,
    attachment_dir: &Path,
    message_id: &str,
    attachments: &[ParsedAttachment],
) -> Result<(), String> {
    fs::create_dir_all(attachment_dir).map_err(|error| error.to_string())?;
    db.execute(
        "DELETE FROM attachments WHERE message_id = ?1",
        params![message_id],
    )
    .map_err(|error| error.to_string())?;
    for (index, attachment) in attachments.iter().enumerate() {
        let attachment_id = format!("att-{}", uuid::Uuid::new_v4());
        let storage_ref = if attachment.bytes.is_empty() {
            None
        } else {
            let extension = extension_for_attachment(&attachment.filename, &attachment.mime_type);
            let filename = format!(
                "{}-{}{}",
                sanitize_storage_segment(message_id),
                index,
                extension
            );
            let path = attachment_dir.join(filename);
            fs::write(&path, &attachment.bytes).map_err(|error| error.to_string())?;
            Some(path.to_string_lossy().to_string())
        };
        db.execute(
            "INSERT INTO attachments (id, message_id, filename, mime_type, size, content_id, storage_ref, scan_status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                attachment_id,
                message_id,
                attachment.filename,
                attachment.mime_type,
                attachment.size as i64,
                normalized_content_id(&attachment.content_id),
                storage_ref,
                if attachment.is_inline { "inline_cached" } else { "cached" }
            ],
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn normalized_content_id(value: &str) -> Option<String> {
    let cleaned = value.trim().trim_matches('<').trim_matches('>').to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn sanitize_storage_segment(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn extension_for_attachment(filename: &str, mime_type: &str) -> String {
    if let Some(pos) = filename.rfind('.') {
        let extension = &filename[pos..];
        if extension.len() <= 12
            && extension
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.')
        {
            return extension.to_string();
        }
    }
    match mime_type.to_ascii_lowercase().as_str() {
        "image/png" => ".png".into(),
        "image/jpeg" | "image/jpg" => ".jpg".into(),
        "image/gif" => ".gif".into(),
        "image/webp" => ".webp".into(),
        "image/avif" => ".avif".into(),
        "image/svg+xml" => ".svg".into(),
        "video/mp4" => ".mp4".into(),
        "video/webm" => ".webm".into(),
        "video/quicktime" => ".mov".into(),
        "application/pdf" => ".pdf".into(),
        "text/plain" => ".txt".into(),
        _ => ".bin".into(),
    }
}

fn upsert_newsletter_candidate(
    db: &Connection,
    account_id: &str,
    message: &ParsedMessage,
) -> Result<(), String> {
    let sender_id = db
        .query_row(
            "SELECT id FROM senders WHERE normalized_email = ?1",
            params![message.from_email],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| error.to_string())?;
    let evidence = serde_json::to_string(&vec!["List-Unsubscribe or list headers".to_string()])
        .unwrap_or_else(|_| "[]".into());
    let (mut unsubscribe_type, unsubscribe_target) = message
        .unsubscribe_target
        .as_ref()
        .map(|target| parse_unsubscribe_target("unknown", target))
        .unwrap_or_else(|| ("unknown".into(), String::new()));
    if message.unsubscribe_one_click && unsubscribe_type == "http_get" {
        unsubscribe_type = "http_post".into();
    }
    db.execute(
        "INSERT INTO newsletter_subscriptions (
           id, sender_id, account_id, evidence_json, unsubscribe_type, unsubscribe_target,
           confidence_score, risk_score, status
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 70, 30, 'candidate')
         ON CONFLICT(account_id, sender_id) DO UPDATE SET
           evidence_json = excluded.evidence_json,
           unsubscribe_target = excluded.unsubscribe_target,
           confidence_score = excluded.confidence_score,
           risk_score = excluded.risk_score,
           updated_at = CURRENT_TIMESTAMP",
        params![
            format!("newsletter-{}", uuid::Uuid::new_v4()),
            sender_id,
            account_id,
            evidence,
            unsubscribe_type,
            if unsubscribe_target.is_empty() {
                None::<String>
            } else {
                Some(unsubscribe_target)
            }
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn open_database(app_data_dir: &Path) -> Result<Connection, String> {
    fs::create_dir_all(app_data_dir).map_err(|error| error.to_string())?;
    let db_path = app_data_dir.join("kizumail.sqlite3");
    let db = Connection::open(db_path).map_err(|error| error.to_string())?;
    db.busy_timeout(Duration::from_secs(5))
        .map_err(|error| error.to_string())?;
    db.pragma_update(None, "foreign_keys", "ON")
        .map_err(|error| error.to_string())?;
    db.pragma_update(None, "journal_mode", "WAL")
        .map_err(|error| error.to_string())?;
    db.pragma_update(None, "synchronous", "NORMAL")
        .map_err(|error| error.to_string())?;
    apply_initial_migration(&db)?;
    ai::index::apply_ai_index_migration(&db)?;
    seed_database(&db)?;
    Ok(db)
}

fn open_read_database(state: &State<'_, AppState>) -> Result<Connection, String> {
    let db = Connection::open_with_flags(
        &state.db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| error.to_string())?;
    db.busy_timeout(Duration::from_secs(5))
        .map_err(|error| error.to_string())?;
    db.pragma_update(None, "query_only", "ON")
        .map_err(|error| error.to_string())?;
    Ok(db)
}

fn apply_initial_migration(db: &Connection) -> Result<(), String> {
    let has_accounts_table: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'accounts'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;

    if has_accounts_table == 0 {
        db.execute_batch(INITIAL_MIGRATION)
            .map_err(|error| error.to_string())?;
    }

    ensure_app_metadata_table(db)?;
    ensure_column(db, "accounts", "imap_host", "TEXT")?;
    ensure_column(db, "accounts", "imap_port", "INTEGER")?;
    ensure_column(db, "accounts", "smtp_host", "TEXT")?;
    ensure_column(db, "accounts", "smtp_port", "INTEGER")?;
    ensure_column(db, "accounts", "last_sync_at", "TEXT")?;
    ensure_column(db, "folders", "role_source", "TEXT")?;
    ensure_column(db, "messages", "body_html_sanitized", "TEXT")?;
    ensure_column(
        db,
        "messages",
        "has_attachments",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_attachments_table(db)?;
    ensure_column(db, "attachments", "storage_ref", "TEXT")?;
    ensure_column(
        db,
        "attachments",
        "scan_status",
        "TEXT NOT NULL DEFAULT 'pending'",
    )?;
    ensure_sync_state_table(db)?;
    ensure_blocked_senders_table(db)?;
    ensure_trusted_senders_table(db)?;
    ensure_performance_indexes(db)?;
    normalize_stored_folder_roles(db)?;
    normalize_stored_message_dates(db)?;

    Ok(())
}

fn ensure_app_metadata_table(db: &Connection) -> Result<(), String> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_metadata (
           key TEXT PRIMARY KEY,
           value TEXT NOT NULL,
           updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
         );",
    )
    .map_err(|error| error.to_string())
}

fn load_desktop_preferences(state: &State<'_, AppState>) -> Result<DesktopPreferences, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let raw = db
        .query_row(
            "SELECT value FROM app_metadata WHERE key = 'desktop_preferences'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    match raw {
        Some(value) => serde_json::from_str(&value).map_err(|error| error.to_string()),
        None => Ok(DesktopPreferences::default()),
    }
}

fn save_desktop_preferences(
    state: &State<'_, AppState>,
    preferences: &DesktopPreferences,
) -> Result<(), String> {
    let value = serde_json::to_string(preferences).map_err(|error| error.to_string())?;
    let db = state.db.lock().map_err(|error| error.to_string())?;
    db.execute(
        "INSERT INTO app_metadata (key, value)
         VALUES ('desktop_preferences', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
        params![value],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn normalize_stored_folder_roles(db: &Connection) -> Result<(), String> {
    let folders = {
        let mut statement = db
            .prepare("SELECT id, path, role_source FROM folders")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows
    };

    for (id, path, role_source) in folders {
        if role_source.as_deref() == Some("special_use") {
            continue;
        }
        let role = folder_role(&path);
        db.execute(
            "UPDATE folders
             SET role = ?2,
                 role_source = ?3,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
            params![id, role, role.map(|_| "heuristic")],
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn normalize_stored_message_dates(db: &Connection) -> Result<(), String> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_metadata (
           key TEXT PRIMARY KEY,
           value TEXT NOT NULL,
           updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
         );",
    )
    .map_err(|error| error.to_string())?;
    let already_normalized = db
        .query_row(
            "SELECT value FROM app_metadata WHERE key = 'message_date_format'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .as_deref()
        == Some("rfc3339-v1");
    if already_normalized {
        return Ok(());
    }

    let messages = {
        let mut statement = db
            .prepare("SELECT id, date FROM messages WHERE date IS NOT NULL AND trim(date) <> ''")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    for (id, raw_date) in messages {
        if let Some(date) = canonical_message_date(&raw_date) {
            db.execute(
                "UPDATE messages SET date = ?2 WHERE id = ?1",
                params![id, date],
            )
            .map_err(|error| error.to_string())?;
        }
    }
    db.execute(
        "INSERT INTO app_metadata (key, value)
         VALUES ('message_date_format', 'rfc3339-v1')
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
        [],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn ensure_attachments_table(db: &Connection) -> Result<(), String> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS attachments (
          id TEXT PRIMARY KEY,
          message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
          filename TEXT,
          mime_type TEXT,
          size INTEGER NOT NULL DEFAULT 0,
          content_id TEXT,
          storage_ref TEXT,
          scan_status TEXT NOT NULL DEFAULT 'pending',
          created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );",
    )
    .map_err(|error| error.to_string())
}

fn ensure_sync_state_table(db: &Connection) -> Result<(), String> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS sync_state (
          id TEXT PRIMARY KEY,
          account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
          folder_id TEXT NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
          cursor_json TEXT NOT NULL DEFAULT '{}',
          last_error TEXT,
          backoff_until TEXT,
          updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE UNIQUE INDEX IF NOT EXISTS sync_state_account_folder_idx ON sync_state(account_id, folder_id);",
    )
    .map_err(|error| error.to_string())
}

fn ensure_blocked_senders_table(db: &Connection) -> Result<(), String> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS blocked_senders (
          id TEXT PRIMARY KEY,
          account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
          sender_email TEXT NOT NULL,
          domain TEXT NOT NULL,
          created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE UNIQUE INDEX IF NOT EXISTS blocked_senders_account_email_idx ON blocked_senders(account_id, sender_email);",
    )
    .map_err(|error| error.to_string())
}

fn ensure_trusted_senders_table(db: &Connection) -> Result<(), String> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS trusted_senders (
          id TEXT PRIMARY KEY,
          account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
          sender_email TEXT NOT NULL,
          created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE UNIQUE INDEX IF NOT EXISTS trusted_senders_account_email_idx ON trusted_senders(account_id, sender_email);",
    )
    .map_err(|error| error.to_string())
}

fn ensure_performance_indexes(db: &Connection) -> Result<(), String> {
    db.execute_batch(
        "CREATE INDEX IF NOT EXISTS messages_account_folder_date_idx ON messages(account_id, folder_id, date);
         CREATE INDEX IF NOT EXISTS messages_folder_uid_idx ON messages(folder_id, uid);
         CREATE INDEX IF NOT EXISTS messages_flags_idx ON messages(flags_json);
         CREATE INDEX IF NOT EXISTS messages_attachments_idx ON messages(has_attachments);
         CREATE INDEX IF NOT EXISTS attachments_message_idx ON attachments(message_id);
         CREATE INDEX IF NOT EXISTS folders_account_role_idx ON folders(account_id, role);",
    )
    .map_err(|error| error.to_string())
}

fn ensure_column(
    db: &Connection,
    table_name: &str,
    column_name: &str,
    column_definition: &str,
) -> Result<(), String> {
    let mut statement = db
        .prepare(&format!("PRAGMA table_info({table_name})"))
        .map_err(|error| error.to_string())?;
    let exists = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?
        .iter()
        .any(|existing| existing == column_name);

    if !exists {
        db.execute(
            &format!("ALTER TABLE {table_name} ADD COLUMN {column_name} {column_definition}"),
            [],
        )
        .map_err(|error| error.to_string())?;
    }

    Ok(())
}

fn seed_database(db: &Connection) -> Result<(), String> {
    let account_count: i64 = db
        .query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;

    if account_count > 0 {
        return Ok(());
    }

    db.execute(
        "INSERT INTO audit_log (id, action, entity_type, entity_id, metadata_json)
         VALUES ('audit-1', 'db.init', 'app', NULL, '{\"summary\":\"Initialized local KizuMail database.\"}')",
        [],
    )
    .map_err(|error| error.to_string())?;

    Ok(())
}

fn audit(
    db: &Connection,
    action: &str,
    entity_type: &str,
    entity_id: Option<&str>,
    metadata: serde_json::Value,
) -> Result<(), String> {
    db.execute(
        "INSERT INTO audit_log (id, action, entity_type, entity_id, metadata_json)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            format!("audit-{}", uuid::Uuid::new_v4()),
            action,
            entity_type,
            entity_id,
            metadata.to_string()
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub fn purge_user_data_cli() -> Result<(), String> {
    let roaming_dir = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("app.kizumail.desktop"));
    let local_dir = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("app.kizumail.desktop"));

    if let Some(data_dir) = roaming_dir.as_ref() {
        let db_path = data_dir.join("kizumail.sqlite3");
        if db_path.exists() {
            if let Ok(db) = Connection::open(&db_path) {
                if let Ok(mut statement) = db.prepare("SELECT id, email, secret_ref FROM accounts")
                {
                    if let Ok(rows) = statement.query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(2)?,
                        ))
                    }) {
                        for row in rows.flatten() {
                            delete_known_password_entries(&row.0, &row.1, row.2.as_deref());
                        }
                    }
                }
            }
        }
    }

    let mut errors = Vec::new();
    let mut directories = Vec::new();
    if let Some(path) = roaming_dir {
        directories.push(path);
    }
    if let Some(path) = local_dir {
        if !directories.iter().any(|candidate| candidate == &path) {
            directories.push(path);
        }
    }
    for directory in directories {
        if directory.exists() {
            if let Err(error) = fs::remove_dir_all(&directory) {
                errors.push(format!("{}: {error}", directory.display()));
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join(" | "))
    }
}

fn delete_known_password_entries(account_id: &str, email: &str, secret_ref: Option<&str>) {
    let mut candidates = vec![
        format!("{account_id}:imap"),
        format!("{account_id}:oauth"),
        email_secret_user(email),
        email_oauth_secret_user(email),
    ];
    if let Some(secret_user) = secret_ref.and_then(|value| value.rsplit('/').next()) {
        candidates.push(secret_user.to_string());
    }
    candidates.sort();
    candidates.dedup();
    for candidate in candidates {
        let _ = delete_password_entry(&candidate);
    }
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn setup_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "KizuMail öffnen", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "KizuMail beenden", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut builder = TrayIconBuilder::new()
        .tooltip("KizuMail")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            ) {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

fn start_background_sync_manager(app: AppHandle) {
    thread::spawn(move || {
        let mut started_accounts = HashSet::new();
        thread::sleep(Duration::from_secs(4));
        loop {
            let account_ids = {
                let state = app.state::<AppState>();
                account_ids_for_sync(&state, None).unwrap_or_default()
            };
            for account_id in account_ids {
                if started_accounts.insert(account_id.clone()) {
                    let account_app = app.clone();
                    thread::spawn(move || background_account_loop(account_app, account_id));
                }
            }
            thread::sleep(Duration::from_secs(20));
        }
    });
}

fn background_account_loop(app: AppHandle, account_id: String) {
    let mut last_sync = Instant::now() - BACKGROUND_FORCE_SYNC_INTERVAL;
    let mut last_backfill = Instant::now() - BACKGROUND_BACKFILL_INTERVAL;
    loop {
        let preferences = {
            let state = app.state::<AppState>();
            if !account_exists(&state, &account_id) {
                return;
            }
            load_desktop_preferences(&state).unwrap_or_default()
        };
        if !preferences.auto_sync {
            thread::sleep(Duration::from_secs(15));
            continue;
        }

        if last_sync.elapsed() >= BACKGROUND_FORCE_SYNC_INTERVAL {
            let include_backfill = last_backfill.elapsed() >= BACKGROUND_BACKFILL_INTERVAL;
            if background_sync_account(&app, &account_id, include_backfill).is_ok() {
                last_sync = Instant::now();
                if include_backfill {
                    last_backfill = Instant::now();
                }
            }
        }

        match wait_for_inbox_change(&app, &account_id) {
            Ok(Some(true)) => {
                if background_sync_account(&app, &account_id, false).is_ok() {
                    last_sync = Instant::now();
                }
            }
            Ok(Some(false)) => {}
            Ok(None) | Err(_) => {
                thread::sleep(BACKGROUND_POLL_INTERVAL);
                if background_sync_account(&app, &account_id, false).is_ok() {
                    last_sync = Instant::now();
                }
            }
        }
    }
}

fn account_exists(state: &State<'_, AppState>, account_id: &str) -> bool {
    state
        .db
        .lock()
        .ok()
        .and_then(|db| {
            db.query_row(
                "SELECT 1 FROM accounts WHERE id = ?1 AND sync_enabled = 1",
                params![account_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .ok()
            .flatten()
        })
        .is_some()
}

fn wait_for_inbox_change(app: &AppHandle, account_id: &str) -> Result<Option<bool>, String> {
    let state = app.state::<AppState>();
    let account = load_account_connection(&state, account_id)?;
    let config = connection_config(&account)?;
    let login_secret = load_login_secret(&state, &account)?;
    let imap_host = config
        .imap_host
        .as_deref()
        .ok_or_else(|| "IMAP-Host fehlt.".to_string())?;
    let imap_port = config
        .imap_port
        .ok_or_else(|| "IMAP-Port fehlt.".to_string())?;
    let mut session = login_imap_account(imap_host, imap_port, &account, &login_secret)?;
    let supports_idle = session
        .capabilities()
        .map_err(|error| format!("IMAP-Fähigkeiten konnten nicht gelesen werden: {error}"))?
        .has_str("IDLE");
    if !supports_idle {
        let _ = session.logout();
        return Ok(None);
    }
    session
        .select("INBOX")
        .map_err(|error| format!("Posteingang konnte für IDLE nicht geöffnet werden: {error}"))?;
    let outcome = session
        .idle()
        .and_then(|idle| idle.wait_with_timeout(BACKGROUND_IDLE_TIMEOUT))
        .map_err(|error| format!("IMAP-IDLE fehlgeschlagen: {error}"))?;
    let _ = session.logout();
    Ok(Some(matches!(
        outcome,
        imap::extensions::idle::WaitOutcome::MailboxChanged
    )))
}

fn background_sync_account(
    app: &AppHandle,
    account_id: &str,
    include_backfill: bool,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let sync_lock = account_sync_lock(&state, account_id)?;
    let _sync_guard = sync_lock.try_lock().map_err(|_| {
        format!("Für dieses Konto läuft bereits eine Synchronisation: {account_id}")
    })?;
    let delta = sync_account_delta(&state, account_id, 250)?;
    if include_backfill {
        let _ = sync_account_backfill(&state, account_id, None, 100);
    }
    let _ = app.emit(
        "sync://messages-changed",
        MessagesChangedEvent {
            account_id: account_id.to_string(),
            message_count: delta.message_count,
        },
    );
    let preferences = load_desktop_preferences(&state).unwrap_or_default();
    if preferences.notifications {
        send_background_notifications(app, &delta, preferences.privacy_mode);
    }
    Ok(())
}

fn send_background_notifications(app: &AppHandle, result: &SyncResult, privacy_mode: bool) {
    for message in result.new_messages.iter().take(5) {
        let title = if privacy_mode {
            "Neue E-Mail"
        } else if message.sender.trim().is_empty() {
            "Neue E-Mail"
        } else {
            &message.sender
        };
        let body = if privacy_mode {
            format!(
                "Neue Nachricht in KizuMail von {}.",
                if message.sender.trim().is_empty() {
                    "einem Absender"
                } else {
                    &message.sender
                }
            )
        } else if message.subject.trim().is_empty() {
            "Neue Nachricht eingegangen.".to_string()
        } else {
            message.subject.clone()
        };
        let _ = app.notification().builder().title(title).body(body).show();
    }
    if result.new_messages.len() > 5 {
        let _ = app
            .notification()
            .builder()
            .title("KizuMail")
            .body(format!(
                "{} weitere neue E-Mails sind eingegangen.",
                result.new_messages.len() - 5
            ))
            .show();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .args(["--minimized"])
                .build(),
        )
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let resource_dir = app
                .path()
                .resource_dir()
                .map_err(|error| format!("failed to resolve resource directory: {error}"))?;
            let _ = OAUTH_RESOURCE_DIR.set(resource_dir);
            let app_data_dir = app
                .path()
                .app_data_dir()
                .map_err(|error| format!("failed to resolve app data directory: {error}"))?;
            let db_path = app_data_dir.join("kizumail.sqlite3");
            let db = open_database(&app_data_dir)?;
            let attachment_dir = app_data_dir.join("attachments");
            fs::create_dir_all(&attachment_dir)
                .map_err(|error| format!("failed to create attachment directory: {error}"))?;
            app.manage(AppState {
                db: Mutex::new(db),
                db_path,
                password_cache: Mutex::new(HashMap::new()),
                account_sync_guards: Mutex::new(HashMap::new()),
                attachment_dir,
            });
            setup_tray(app)?;
            start_background_sync_manager(app.handle().clone());
            if std::env::args().any(|argument| argument == "--minimized") {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let minimize_to_tray =
                    load_desktop_preferences(&window.app_handle().state::<AppState>())
                        .map(|preferences| preferences.minimize_to_tray)
                        .unwrap_or(true);
                if minimize_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            desktop_preferences_get,
            desktop_preferences_update,
            accounts_list,
            accounts_create_imap,
            accounts_update_password,
            accounts_repair_credentials,
            accounts_security_audit,
            accounts_remove,
            accounts_test_imap_connection,
            accounts_sync_now,
            sync_delta,
            sync_backfill,
            sync_status,
            messages_list,
            messages_get,
            messages_move_local,
            messages_mark_read,
            messages_mark_flagged,
            sender_stats,
            sender_block,
            sender_unblock,
            smtp_send_email,
            oauth_provider_config,
            oauth_start,
            oauth_connect_interactive,
            oauth_complete,
            newsletter_list,
            newsletter_unsubscribe_preview,
            newsletter_unsubscribe_execute,
            audit_list,
            ai::commands::ai_settings_get,
            ai::commands::ai_settings_update,
            ai::commands::ai_summary_generate,
            ai::commands::ai_quick_replies_generate,
            ai::commands::ai_search_messages,
            ai::commands::ai_index_status,
            ai::commands::ai_index_start,
            ai::commands::ai_index_cancel,
            ai::commands::ai_secret_set,
            ai::commands::ai_secret_delete
        ])
        .run(tauri::generate_context!())
        .expect("error while running KizuMail");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_credentials_are_split_without_losing_unicode() {
        let value = format!("{}{}", "a".repeat(6000), "\u{1F510}".repeat(100));
        let chunks = split_credential_value(&value);
        assert!(chunks.len() > 1);
        assert!(chunks
            .iter()
            .all(|chunk| credential_utf16_byte_len(chunk) <= KEYRING_WINDOWS_BLOB_BYTE_LIMIT));
        assert!(chunks
            .iter()
            .all(|chunk| chunk.encode_utf16().count() <= KEYRING_CHUNK_UTF16_LIMIT));
        assert_eq!(chunks.concat(), value);

        let manifest = format!("{KEYRING_CHUNK_MANIFEST_PREFIX}{}", chunks.len());
        assert_eq!(
            credential_chunk_count(&manifest).unwrap(),
            Some(chunks.len())
        );
        assert_eq!(credential_chunk_count("legacy-secret").unwrap(), None);
        assert!(credential_chunk_count("kizumail-chunked-v1:0").is_err());
    }

    #[test]
    fn windows_credential_limit_is_checked_in_bytes() {
        assert_eq!(credential_utf16_byte_len(&"a".repeat(1280)), 2560);
        assert_eq!(credential_utf16_byte_len(&"a".repeat(1281)), 2562);
        assert_eq!(credential_utf16_byte_len("\u{1F510}"), 4);
        assert!(credential_fits_single_entry("xxxx-xxxx-xxxx-xxxx"));
        assert!(credential_fits_single_entry(&"a".repeat(1280)));
        assert!(!credential_fits_single_entry(&"a".repeat(1281)));
        assert!(KEYRING_CHUNK_UTF16_LIMIT * 2 < KEYRING_WINDOWS_BLOB_BYTE_LIMIT);
    }

    #[test]
    fn parses_list_unsubscribe_preferably_as_https_then_mailto() {
        assert_eq!(
            parse_unsubscribe_target(
                "unknown",
                "<mailto:list@example.com?subject=unsubscribe>, <HTTPS://example.com/u>"
            ),
            ("http_get".into(), "https://example.com/u".into())
        );
        assert_eq!(
            parse_unsubscribe_target("unknown", "<MAILTO:list@example.com>"),
            ("mailto".into(), "mailto:list@example.com".into())
        );
    }

    #[test]
    fn rejects_unsafe_unsubscribe_targets() {
        assert!(validate_http_unsubscribe_target("file:///tmp/unsubscribe").is_err());
        assert!(validate_http_unsubscribe_target("https://user:pass@example.com/u").is_err());
        assert!(validate_http_unsubscribe_target("https://example.com/u#confirm").is_err());
        assert!(validate_mailto_unsubscribe_target("mailto:unsubscribe").is_err());
        assert!(validate_mailto_unsubscribe_target("mailto:list@example.com?subject=ok").is_ok());
    }

    #[test]
    fn cursor_keeps_uid_boundaries_across_batches() {
        let mut cursor = SyncCursor {
            uid_validity: Some("42".into()),
            highest_uid: Some(10),
            lowest_uid: Some(10),
            backfill_complete: false,
            backfill_version: SYNC_BACKFILL_VERSION,
        };
        let messages = vec![
            ParsedMessage {
                uid: "12".into(),
                subject: String::new(),
                from_name: String::new(),
                from_email: String::new(),
                to_json: String::new(),
                date: String::new(),
                flags_json: String::new(),
                body_text: String::new(),
                body_html: String::new(),
                raw_headers_json: String::new(),
                attachments: Vec::new(),
                has_attachments: false,
                has_newsletter_signals: false,
                unsubscribe_target: None,
                unsubscribe_one_click: false,
            },
            ParsedMessage {
                uid: "11".into(),
                subject: String::new(),
                from_name: String::new(),
                from_email: String::new(),
                to_json: String::new(),
                date: String::new(),
                flags_json: String::new(),
                body_text: String::new(),
                body_html: String::new(),
                raw_headers_json: String::new(),
                attachments: Vec::new(),
                has_attachments: false,
                has_newsletter_signals: false,
                unsubscribe_target: None,
                unsubscribe_one_click: false,
            },
        ];
        update_cursor_from_messages(&mut cursor, &messages);
        assert_eq!(cursor.highest_uid, Some(12));
        assert_eq!(cursor.lowest_uid, Some(10));
    }

    #[test]
    fn backfill_selects_existing_sparse_uids_instead_of_assuming_dense_ranges() {
        let (selected, has_more) = select_backfill_uids(vec![2, 17, 81, 402, 900, 1_450], 2_000, 3);
        assert_eq!(selected, vec![402, 900, 1_450]);
        assert!(has_more);

        let (remaining, has_more) = select_backfill_uids(vec![2, 17, 81], 402, 10);
        assert_eq!(remaining, vec![2, 17, 81]);
        assert!(!has_more);
    }

    #[test]
    fn legacy_backfill_cursor_is_reopened_once_for_sparse_uid_algorithm() {
        let cursor = parse_sync_cursor(
            r#"{"uidValidity":"7","highestUid":900,"lowestUid":402,"backfillComplete":true}"#,
        );
        assert!(!cursor.backfill_complete);
        assert_eq!(cursor.backfill_version, SYNC_BACKFILL_VERSION);

        let encoded = serde_json::to_string(&cursor).unwrap();
        let reparsed = parse_sync_cursor(&encoded);
        assert!(!reparsed.backfill_complete);
        assert_eq!(reparsed.backfill_version, SYNC_BACKFILL_VERSION);
    }

    #[test]
    fn fair_page_target_covers_offset_and_has_more_sentinel() {
        assert_eq!(fair_page_target(250, 0), 251);
        assert_eq!(fair_page_target(250, 250), 501);
    }

    #[test]
    fn notification_candidate_requires_unread_flags() {
        assert!(flags_json_is_unread("[]"));
        assert!(flags_json_is_unread("[\"\\\\Flagged\"]"));
        assert!(!flags_json_is_unread("[\"\\\\Seen\"]"));
        assert!(!flags_json_is_unread("[\"\\\\seen\",\"\\\\Flagged\"]"));
    }

    #[test]
    fn trusted_sender_rules_are_account_scoped_and_reversible() {
        let db = Connection::open_in_memory().unwrap();
        apply_initial_migration(&db).unwrap();
        for id in ["account-a", "account-b"] {
            db.execute(
                "INSERT INTO accounts (id, provider, email, display_name, auth_type)
                 VALUES (?1, 'imap', ?2, ?2, 'password')",
                params![id, format!("{id}@mail.test")],
            )
            .unwrap();
        }

        set_sender_trusted_locked(&db, "account-a", "Sender@Example.com", true).unwrap();
        assert!(sender_is_trusted_locked(&db, "account-a", "sender@example.com").unwrap());
        assert!(!sender_is_trusted_locked(&db, "account-b", "sender@example.com").unwrap());
        assert_eq!(
            trusted_sender_emails_locked(&db, "account-a").unwrap(),
            HashSet::from(["sender@example.com".to_string()])
        );

        set_sender_trusted_locked(&db, "account-a", "sender@example.com", false).unwrap();
        assert!(!sender_is_trusted_locked(&db, "account-a", "sender@example.com").unwrap());
    }

    #[test]
    fn folder_roles_cover_localized_standard_mailboxes_without_child_false_positives() {
        assert_eq!(folder_role("[Gmail]/Alle Nachrichten"), Some("archive"));
        assert_eq!(folder_role("[Gmail]/Entw&APw-rfe"), Some("drafts"));
        assert_eq!(folder_role("Deleted Messages"), Some("trash"));
        assert_eq!(folder_role("Deleted/Syntax Institut"), None);
    }

    #[test]
    fn imap_special_use_attributes_override_localized_folder_names() {
        let sent = discovered_folder(
            "Gesendete Elemente",
            &[imap::types::NameAttribute::from("\\Sent")],
        );
        assert_eq!(sent.role.as_deref(), Some("sent"));
        assert_eq!(sent.role_source.as_deref(), Some("special_use"));

        let hidden = discovered_folder("Ordnergruppe", &[imap::types::NameAttribute::NoSelect]);
        assert!(!hidden.selectable);
    }

    #[test]
    fn mail_dates_are_normalized_for_stable_sql_sorting() {
        assert_eq!(
            canonical_message_date("Fri, 02 Jan 2026 23:38:16 +0000").as_deref(),
            Some("2026-01-02T23:38:16Z")
        );
        assert!(canonical_message_date("_smtpDate . 101468159").is_none());
    }

    #[test]
    fn message_role_query_returns_matching_mail_from_all_accounts() {
        let db = Connection::open_in_memory().unwrap();
        apply_initial_migration(&db).unwrap();
        for (id, provider) in [("account-a", "gmail"), ("account-b", "microsoft")] {
            db.execute(
                "INSERT INTO accounts (id, provider, email, display_name, auth_type)
                 VALUES (?1, ?2, ?3, ?3, 'oauth')",
                params![id, provider, format!("{id}@mail.test")],
            )
            .unwrap();
            db.execute(
                "INSERT INTO folders (id, account_id, path, display_name, role, role_source)
                 VALUES (?1, ?2, 'Sent', 'Sent', 'sent', 'special_use')",
                params![format!("folder-{id}"), id],
            )
            .unwrap();
            db.execute(
                "INSERT INTO messages (
                   id, account_id, folder_id, uid, subject, from_addr, date, flags_json, body_text
                 ) VALUES (?1, ?2, ?3, '1', ?4, ?5, '2026-07-13T12:00:00Z', '[\"\\\\Seen\"]', 'body')",
                params![
                    format!("message-{id}"),
                    id,
                    format!("folder-{id}"),
                    format!("subject-{id}"),
                    format!("sender-{id}@mail.test")
                ],
            )
            .unwrap();
        }

        let result = messages_list_from_db(
            &db,
            Some(MessageListInput {
                limit: Some(250),
                offset: Some(0),
                sort: Some("newest".into()),
                account_id: None,
                folder: None,
                folder_role: Some("sent".into()),
                unread: None,
                flagged: None,
                has_attachments: None,
                date_from: None,
                date_to: None,
            }),
        )
        .unwrap();

        assert_eq!(result.messages.len(), 2);
        assert!(result
            .messages
            .iter()
            .all(|message| message.body_text.is_empty() && message.body_html.is_none()));
        assert!(result
            .messages
            .iter()
            .all(|message| message.attachments.is_empty()));
        assert!(result
            .messages
            .iter()
            .all(|message| message.folder_role.as_deref() == Some("sent")));
        assert_eq!(
            result
                .messages
                .iter()
                .map(|message| message.account_id.as_str())
                .collect::<HashSet<_>>()
                .len(),
            2
        );

        let detail = message_get_from_db(&db, "message-account-a").unwrap();
        assert_eq!(detail.body_text, "body");
        assert_eq!(detail.account_id, "account-a");
    }

    #[test]
    fn inline_content_location_media_is_cached_as_an_attachment() {
        let raw = b"MIME-Version: 1.0\r\n\
Content-Type: multipart/related; boundary=mail-boundary\r\n\
\r\n\
--mail-boundary\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<html><body><img src=\"https://cdn.example.test/hero.png\"></body></html>\r\n\
--mail-boundary\r\n\
Content-Type: image/png\r\n\
Content-Location: https://cdn.example.test/hero.png\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
aGVsbG8=\r\n\
--mail-boundary--\r\n";
        let parsed = mailparse::parse_mail(raw).unwrap();
        let attachments = extract_attachments(&parsed);

        assert_eq!(attachments.len(), 1);
        assert!(attachments[0].is_inline);
        assert_eq!(
            attachments[0].content_id,
            "https://cdn.example.test/hero.png"
        );
        assert_eq!(attachments[0].mime_type, "image/png");
        assert_eq!(attachments[0].bytes, b"hello");
    }

    #[test]
    fn attachment_extensions_cover_previews_used_by_the_webview() {
        assert_eq!(extension_for_attachment("clip", "video/mp4"), ".mp4");
        assert_eq!(extension_for_attachment("vector", "image/svg+xml"), ".svg");
        assert_eq!(
            extension_for_attachment("document", "application/pdf"),
            ".pdf"
        );
    }

    #[test]
    fn microsoft_oauth_start_contains_desktop_pkce_scopes() {
        let start = build_oauth_start(
            "microsoft",
            "client-id",
            "person@outlook.com".into(),
            "http://127.0.0.1:53682/oauth/callback".into(),
        )
        .unwrap();
        assert!(start
            .authorization_url
            .contains("code_challenge_method=S256"));
        assert!(start.authorization_url.contains("offline_access"));
        assert!(start.authorization_url.contains("IMAP.AccessAsUser.All"));
        assert!(start.authorization_url.contains("SMTP.Send"));
        assert!(start.authorization_url.contains("openid"));
        assert!(start.authorization_url.contains("prompt=select_account"));
    }
}
