export type AccountProvider = "imap" | "microsoft" | "gmail";
export type AuthType = "password" | "oauth";
export type RiskLevel = "low" | "medium" | "high";
export type UnsubscribeMethod = "mailto" | "http_get" | "http_post" | "html_link" | "unknown";
export type MessageSortMode = "newest" | "oldest" | "unread" | "sender" | "subject" | "attachments";
export type AIProvider = "gemini" | "ollama";
export type AIIndexState = "not_started" | "idle" | "running" | "complete" | "cancelled" | "error";

export interface DesktopPreferences {
  autoSync: boolean;
  notifications: boolean;
  privacyMode: boolean;
  launchAtLogin: boolean;
  minimizeToTray: boolean;
  initialized: boolean;
}

export type DesktopPreferencesInput = Omit<DesktopPreferences, "initialized">;

export interface Account {
  id: string;
  provider: AccountProvider;
  email: string;
  displayName: string;
  authType: AuthType;
  syncEnabled: boolean;
  lastSyncAt?: string;
  credentialStatus?: "ok" | "session_only" | "missing" | "unknown";
  authStatus?: "connected" | "repairable" | "needs_reconnect" | "needs_password";
}

export interface CreateImapAccountInput {
  displayName: string;
  email: string;
  password: string;
  imapHost: string;
  imapPort: number;
  smtpHost: string;
  smtpPort: number;
}

export interface ImapConnectionTestResult {
  ok: boolean;
  summary: string;
  details: string;
}

export interface SyncResult {
  ok: boolean;
  accountId: string;
  folderCount: number;
  messageCount: number;
  newMessages: NewMailNotification[];
  summary: string;
  details: string;
}

export interface NewMailNotification {
  messageId: string;
  accountId: string;
  sender: string;
  subject: string;
}

export interface MessageListInput {
  limit?: number;
  offset?: number;
  sort?: MessageSortMode;
  accountId?: string;
  folder?: string;
  folderRole?: "inbox" | "sent" | "drafts" | "archive" | "spam" | "trash";
  unread?: boolean;
  flagged?: boolean;
  hasAttachments?: boolean;
  dateFrom?: string;
  dateTo?: string;
}

export interface MessageListResult {
  messages: MessageSummary[];
  nextCursor?: string;
  hasMore: boolean;
  syncStatus: SyncOverview;
}

export interface SyncOverview {
  running: boolean;
  summary: string;
  backfillAvailable: boolean;
}

export interface SyncStatus {
  accountId: string;
  accountEmail: string;
  folderId: string;
  folder: string;
  role?: string;
  lastSyncedAt?: string;
  lastError?: string;
  backfillComplete: boolean;
  highestUid?: number;
  lowestUid?: number;
}

export interface OAuthStartResult {
  ok: boolean;
  provider: string;
  authorizationUrl: string;
  state: string;
  codeVerifier: string;
  redirectUri: string;
  summary: string;
}

export interface OAuthCompleteResult {
  ok: boolean;
  account: Account;
  summary: string;
}

export interface OAuthProviderStatus {
  provider: "gmail" | "microsoft";
  label: string;
  clientIdConfigured: boolean;
  summary: string;
  requiresOwnRegistration?: boolean;
  endUserReady?: boolean;
}

export interface CredentialRepairResult {
  ok: boolean;
  repaired: number;
  missing: number;
  summary: string;
}

export interface SecurityAuditResult {
  ok: boolean;
  summary: string;
  checks: SecurityCheck[];
}

export interface SecurityCheck {
  label: string;
  ok: boolean;
  details: string;
}

export interface SenderStats {
  sender: string;
  senderEmail: string;
  domain: string;
  firstMessageAt?: string;
  lastMessageAt?: string;
  messageCount: number;
  newsletter: boolean;
  trusted: boolean;
  blocked: boolean;
}

export interface MessageSummary {
  id: string;
  accountId: string;
  sender: string;
  senderEmail: string;
  subject: string;
  preview: string;
  bodyText: string;
  bodyHtml?: string;
  date: string;
  folder: string;
  folderRole?: string;
  unread: boolean;
  flagged: boolean;
  hasAttachments: boolean;
  attachments: MessageAttachment[];
  hasNewsletterSignals: boolean;
  renderModeAvailable?: {
    readable: boolean;
    original: boolean;
  };
  renderWarnings?: string[];
}

export interface MessageAttachment {
  filename: string;
  mimeType: string;
  size: number;
  contentId?: string;
  storageRef?: string;
  inline: boolean;
  disposition?: "inline" | "attachment" | "body";
  previewKind?: "image" | "video" | "pdf" | "file";
  downloadPath?: string;
}

export interface NewsletterSubscription {
  id: string;
  sender: string;
  senderEmail: string;
  domain: string;
  messageCount: number;
  lastMessageAt: string;
  unsubscribeMethod: UnsubscribeMethod;
  confidenceScore: number;
  riskScore: number;
  riskLevel: RiskLevel;
  evidence: string[];
}

export interface NewsletterActionResult {
  ok: boolean;
  method: string;
  target: string;
  summary: string;
}

export interface AuditEvent {
  id: string;
  action: string;
  entity: string;
  summary: string;
  createdAt: string;
}

export interface AISettings {
  enabled: boolean;
  accountId?: string;
  provider: AIProvider;
  generationModel: string;
  embeddingModel: string;
  ollamaBaseUrl?: string;
  geminiKeyConfigured: boolean;
}

export interface AISettingsUpdateInput {
  enabled: boolean;
  accountId?: string;
  provider: AIProvider;
  generationModel: string;
  embeddingModel: string;
  ollamaBaseUrl?: string;
}

export interface AIMessageActionInput {
  messageId: string;
  accountId: string;
  includeAttachments: boolean;
}

export type AIEvidence = string | {
  messageId?: string;
  label?: string;
  excerpt: string;
};

export interface AISummaryResult {
  summary: string;
  evidence: AIEvidence[];
  provider?: AIProvider;
  model?: string;
  generatedAt?: string;
}

export interface AIQuickReply {
  id?: string;
  label?: string;
  tone?: string;
  text: string;
}

export interface AIQuickRepliesResult {
  replies: AIQuickReply[];
  provider?: AIProvider;
  model?: string;
}

export interface AISearchInput {
  query: string;
  accountId?: string;
  limit?: number;
}

export interface AISearchHit {
  messageId: string;
  accountId: string;
  sender?: string;
  senderEmail?: string;
  subject?: string;
  date?: string;
  snippet: string;
  evidence: AIEvidence[];
  score?: number;
}

export interface AISearchResult {
  query: string;
  hits: AISearchHit[];
  summary?: string;
}

export interface AIIndexStatus {
  state: AIIndexState;
  accountId?: string;
  processed: number;
  total: number;
  progress?: number;
  message?: string;
  lastError?: string;
  updatedAt?: string;
}

export interface AIIndexInput {
  accountId?: string;
}

export interface AISecretSetInput {
  provider: "gemini";
  secret: string;
}

export interface AISecretDeleteInput {
  provider: "gemini";
}
