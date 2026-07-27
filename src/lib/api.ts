import { invoke } from "@tauri-apps/api/core";
import type {
  Account,
  AIIndexInput,
  AIIndexStatus,
  AIMessageActionInput,
  AIQuickRepliesResult,
  AISearchInput,
  AISearchResult,
  AISecretDeleteInput,
  AISecretSetInput,
  AISettings,
  AISettingsUpdateInput,
  AISummaryResult,
  AuditEvent,
  CreateImapAccountInput,
  DesktopPreferences,
  DesktopPreferencesInput,
  ImapConnectionTestResult,
  MessageListInput,
  MessageListResult,
  MessageSummary,
  NewsletterActionResult,
  NewsletterSubscription,
  OAuthCompleteResult,
  OAuthProviderStatus,
  OAuthStartResult,
  CredentialRepairResult,
  SecurityAuditResult,
  SenderStats,
  SyncStatus,
  SyncResult
} from "../types";

function isTauriRuntime() {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

const messageListRequests = new Map<string, Promise<MessageListResult>>();

async function callOrMock<T>(command: string, fallback: T): Promise<T> {
  try {
    return await invoke<T>(command);
  } catch (error) {
    if (isTauriRuntime()) throw error;
    return fallback;
  }
}

export const api = {
  getDesktopPreferences: () => callOrMock<DesktopPreferences>("desktop_preferences_get", {
    autoSync: true,
    notifications: false,
    privacyMode: true,
    launchAtLogin: false,
    minimizeToTray: true,
    initialized: false
  }),
  updateDesktopPreferences: (input: DesktopPreferencesInput) =>
    invoke<DesktopPreferences>("desktop_preferences_update", { input }),
  listAccounts: () => callOrMock<Account[]>("accounts_list", []),
  listMessages: async (input: MessageListInput = {}) => {
    const normalizedInput: MessageListInput = { ...input };
    if (!normalizedInput.accountId || normalizedInput.accountId === "all") {
      delete normalizedInput.accountId;
    }
    const requestKey = JSON.stringify(normalizedInput, Object.keys(normalizedInput).sort());
    const existing = messageListRequests.get(requestKey);
    if (existing) return existing;
    const request = invoke<MessageListResult>("messages_list", { input: normalizedInput })
      .catch((error) => {
        if (isTauriRuntime()) throw error;
        return { messages: [] as MessageSummary[], hasMore: false, syncStatus: { running: false, summary: "Desktop-App erforderlich", backfillAvailable: false } };
      })
      .finally(() => messageListRequests.delete(requestKey));
    messageListRequests.set(requestKey, request);
    return request;
  },
  getMessage: (messageId: string) =>
    invoke<MessageSummary>("messages_get", { messageId }),
  listNewsletters: () => callOrMock<NewsletterSubscription[]>("newsletter_list", []),
  previewNewsletterUnsubscribe: (subscriptionId: string) =>
    invoke<NewsletterActionResult>("newsletter_unsubscribe_preview", { input: { subscriptionId } }),
  executeNewsletterUnsubscribe: (subscriptionId: string) =>
    invoke<NewsletterActionResult>("newsletter_unsubscribe_execute", { input: { subscriptionId } }),
  listAuditEvents: () => callOrMock<AuditEvent[]>("audit_list", []),
  createImapAccount: (input: CreateImapAccountInput) =>
    invoke<Account>("accounts_create_imap", { input }),
  updateAccountPassword: (accountId: string, password: string) =>
    invoke<Account>("accounts_update_password", { input: { accountId, password } }),
  repairCredentials: () =>
    invoke<CredentialRepairResult>("accounts_repair_credentials"),
  securityAudit: () =>
    invoke<SecurityAuditResult>("accounts_security_audit"),
  removeAccount: (accountId: string) =>
    invoke<{ ok: boolean; summary: string }>("accounts_remove", { input: { accountId } }),
  testImapConnection: (input: CreateImapAccountInput) =>
    callOrMock<ImapConnectionTestResult>("accounts_test_imap_connection", {
      ok: false,
      summary: "Desktop-App erforderlich",
      details: `Der echte IMAP-Test läuft in der Tauri-App. Browser-Vorschau kann ${input.imapHost}:${input.imapPort} nicht nativ testen.`
    }),
  syncAccount: (accountId: string, limit = 30) =>
    invoke<SyncResult>("accounts_sync_now", { accountId, limit }),
  syncDelta: (accountId?: string) => {
    const normalizedAccountId = accountId?.trim();
    return invoke<SyncResult[]>("sync_delta", normalizedAccountId && normalizedAccountId !== "all"
      ? { accountId: normalizedAccountId }
      : {});
  },
  syncBackfill: (input: { accountId?: string; folderId?: string; batchSize?: number } = {}) => {
    const normalizedInput = { ...input };
    if (!normalizedInput.accountId || normalizedInput.accountId === "all") {
      delete normalizedInput.accountId;
    }
    return invoke<SyncResult[]>("sync_backfill", { input: normalizedInput });
  },
  syncStatus: () => callOrMock<SyncStatus[]>("sync_status", []),
  moveMessageLocal: (messageId: string, role: "archive" | "trash" | "spam" | "inbox") =>
    invoke<{ ok: boolean; summary: string }>("messages_move_local", { input: { messageId, role } }),
  markMessageRead: (messageId: string, unread: boolean) =>
    invoke<{ ok: boolean; summary: string }>("messages_mark_read", { input: { messageId, unread } }),
  markMessageFlagged: (messageId: string, flagged: boolean) =>
    invoke<{ ok: boolean; summary: string }>("messages_mark_flagged", { input: { messageId, flagged } }),
  getSenderStats: (messageId: string) =>
    invoke<SenderStats>("sender_stats", { input: { messageId } }),
  blockSender: (messageId: string) =>
    invoke<{ ok: boolean; summary: string }>("sender_block", { input: { messageId } }),
  unblockSender: (messageId: string) =>
    invoke<{ ok: boolean; summary: string }>("sender_unblock", { input: { messageId } }),
  sendEmail: (accountId: string, to: string, subject: string, body: string) =>
    invoke<{ ok: boolean; summary: string }>("smtp_send_email", { input: { accountId, to, subject, body } }),
  getOAuthProviders: () =>
    callOrMock<OAuthProviderStatus[]>("oauth_provider_config", [
      { provider: "gmail", label: "Gmail / Google", clientIdConfigured: false, summary: "Desktop-App erforderlich" },
      { provider: "microsoft", label: "Outlook / Hotmail / Microsoft", clientIdConfigured: false, summary: "Desktop-App erforderlich", requiresOwnRegistration: true, endUserReady: false }
    ]),
  startOAuth: (provider: string, clientId: string, email?: string, redirectUri?: string) =>
    invoke<OAuthStartResult>("oauth_start", { input: { provider, clientId, email, redirectUri } }),
  connectOAuthAccount: (input: { provider: string; email: string; displayName?: string; clientId?: string }) =>
    invoke<OAuthCompleteResult>("oauth_connect_interactive", { input }),
  completeOAuth: (input: { provider: string; email: string; displayName?: string; clientId: string; code: string; codeVerifier: string; redirectUri: string }) =>
    invoke<OAuthCompleteResult>("oauth_complete", { input }),
  getAISettings: () =>
    callOrMock<AISettings>("ai_settings_get", {
      enabled: false,
      provider: "gemini",
      generationModel: "gemini-3.1-flash-lite",
      embeddingModel: "gemini-embedding-001",
      ollamaBaseUrl: "http://127.0.0.1:11434",
      geminiKeyConfigured: false
    }),
  updateAISettings: (input: AISettingsUpdateInput) =>
    invoke<AISettings>("ai_settings_update", { input }),
  generateAISummary: (input: AIMessageActionInput) =>
    invoke<AISummaryResult>("ai_summary_generate", { input }),
  generateAIQuickReplies: (input: AIMessageActionInput) =>
    invoke<AIQuickRepliesResult>("ai_quick_replies_generate", { input }),
  searchAIMessages: (input: AISearchInput) =>
    invoke<AISearchResult>("ai_search_messages", { input }),
  getAIIndexStatus: (input: AIIndexInput = {}) =>
    isTauriRuntime()
      ? invoke<AIIndexStatus>("ai_index_status", { input })
      : Promise.resolve<AIIndexStatus>({
          state: "not_started",
          accountId: input.accountId,
          processed: 0,
          total: 0,
          progress: 0,
          message: "Desktop-App erforderlich"
        }),
  startAIIndex: (input: AIIndexInput = {}) =>
    invoke<AIIndexStatus>("ai_index_start", { input }),
  cancelAIIndex: (input: AIIndexInput = {}) =>
    invoke<AIIndexStatus>("ai_index_cancel", { input }),
  setAISecret: (input: AISecretSetInput) =>
    invoke<void>("ai_secret_set", { input }),
  deleteAISecret: (input: AISecretDeleteInput) =>
    invoke<void>("ai_secret_delete", { input })
};
