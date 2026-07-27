import { startTransition, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties, FormEvent, MouseEvent, ReactNode } from "react";
import { createPortal } from "react-dom";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification
} from "@tauri-apps/plugin-notification";
import {
  disable as disableAutostart,
  enable as enableAutostart,
  isEnabled as isAutostartEnabled
} from "@tauri-apps/plugin-autostart";
import {
  Archive,
  ArchiveX,
  Ban,
  CalendarDays,
  Check,
  ChevronDown,
  Edit3,
  Flag,
  Forward,
  Inbox,
  LayoutList,
  Mail,
  MailOpen,
  MoreHorizontal,
  Paperclip,
  Plus,
  Reply,
  Search,
  Send,
  Settings,
  ShieldCheck,
  SlidersHorizontal,
  Sparkles,
  Star,
  Sun,
  Trash2,
  User,
  UserPlus,
  X
} from "lucide-react";
import { api } from "./lib/api";
import { safeAssetSrc } from "./lib/assets";
import kizumailDarkIcon from "./assets/kizumail-icon-dark.png";
import kizumailLightIcon from "./assets/kizumail-icon-light.png";
import {
  attachmentPreviewKind,
  buildOriginalEmailDocument,
  buildReadableEmailContent,
  extractRemoteEmailMedia,
  isBodyLikeAttachment,
  shortenLink
} from "./lib/mailRendering";
import type {
  Account,
  AIEvidence,
  AIIndexStatus,
  AIProvider,
  AIQuickReply,
  AISearchHit,
  AISearchResult,
  AISettings,
  AISettingsUpdateInput,
  AISummaryResult,
  CreateImapAccountInput,
  ImapConnectionTestResult,
  MessageAttachment,
  MessageListInput,
  MessageListResult,
  MessageSortMode,
  MessageSummary,
  NewsletterSubscription,
  OAuthProviderStatus,
  SecurityAuditResult,
  SenderStats,
  SyncOverview,
  SyncResult,
  SyncStatus,
  UnsubscribeMethod
} from "./types";

type View =
  | "inbox"
  | "starred"
  | "sent"
  | "drafts"
  | "archive"
  | "spam"
  | "trash"
  | "unread"
  | "attachments"
  | "today"
  | "important"
  | "newsletters"
  | "rules"
  | "settings";

const initialAccountForm: CreateImapAccountInput = {
  displayName: "",
  email: "",
  password: "",
  imapHost: "",
  imapPort: 993,
  smtpHost: "",
  smtpPort: 587
};

interface LocalRule {
  id: string;
  name: string;
  contains: string;
  target: string;
  enabled: boolean;
}

interface LocalSettings {
  autoSync: boolean;
  notifications: boolean;
  compactList: boolean;
  privacyMode: boolean;
  launchAtLogin: boolean;
  minimizeToTray: boolean;
}

interface MailContextMenuState {
  x: number;
  y: number;
  messageId: string;
}

const defaultSettings: LocalSettings = {
  autoSync: true,
  notifications: false,
  compactList: false,
  privacyMode: true,
  launchAtLogin: false,
  minimizeToTray: true
};

type ListFilter = "all" | "unread" | "flagged" | "attachments";
type ListDensity = "comfortable" | "compact";
type ReaderMode = "readable" | "original";
type ThemeMode = "system" | "dark" | "light";
type SearchMode = "local" | "ai";
type RequestStatus = "idle" | "loading" | "success" | "error";

interface ComposerDraft {
  accountId: string;
  to: string;
  subject: string;
  body: string;
}

interface AIRequestState<T> {
  status: RequestStatus;
  data: T | null;
  error: string;
}

const emptyAIRequest = <T,>(): AIRequestState<T> => ({ status: "idle", data: null, error: "" });
const DEFAULT_MESSAGE_LIMIT = 100;
const MESSAGE_LOAD_STEP = 100;
const MESSAGE_CACHE_TTL_MS = 5 * 60 * 1000;

const defaultSyncOverview: SyncOverview = {
  running: false,
  summary: "Smart Sync bereit.",
  backfillAvailable: false
};

export function App() {
  const [activeView, setActiveView] = useState<View>("inbox");
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [messages, setMessages] = useState<MessageSummary[]>([]);
  const [selectedMessageDetail, setSelectedMessageDetail] = useState<MessageSummary | null>(null);
  const [isMessageDetailLoading, setIsMessageDetailLoading] = useState(false);
  const [newsletters, setNewsletters] = useState<NewsletterSubscription[]>([]);
  const [selectedMessageId, setSelectedMessageId] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [searchMode, setSearchMode] = useState<SearchMode>("local");
  const [aiSearch, setAISearch] = useState<AIRequestState<AISearchResult>>(() => emptyAIRequest<AISearchResult>());
  const [listFilter, setListFilter] = useState<ListFilter>("all");
  const [listSort, setListSort] = useState<MessageSortMode>("newest");
  const [listAccountFilter, setListAccountFilter] = useState("all");
  const [listDensity, setListDensity] = useState<ListDensity>("comfortable");
  const [isLoadingMoreMessages, setIsLoadingMoreMessages] = useState(false);
  const [hasMoreMessages, setHasMoreMessages] = useState(false);
  const [nextMessageCursor, setNextMessageCursor] = useState<string | undefined>();
  const [syncOverview, setSyncOverview] = useState<SyncOverview>(defaultSyncOverview);
  const [syncStatuses, setSyncStatuses] = useState<SyncStatus[]>([]);
  const [isBackgroundSyncing, setIsBackgroundSyncing] = useState(false);
  const [isAccountModalOpen, setIsAccountModalOpen] = useState(false);
  const [accountForm, setAccountForm] = useState<CreateImapAccountInput>(initialAccountForm);
  const [toast, setToast] = useState("Bereit.");
  const [isSavingAccount, setIsSavingAccount] = useState(false);
  const [isTestingAccount, setIsTestingAccount] = useState(false);
  const [oauthProviders, setOAuthProviders] = useState<OAuthProviderStatus[]>([]);
  const [oauthClientIds, setOAuthClientIds] = useState<Record<string, string>>(() => loadOAuthClientIds());
  const [oauthConnectingProvider, setOAuthConnectingProvider] = useState<string | null>(null);
  const [syncingAccountId, setSyncingAccountId] = useState<string | null>(null);
  const [isComposerOpen, setIsComposerOpen] = useState(false);
  const [composerDraft, setComposerDraft] = useState<ComposerDraft>({ accountId: "", to: "", subject: "", body: "" });
  const [isSendingEmail, setIsSendingEmail] = useState(false);
  const [passwordRecoveryAccountId, setPasswordRecoveryAccountId] = useState<string | null>(null);
  const [recoveryPassword, setRecoveryPassword] = useState("");
  const [isMoreMenuOpen, setIsMoreMenuOpen] = useState(false);
  const [mailContextMenu, setMailContextMenu] = useState<MailContextMenuState | null>(null);
  const [settings, setSettings] = useState<LocalSettings>(() => loadLocalSettings());
  const [rules, setRules] = useState<LocalRule[]>(() => loadLocalRules());
  const [connectionTestResult, setConnectionTestResult] = useState<ImapConnectionTestResult | null>(null);
  const [securityAudit, setSecurityAudit] = useState<SecurityAuditResult | null>(null);
  const [isSecurityBusy, setIsSecurityBusy] = useState(false);
  const [themeMode, setThemeMode] = useState<ThemeMode>(() => loadThemeMode());
  const [resolvedTheme, setResolvedTheme] = useState<"dark" | "light">(() => resolveTheme(loadThemeMode()));
  const [senderStats, setSenderStats] = useState<SenderStats | null>(null);
  const [quickReplyOpen, setQuickReplyOpen] = useState(false);
  const [quickReplies, setQuickReplies] = useState<AIRequestState<AIQuickReply[]>>(() => emptyAIRequest<AIQuickReply[]>());
  const [aiSummary, setAISummary] = useState<AIRequestState<AISummaryResult>>(() => emptyAIRequest<AISummaryResult>());
  const [aiSettings, setAISettings] = useState<AISettings | null>(null);
  const [aiSettingsStatus, setAISettingsStatus] = useState<RequestStatus>("loading");
  const [aiSettingsError, setAISettingsError] = useState("");
  const [isAISettingsSaving, setIsAISettingsSaving] = useState(false);
  const [aiIndexStatus, setAIIndexStatus] = useState<AIIndexStatus | null>(null);
  const [aiIndexError, setAIIndexError] = useState("");
  const [isAIIndexBusy, setIsAIIndexBusy] = useState(false);
  const [mailTheme, setMailTheme] = useState<"dark" | "light">("dark");
  const refreshGeneration = useRef(0);
  const aiSearchGeneration = useRef(0);
  const aiActionGeneration = useRef(0);
  const backgroundSyncInFlight = useRef(false);
  const syncInFlight = useRef<string | null>(null);
  const backfillInFlight = useRef(false);
  const lastSyncRequestedAt = useRef(0);
  const postConnectSyncTimers = useRef(new Set<number>());
  const messageCache = useRef(new Map<string, { result: MessageListResult; cachedAt: number }>());
  const messageDetailCache = useRef(new Map<string, MessageSummary>());
  const messageDetailGeneration = useRef(0);
  const messageQueryRef = useRef({
    sort: "newest" as MessageSortMode,
    accountFilter: "all",
    activeView: "inbox" as View,
    listFilter: "all" as ListFilter
  });
  messageQueryRef.current = {
    sort: listSort,
    accountFilter: listAccountFilter,
    activeView,
    listFilter
  };

  useEffect(() => {
    void refreshOAuthProviders();
  }, []);

  useEffect(() => {
    if (!isTauriDesktop()) return;
    let cancelled = false;
    Promise.all([api.getDesktopPreferences(), isAutostartEnabled()])
      .then(async ([desktopPreferences, autostartEnabled]) => {
        if (cancelled) return;
        if (desktopPreferences.initialized) {
          persistSettings({
            ...settings,
            autoSync: desktopPreferences.autoSync,
            notifications: desktopPreferences.notifications,
            privacyMode: desktopPreferences.privacyMode,
            launchAtLogin: autostartEnabled,
            minimizeToTray: desktopPreferences.minimizeToTray
          });
          return;
        }
        await api.updateDesktopPreferences({
          autoSync: settings.autoSync,
          notifications: settings.notifications,
          privacyMode: settings.privacyMode,
          launchAtLogin: autostartEnabled,
          minimizeToTray: settings.minimizeToTray
        });
        if (!cancelled && autostartEnabled !== settings.launchAtLogin) {
          persistSettings({ ...settings, launchAtLogin: autostartEnabled });
        }
      })
      .catch((error) => {
        if (!cancelled) {
          setToast(`Desktop-Einstellungen konnten nicht geladen werden: ${errorMessage(error)}`);
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!settings.autoSync || !accounts.length) return;
    let lastVisibleRefresh = Date.now();
    const refreshWhenVisible = () => {
      if (document.visibilityState !== "visible") return;
      if (Date.now() - lastVisibleRefresh < 15_000) return;
      lastVisibleRefresh = Date.now();
      void Promise.all([refreshMessages(), refreshSyncStatuses()]);
    };
    document.addEventListener("visibilitychange", refreshWhenVisible);
    window.addEventListener("focus", refreshWhenVisible);
    return () => {
      document.removeEventListener("visibilitychange", refreshWhenVisible);
      window.removeEventListener("focus", refreshWhenVisible);
    };
  }, [settings.autoSync, accounts.length]);

  useEffect(() => {
    let cancelled = false;
    setAISettingsStatus("loading");
    Promise.allSettled([api.getAISettings(), api.getAIIndexStatus()]).then(([settingsResult, indexResult]) => {
      if (cancelled) return;
      if (settingsResult.status === "fulfilled") {
        setAISettings(settingsResult.value);
        setAISettingsStatus("success");
        setAISettingsError("");
      } else {
        setAISettingsStatus("error");
        setAISettingsError(errorMessage(settingsResult.reason));
      }
      if (indexResult.status === "fulfilled") {
        setAIIndexStatus(indexResult.value);
        setAIIndexError("");
      } else {
        setAIIndexError(errorMessage(indexResult.reason));
      }
    });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (aiIndexStatus?.state !== "running") return;
    let cancelled = false;
    let timer: number | null = null;
    async function pollIndexStatus() {
      try {
        const status = await api.getAIIndexStatus({ accountId: aiSettings?.accountId });
        if (cancelled) return;
        setAIIndexStatus(status);
        setAIIndexError("");
        if (status.state === "running") {
          timer = window.setTimeout(() => void pollIndexStatus(), 1200);
        }
      } catch (error) {
        if (!cancelled) {
          setAIIndexError(errorMessage(error));
          timer = window.setTimeout(() => void pollIndexStatus(), 2000);
        }
      }
    }
    timer = window.setTimeout(() => void pollIndexStatus(), 1200);
    return () => {
      cancelled = true;
      if (timer !== null) window.clearTimeout(timer);
    };
  }, [aiIndexStatus?.state, aiSettings?.accountId]);

  useEffect(() => {
    const apply = () => {
      const nextTheme = resolveTheme(themeMode);
      setResolvedTheme(nextTheme);
      document.documentElement.dataset.theme = nextTheme;
      document.documentElement.dataset.themeMode = themeMode;
      localStorage.setItem("kizumail.theme", themeMode);
    };
    apply();
    const media = window.matchMedia("(prefers-color-scheme: light)");
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [themeMode]);

  useEffect(() => {
    if (!settings.notifications || !isTauriDesktop()) return;
    let cancelled = false;
    void isPermissionGranted().then((granted) => {
      if (!cancelled && !granted) {
        persistSettings({ ...settings, notifications: false });
        setToast("Desktop-Benachrichtigungen sind in Windows nicht freigegeben.");
      }
    }).catch((error) => {
      if (!cancelled) setToast(`Benachrichtigungsstatus konnte nicht geprüft werden: ${errorMessage(error)}`);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    aiActionGeneration.current += 1;
    setQuickReplyOpen(false);
    setQuickReplies(emptyAIRequest<AIQuickReply[]>());
    setAISummary(emptyAIRequest<AISummaryResult>());
    setMailTheme("dark");
    if (!selectedMessageId) {
      setSenderStats(null);
      return;
    }
    let cancelled = false;
    api.getSenderStats(selectedMessageId)
      .then((stats) => {
        if (!cancelled) setSenderStats(stats);
      })
      .catch(() => {
        if (!cancelled) setSenderStats(null);
      });
    return () => {
      cancelled = true;
    };
  }, [selectedMessageId]);

  useEffect(() => {
    const generation = ++messageDetailGeneration.current;
    if (!selectedMessageId) {
      setSelectedMessageDetail(null);
      setIsMessageDetailLoading(false);
      return;
    }
    const cached = messageDetailCache.current.get(selectedMessageId);
    if (cached) {
      setSelectedMessageDetail(cached);
      setIsMessageDetailLoading(false);
      return;
    }
    setSelectedMessageDetail(null);
    setIsMessageDetailLoading(true);
    api.getMessage(selectedMessageId)
      .then((message) => {
        messageDetailCache.current.set(message.id, message);
        if (generation === messageDetailGeneration.current) {
          setSelectedMessageDetail(message);
        }
      })
      .catch((error) => {
        if (generation === messageDetailGeneration.current) {
          setToast(`Nachricht konnte nicht geöffnet werden: ${errorMessage(error)}`);
        }
      })
      .finally(() => {
        if (generation === messageDetailGeneration.current) {
          setIsMessageDetailLoading(false);
        }
      });
  }, [selectedMessageId]);

  useEffect(() => {
    const queryState = messageQueryRef.current;
    const cached = messageCache.current.get(messageCacheKey(queryState));
    if (cached && Date.now() - cached.cachedAt < MESSAGE_CACHE_TTL_MS) {
      startTransition(() => applyMessageResult(cached.result));
    }
    void refreshMessages();
  }, [listSort, listAccountFilter, activeView, listFilter]);

  useEffect(() => {
    void refreshMetadata();
  }, []);

  useEffect(() => {
    aiSearchGeneration.current += 1;
    setAISearch(emptyAIRequest<AISearchResult>());
  }, [listAccountFilter]);

  useEffect(() => {
    if (!settings.autoSync || !accounts.length || isAccountModalOpen || oauthConnectingProvider) return;
    let cancelled = false;

    async function refreshBackgroundState() {
      if (cancelled || backgroundSyncInFlight.current) return;
      backgroundSyncInFlight.current = true;
      setIsBackgroundSyncing(true);
      try {
        if (document.visibilityState === "visible") {
          await Promise.all([refreshMessages(), refreshSyncStatuses()]);
        }
      } catch (error) {
        if (!cancelled) setToast(error instanceof Error ? error.message : String(error));
      } finally {
        backgroundSyncInFlight.current = false;
        if (!cancelled) setIsBackgroundSyncing(false);
      }
    }

    const startupTimer = window.setTimeout(() => void refreshBackgroundState(), 15_000);
    const interval = window.setInterval(() => void refreshBackgroundState(), 2 * 60_000);
    return () => {
      cancelled = true;
      window.clearTimeout(startupTimer);
      window.clearInterval(interval);
    };
  }, [settings.autoSync, accounts.length, isAccountModalOpen, oauthConnectingProvider]);

  useEffect(() => {
    if (!isTauriDesktop()) return;
    let unlisten: UnlistenFn | undefined;
    let refreshTimer: number | undefined;
    void listen<{ accountId: string; messageCount: number }>("sync://messages-changed", () => {
      messageCache.current.clear();
      if (refreshTimer !== undefined) window.clearTimeout(refreshTimer);
      refreshTimer = window.setTimeout(() => {
        if (document.visibilityState === "visible") {
          void Promise.all([refreshMessages(), refreshSyncStatuses()]);
        }
      }, 250);
    }).then((dispose) => {
      unlisten = dispose;
    });
    return () => {
      if (refreshTimer !== undefined) window.clearTimeout(refreshTimer);
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    function closeMenus() {
      setMailContextMenu(null);
      setIsMoreMenuOpen(false);
    }

    function closeOnEscape(event: KeyboardEvent) {
      if (event.key === "Escape") closeMenus();
    }

    window.addEventListener("click", closeMenus);
    window.addEventListener("keydown", closeOnEscape);
    return () => {
      window.removeEventListener("click", closeMenus);
      window.removeEventListener("keydown", closeOnEscape);
    };
  }, []);

  function applyMessageResult(result: MessageListResult, append = false) {
    startTransition(() => {
      setMessages((current) => append ? mergeMessages(current, result.messages) : result.messages);
      setHasMoreMessages(result.hasMore);
      setNextMessageCursor(result.nextCursor);
      setSyncOverview(result.syncStatus);
      setSelectedMessageId((current) => {
        if (append) return current ?? result.messages[0]?.id ?? null;
        const available = result.messages;
        return current && available.some((message) => message.id === current)
          ? current
          : available[0]?.id ?? null;
      });
    });
  }

  async function refreshMessages(
    limitOverride = DEFAULT_MESSAGE_LIMIT,
    sortOverride?: MessageSortMode,
    offset = 0,
    append = false
  ) {
    const generation = ++refreshGeneration.current;
    try {
      const queryState = messageQueryRef.current;
      const effectiveSort = sortOverride ?? queryState.sort;
      const messageInput = messageListInputForView(queryState.activeView, queryState.listFilter);
      const nextMessageResult = await api.listMessages({
        ...messageInput,
        limit: limitOverride,
        offset,
        sort: effectiveSort,
        accountId: queryState.accountFilter === "all" ? undefined : queryState.accountFilter
      });
      if (generation !== refreshGeneration.current) return false;
      if (offset === 0) {
        messageCache.current.set(messageCacheKey(queryState), {
          result: nextMessageResult,
          cachedAt: Date.now()
        });
      }
      applyMessageResult(nextMessageResult, append);
      return true;
    } catch (error) {
      if (generation === refreshGeneration.current) {
        setToast(error instanceof Error ? error.message : String(error));
      }
      return false;
    }
  }

  async function refreshSyncStatuses() {
    try {
      setSyncStatuses(await api.syncStatus());
    } catch (error) {
      setToast(errorMessage(error));
    }
  }

  async function refreshMetadata() {
    try {
      const [nextAccounts, nextNewsletters, nextSyncStatuses] = await Promise.all([
        api.listAccounts(),
        api.listNewsletters(),
        api.syncStatus()
      ]);
      setAccounts(nextAccounts);
      setNewsletters(nextNewsletters);
      setSyncStatuses(nextSyncStatuses);
      if (messageQueryRef.current.accountFilter !== "all" && !nextAccounts.some(
        (account) => account.id === messageQueryRef.current.accountFilter
      )) {
        setListAccountFilter("all");
      }
      return true;
    } catch (error) {
      setToast(errorMessage(error));
      return false;
    }
  }

  async function refreshData(limitOverride = DEFAULT_MESSAGE_LIMIT, sortOverride?: MessageSortMode) {
    messageCache.current.clear();
    const [metadataOk, messagesOk] = await Promise.all([
      refreshMetadata(),
      refreshMessages(limitOverride, sortOverride)
    ]);
    return metadataOk && messagesOk;
  }

  async function refreshOAuthProviders() {
    const providers = await api.getOAuthProviders();
    setOAuthProviders(providers);
  }

  function shouldAskForPassword(accountId: string, message: string) {
    const account = accounts.find((item) => item.id === accountId);
    const normalized = message.toLowerCase();
    return account?.authType === "password" && (
      normalized.includes("credential manager") ||
      normalized.includes("passwort fehlt") ||
      normalized.includes("passwort")
    );
  }

  function recoverCredentialsFromSyncResults(results: SyncResult[]) {
    const recoverable = results.find((result) => !result.ok && shouldAskForPassword(result.accountId, result.details));
    if (recoverable) setPasswordRecoveryAccountId(recoverable.accountId);
  }

  function startPostConnectSync(accountId: string) {
    const timer = window.setTimeout(() => {
      postConnectSyncTimers.current.delete(timer);
      void runPostConnectSync(accountId);
    }, 900);
    postConnectSyncTimers.current.add(timer);
  }

  async function runDeltaSync(accountId?: string) {
    if (syncInFlight.current || backfillInFlight.current) return null;
    const syncKey = accountId ?? "all";
    syncInFlight.current = syncKey;
    lastSyncRequestedAt.current = Date.now();
    setSyncingAccountId(syncKey);
    try {
      const results = await api.syncDelta(accountId);
      await sendNewMailNotifications(results, settings);
      return results;
    } finally {
      syncInFlight.current = null;
      setSyncingAccountId(null);
    }
  }

  async function runPostConnectSync(accountId: string) {
    if (syncInFlight.current || backfillInFlight.current) return;
    setToast("Konto verbunden. Inbox-Sync läuft im Hintergrund...");
    try {
      const results = await runDeltaSync(accountId);
      if (results) {
        let historySyncFailed = false;
        backfillInFlight.current = true;
        try {
          await api.syncBackfill({
            accountId,
            batchSize: MESSAGE_LOAD_STEP
          });
        } catch {
          historySyncFailed = true;
        } finally {
          backfillInFlight.current = false;
        }
        await refreshData();
        recoverCredentialsFromSyncResults(results);
        setToast(syncResultMessage(
          results,
          historySyncFailed
            ? "Konto verbunden; die Historie wird beim nächsten Hintergrundlauf fortgesetzt."
            : "Konto verbunden, aktuelle und ältere Mails wurden synchronisiert."
        ));
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setToast(message);
      if (shouldAskForPassword(accountId, message)) {
        setPasswordRecoveryAccountId(accountId);
      }
    }
  }

  async function handleCreateAccount(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setIsSavingAccount(true);
    setToast("Konto wird lokal angelegt...");

    try {
      const normalizedForm = applyProviderPreset(accountForm);
      const created = await api.createImapAccount(normalizedForm);
      setAccounts((current) => [...current, created]);
      setAccountForm(initialAccountForm);
      setIsAccountModalOpen(false);
      await refreshData();
      startPostConnectSync(created.id);
    } catch (error) {
      setToast(error instanceof Error ? error.message : String(error));
    } finally {
      setIsSavingAccount(false);
    }
  }

  async function handleTestConnection() {
    setIsTestingAccount(true);
    setConnectionTestResult(null);
    setToast("IMAP-Verbindung wird getestet...");

    try {
      const result = await api.testImapConnection(applyProviderPreset(accountForm));
      setConnectionTestResult(result);
      setToast(result.summary);
    } catch (error) {
      const summary = error instanceof Error ? error.message : String(error);
      setConnectionTestResult({
        ok: false,
        summary: "IMAP-Test fehlgeschlagen",
        details: summary
      });
      setToast("IMAP-Test fehlgeschlagen.");
    } finally {
      setIsTestingAccount(false);
    }
  }

  async function handleOAuthConnect(provider: "gmail" | "microsoft") {
    const email = accountForm.email.trim();
    if (!email) {
      setToast("Bitte zuerst die E-Mail-Adresse eingeben.");
      return;
    }
    setOAuthConnectingProvider(provider);
    setToast(provider === "gmail" ? "Gmail-Anmeldung wird geöffnet..." : "Microsoft-Anmeldung wird geöffnet...");
    try {
      const result = await api.connectOAuthAccount({
        provider,
        email,
        displayName: accountForm.displayName,
        clientId: oauthClientIds[provider]?.trim() || undefined
      });
      setAccounts((current) => {
        const withoutExisting = current.filter((account) => account.id !== result.account.id);
        return [...withoutExisting, result.account];
      });
      setAccountForm(initialAccountForm);
      setIsAccountModalOpen(false);
      await refreshData();
      startPostConnectSync(result.account.id);
    } catch (error) {
      setToast(error instanceof Error ? error.message : String(error));
    } finally {
      setOAuthConnectingProvider(null);
    }
  }

  async function handleSyncAccount(accountId: string) {
    if (syncInFlight.current || backfillInFlight.current) return;
    setToast("Delta-Sync läuft...");
    try {
      const results = await runDeltaSync(accountId);
      if (results) {
        await refreshData();
        recoverCredentialsFromSyncResults(results);
        setToast(syncResultMessage(results, "Delta-Sync abgeschlossen."));
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setToast(message);
      if (shouldAskForPassword(accountId, message)) {
        setPasswordRecoveryAccountId(accountId);
      }
    }
  }

  async function handleSyncAllAccounts() {
    if (!accounts.length || syncInFlight.current || backfillInFlight.current || isBackgroundSyncing) return;
    setToast("Alle Konten werden synchronisiert...");
    try {
      const results = await runDeltaSync();
      if (results) {
        await refreshData();
        recoverCredentialsFromSyncResults(results);
        setToast(syncResultMessage(results, `${results.length} Konten synchronisiert.`));
      }
    } catch (error) {
      setToast(error instanceof Error ? error.message : String(error));
    }
  }

  async function handleLoadMoreMessages() {
    if (backfillInFlight.current || syncInFlight.current) return;
    backfillInFlight.current = true;
    setIsLoadingMoreMessages(true);
    try {
      const offset = Number(nextMessageCursor ?? messages.length);
      if (!hasMoreMessages) {
        await api.syncBackfill({
          accountId: listAccountFilter === "all" ? undefined : listAccountFilter,
          batchSize: MESSAGE_LOAD_STEP
        });
        messageCache.current.clear();
      }
      await refreshMessages(
        MESSAGE_LOAD_STEP,
        listSort,
        offset,
        true
      );
      setToast("Ältere Mails wurden paketweise nachgeladen.");
    } catch (error) {
      setToast(error instanceof Error ? error.message : String(error));
    } finally {
      backfillInFlight.current = false;
      setIsLoadingMoreMessages(false);
    }
  }

  async function handleFullSync() {
    if (backfillInFlight.current || syncInFlight.current) return;
    backfillInFlight.current = true;
    setIsLoadingMoreMessages(true);
    setToast("Mehrere Backfill-Pakete werden geladen...");
    try {
      for (let index = 0; index < 4; index += 1) {
        await api.syncBackfill({
          accountId: listAccountFilter === "all" ? undefined : listAccountFilter,
          batchSize: 250
        });
      }
      await refreshData(DEFAULT_MESSAGE_LIMIT, listSort);
      setToast("Historie in größeren Paketen nachgeladen.");
    } catch (error) {
      setToast(error instanceof Error ? error.message : String(error));
    } finally {
      backfillInFlight.current = false;
      setIsLoadingMoreMessages(false);
    }
  }

  async function handleRecoverPassword(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!passwordRecoveryAccountId) return;
    setToast("Passwort wird aktualisiert...");
    try {
      await api.updateAccountPassword(passwordRecoveryAccountId, recoveryPassword);
      setRecoveryPassword("");
      setPasswordRecoveryAccountId(null);
      const results = await runDeltaSync(passwordRecoveryAccountId);
      if (results) {
        await refreshData();
        recoverCredentialsFromSyncResults(results);
        setToast(syncResultMessage(results, "Passwort aktualisiert und Delta-Sync abgeschlossen."));
      }
    } catch (error) {
      setToast(error instanceof Error ? error.message : String(error));
    }
  }

  async function updateSettings(next: LocalSettings) {
    let effectiveSettings = next;
    if (next.notifications && !settings.notifications && isTauriDesktop()) {
      try {
        let granted = await isPermissionGranted();
        if (!granted) {
          granted = (await requestPermission()) === "granted";
        }
        if (!granted) {
          effectiveSettings = { ...next, notifications: false };
          await api.updateDesktopPreferences(desktopPreferencesInput(effectiveSettings));
          persistSettings(effectiveSettings);
          setToast("Desktop-Benachrichtigungen wurden nicht freigegeben.");
          return;
        }
        sendNotification({
          title: "KizuMail",
          body: "Desktop-Benachrichtigungen sind aktiviert."
        });
      } catch (error) {
        effectiveSettings = { ...next, notifications: false };
        try {
          await api.updateDesktopPreferences(desktopPreferencesInput(effectiveSettings));
        } catch {
          // The original notification error is more actionable for the user.
        }
        persistSettings(effectiveSettings);
        setToast(`Desktop-Benachrichtigungen konnten nicht aktiviert werden: ${errorMessage(error)}`);
        return;
      }
    }
    if (isTauriDesktop()) {
      try {
        if (effectiveSettings.launchAtLogin !== settings.launchAtLogin) {
          if (effectiveSettings.launchAtLogin) {
            await enableAutostart();
          } else {
            await disableAutostart();
          }
        }
        await api.updateDesktopPreferences(desktopPreferencesInput(effectiveSettings));
      } catch (error) {
        setToast(`Desktop-Einstellungen konnten nicht gespeichert werden: ${errorMessage(error)}`);
        return;
      }
    }
    persistSettings(effectiveSettings);
  }

  function persistSettings(next: LocalSettings) {
    setSettings(next);
    localStorage.setItem("kizumail.settings", JSON.stringify(next));
  }

  function saveRules(next: LocalRule[]) {
    setRules(next);
    localStorage.setItem("kizumail.rules", JSON.stringify(next));
  }

  function updateOAuthClientId(provider: string, clientId: string) {
    const next = { ...oauthClientIds, [provider]: clientId };
    setOAuthClientIds(next);
    localStorage.setItem("kizumail.oauthClientIds", JSON.stringify(next));
  }

  async function handleRepairCredentials() {
    setIsSecurityBusy(true);
    setToast("Credential Manager wird geprüft...");
    try {
      const result = await api.repairCredentials();
      await refreshData();
      setToast(result.summary);
    } catch (error) {
      setToast(error instanceof Error ? error.message : String(error));
    } finally {
      setIsSecurityBusy(false);
    }
  }

  async function handleSecurityAudit() {
    setIsSecurityBusy(true);
    setToast("Sicherheitsprüfung läuft...");
    try {
      const result = await api.securityAudit();
      setSecurityAudit(result);
      setToast(result.summary);
    } catch (error) {
      setToast(error instanceof Error ? error.message : String(error));
    } finally {
      setIsSecurityBusy(false);
    }
  }

  async function handleRemoveAccount(accountId: string) {
    const account = accounts.find((item) => item.id === accountId);
    const confirmed = window.confirm(`Konto ${account?.email ?? ""} wirklich entfernen? Lokale Mails, Ordner und Credentials werden gelöscht.`);
    if (!confirmed) return;
    setToast("Konto wird entfernt...");
    try {
      const result = await api.removeAccount(accountId);
      setSelectedMessageId(null);
      await refreshData();
      setToast(result.summary);
    } catch (error) {
      setToast(error instanceof Error ? error.message : String(error));
    }
  }

  async function handleSendEmail(input: { accountId: string; to: string; subject: string; body: string }) {
    setIsSendingEmail(true);
    setToast("SMTP-Versand läuft...");
    try {
      const result = await api.sendEmail(input.accountId, input.to, input.subject, input.body);
      setIsComposerOpen(false);
      setToast(result.summary);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setToast(message);
      if (shouldAskForPassword(input.accountId, message)) {
        setPasswordRecoveryAccountId(input.accountId);
      }
    } finally {
      setIsSendingEmail(false);
    }
  }

  async function handleMoveSelected(role: "archive" | "trash" | "spam" | "inbox") {
    if (!selectedMessage) return;
    await moveMessage(selectedMessage, role);
  }

  async function moveMessage(message: MessageSummary, role: "archive" | "trash" | "spam" | "inbox") {
    setIsMoreMenuOpen(false);
    setMailContextMenu(null);
    try {
      const result = await api.moveMessageLocal(message.id, role);
      setSelectedMessageId(null);
      await refreshData();
      setToast(result.summary);
    } catch (error) {
      setToast(error instanceof Error ? error.message : String(error));
    }
  }

  async function handleToggleRead() {
    if (!selectedMessage) return;
    await setMessageUnread(selectedMessage, !selectedMessage.unread, true);
  }

  async function handleToggleFlagged() {
    if (!selectedMessage) return;
    await setMessageFlagged(selectedMessage, !selectedMessage.flagged);
  }

  function cycleTheme() {
    setThemeMode(resolvedTheme === "light" ? "dark" : "light");
  }

  function openComposer(draft: Partial<ComposerDraft> = {}) {
    setComposerDraft({
      accountId: draft.accountId ?? selectedMessage?.accountId ?? accounts[0]?.id ?? "",
      to: draft.to ?? "",
      subject: draft.subject ?? "",
      body: draft.body ?? ""
    });
    setIsComposerOpen(true);
  }

  function openReplyComposer(body = "") {
    if (!selectedMessage) return;
    const subject = selectedMessage.subject.toLowerCase().startsWith("re:")
      ? selectedMessage.subject
      : `Re: ${selectedMessage.subject}`;
    openComposer({
      accountId: selectedMessage.accountId,
      to: selectedMessage.senderEmail,
      subject,
      body
    });
  }

  function changeSearchMode(mode: SearchMode) {
    setSearchMode(mode);
    setAISearch(emptyAIRequest<AISearchResult>());
  }

  function clearSearch() {
    aiSearchGeneration.current += 1;
    setQuery("");
    setAISearch(emptyAIRequest<AISearchResult>());
  }

  async function handleAISearch() {
    const normalizedQuery = query.trim();
    if (!normalizedQuery) {
      setAISearch({ status: "error", data: null, error: "Bitte einen Suchbegriff eingeben." });
      return;
    }
    if (!aiSettings?.enabled) {
      setAISearch({ status: "error", data: null, error: "KI ist deaktiviert. Aktiviere sie zuerst in den Einstellungen." });
      return;
    }
    const generation = ++aiSearchGeneration.current;
    setAISearch({ status: "loading", data: null, error: "" });
    try {
      const result = await api.searchAIMessages({
        query: normalizedQuery,
        accountId: listAccountFilter === "all" ? aiSettings.accountId : listAccountFilter,
        limit: 30
      });
      if (generation === aiSearchGeneration.current) {
        setAISearch({ status: "success", data: result, error: "" });
      }
    } catch (error) {
      if (generation === aiSearchGeneration.current) {
        setAISearch({ status: "error", data: null, error: errorMessage(error) });
      }
    }
  }

  function handleSelectAISearchHit(hit: AISearchHit) {
    const message = messageById.get(hit.messageId);
    if (message) {
      void handleSelectMessage(message);
      return;
    }
    setToast("Der belegte Treffer ist nicht im aktuell geladenen Mail-Ausschnitt. Lade mehr Historie für die Detailansicht.");
  }

  async function handleAISummary(includeAttachments: boolean) {
    if (!selectedMessage || !aiSettings?.enabled) return;
    const messageId = selectedMessage.id;
    const generation = ++aiActionGeneration.current;
    setAISummary({ status: "loading", data: null, error: "" });
    try {
      const result = await api.generateAISummary({
        messageId: selectedMessage.id,
        accountId: selectedMessage.accountId,
        includeAttachments
      });
      if (generation === aiActionGeneration.current && selectedMessageId === messageId) {
        setAISummary({ status: "success", data: result, error: "" });
      }
    } catch (error) {
      if (generation === aiActionGeneration.current && selectedMessageId === messageId) {
        setAISummary({ status: "error", data: null, error: errorMessage(error) });
      }
    }
  }

  async function handleAIQuickReplies(includeAttachments: boolean) {
    if (!selectedMessage || !aiSettings?.enabled) return;
    const messageId = selectedMessage.id;
    const generation = ++aiActionGeneration.current;
    setQuickReplies({ status: "loading", data: null, error: "" });
    try {
      const result = await api.generateAIQuickReplies({
        messageId: selectedMessage.id,
        accountId: selectedMessage.accountId,
        includeAttachments
      });
      if (generation === aiActionGeneration.current && selectedMessageId === messageId) {
        setQuickReplies({ status: "success", data: result.replies.slice(0, 3), error: "" });
      }
    } catch (error) {
      if (generation === aiActionGeneration.current && selectedMessageId === messageId) {
        setQuickReplies({ status: "error", data: null, error: errorMessage(error) });
      }
    }
  }

  function updateQuickReply(index: number, text: string) {
    setQuickReplies((current) => current.data
      ? { ...current, data: current.data.map((reply, replyIndex) => replyIndex === index ? { ...reply, text } : reply) }
      : current);
  }

  async function handleSaveAISettings(input: AISettingsUpdateInput): Promise<AISettings> {
    setIsAISettingsSaving(true);
    setAISettingsError("");
    try {
      const saved = await api.updateAISettings(input);
      setAISettings(saved);
      setAISettingsStatus("success");
      setToast("KI-Einstellungen gespeichert.");
      const status = await api.getAIIndexStatus({ accountId: saved.accountId });
      setAIIndexStatus(status);
      setAIIndexError("");
      return saved;
    } catch (error) {
      const message = errorMessage(error);
      setAISettingsError(message);
      setToast(message);
      throw error;
    } finally {
      setIsAISettingsSaving(false);
    }
  }

  async function handleSetAISecret(secret: string) {
    setIsAISettingsSaving(true);
    setAISettingsError("");
    try {
      await api.setAISecret({ provider: "gemini", secret });
      const saved = await api.getAISettings();
      setAISettings(saved);
      setAISettingsStatus("success");
      setToast("Gemini-Key sicher im Backend gespeichert.");
    } catch (error) {
      const message = errorMessage(error);
      setAISettingsError(message);
      setToast(message);
      throw error;
    } finally {
      setIsAISettingsSaving(false);
    }
  }

  async function handleDeleteAISecret() {
    setIsAISettingsSaving(true);
    setAISettingsError("");
    try {
      await api.deleteAISecret({ provider: "gemini" });
      const saved = await api.getAISettings();
      setAISettings(saved);
      setAISettingsStatus("success");
      setToast("Gemini-Key aus dem Backend gelöscht.");
    } catch (error) {
      const message = errorMessage(error);
      setAISettingsError(message);
      setToast(message);
      throw error;
    } finally {
      setIsAISettingsSaving(false);
    }
  }

  async function handleStartAIIndex() {
    if (!aiSettings?.enabled) return;
    setIsAIIndexBusy(true);
    setAIIndexError("");
    try {
      const status = await api.startAIIndex({ accountId: aiSettings.accountId });
      setAIIndexStatus(status);
      setToast("KI-Indexierung gestartet.");
    } catch (error) {
      const message = errorMessage(error);
      setAIIndexError(message);
      setToast(message);
    } finally {
      setIsAIIndexBusy(false);
    }
  }

  async function handleCancelAIIndex() {
    setIsAIIndexBusy(true);
    setAIIndexError("");
    try {
      const status = await api.cancelAIIndex({ accountId: aiSettings?.accountId });
      setAIIndexStatus(status);
      setToast("Indexierung abgebrochen.");
    } catch (error) {
      const message = errorMessage(error);
      setAIIndexError(message);
      setToast(message);
    } finally {
      setIsAIIndexBusy(false);
    }
  }

  async function handleToggleSenderBlocked(message?: MessageSummary | null) {
    const targetMessage = message ?? selectedMessage;
    if (!targetMessage) return;
    setIsMoreMenuOpen(false);
    setMailContextMenu(null);
    try {
      const activeStats = targetMessage.id === selectedMessageId ? senderStats : null;
      const result = activeStats?.blocked
        ? await api.unblockSender(targetMessage.id)
        : await api.blockSender(targetMessage.id);
      setToast(result.summary);
      const stats = await api.getSenderStats(targetMessage.id);
      setSenderStats(stats);
      await refreshData();
      if (!activeStats?.blocked) {
        setSelectedMessageId(null);
      }
    } catch (error) {
      setToast(error instanceof Error ? error.message : String(error));
    }
  }

  async function setMessageUnread(message: MessageSummary, unread: boolean, showToast = false) {
    setIsMoreMenuOpen(false);
    setMailContextMenu(null);
    refreshGeneration.current += 1;
    setMessages((current) => current.map((item) => (item.id === message.id ? { ...item, unread } : item)));
    setSelectedMessageDetail((current) => current?.id === message.id ? { ...current, unread } : current);
    const cachedDetail = messageDetailCache.current.get(message.id);
    if (cachedDetail) messageDetailCache.current.set(message.id, { ...cachedDetail, unread });
    try {
      const result = await api.markMessageRead(message.id, unread);
      if (showToast) setToast(result.summary);
    } catch (error) {
      setMessages((current) => current.map((item) => (item.id === message.id ? { ...item, unread: message.unread } : item)));
      setSelectedMessageDetail((current) => current?.id === message.id ? { ...current, unread: message.unread } : current);
      await refreshData();
      setToast(error instanceof Error ? error.message : String(error));
    }
  }

  async function setMessageFlagged(message: MessageSummary, flagged: boolean) {
    setIsMoreMenuOpen(false);
    setMailContextMenu(null);
    const previousMessages = messages;
    setMessages((current) => current.map((item) => (item.id === message.id ? { ...item, flagged } : item)));
    setSelectedMessageDetail((current) => current?.id === message.id ? { ...current, flagged } : current);
    const cachedDetail = messageDetailCache.current.get(message.id);
    if (cachedDetail) messageDetailCache.current.set(message.id, { ...cachedDetail, flagged });
    try {
      const result = await api.markMessageFlagged(message.id, flagged);
      setToast(result.summary);
    } catch (error) {
      setMessages(previousMessages);
      setSelectedMessageDetail((current) => current?.id === message.id ? { ...current, flagged: message.flagged } : current);
      if (cachedDetail) messageDetailCache.current.set(message.id, cachedDetail);
      await refreshData();
      setToast(error instanceof Error ? error.message : String(error));
    }
  }

  async function handleSelectMessage(message: MessageSummary) {
    setSelectedMessageId(message.id);
    setMailContextMenu(null);
    setIsMoreMenuOpen(false);
    if (message.unread) {
      await setMessageUnread(message, false);
    }
  }

  function openMessageContextMenu(event: MouseEvent, message: MessageSummary) {
    event.preventDefault();
    event.stopPropagation();
    setSelectedMessageId(message.id);
    setMailContextMenu({ x: event.clientX, y: event.clientY, messageId: message.id });
  }

  const hasAccounts = accounts.length > 0;
  const unreadCount = messages.filter((message) => message.unread).length;
  const isWorkspaceView = activeView === "rules" || activeView === "settings" || activeView === "newsletters";
  const selectedListMessage = isWorkspaceView ? null : messages.find((message) => message.id === selectedMessageId) ?? null;
  const selectedMessage = selectedMessageDetail?.id === selectedMessageId && selectedListMessage
    ? {
        ...selectedMessageDetail,
        unread: selectedListMessage.unread,
        flagged: selectedListMessage.flagged,
        folder: selectedListMessage.folder,
        folderRole: selectedListMessage.folderRole
      }
    : selectedListMessage;
  const contextMessage = mailContextMenu ? messages.find((message) => message.id === mailContextMenu.messageId) ?? null : null;
  const aiEnabled = aiSettingsStatus === "success" && Boolean(aiSettings?.enabled);
  const accountById = useMemo(
    () => new Map<string, Account>(accounts.map((account) => [account.id, account])),
    [accounts]
  );
  const messageById = useMemo(
    () => new Map<string, MessageSummary>(messages.map((message) => [message.id, message])),
    [messages]
  );
  const viewMessages = useMemo(() => messages.filter((message) => messageMatchesView(message, activeView)), [messages, activeView]);
  const filteredMessages = useMemo(() => {
    const normalized = searchMode === "local" ? query.trim().toLowerCase() : "";
    const byFilter = viewMessages.filter((message) => {
      if (listAccountFilter !== "all" && message.accountId !== listAccountFilter) return false;
      switch (listFilter) {
        case "unread":
          return message.unread;
        case "flagged":
          return message.flagged;
        case "attachments":
          return message.hasAttachments;
        default:
          return true;
      }
    });
    const searched = normalized ? byFilter.filter((message) =>
      [message.sender, message.senderEmail, message.subject, message.preview]
        .join(" ")
        .toLowerCase()
        .includes(normalized)
    ) : byFilter;
    return sortMessages(searched, listSort);
  }, [viewMessages, query, searchMode, listFilter, listSort, listAccountFilter]);
  const groupedMessages = useMemo(() => groupMessagesByDate(filteredMessages), [filteredMessages]);

  return (
    <main className={`mail-window kizumail-redesign theme-${resolvedTheme}`}>
      <aside className="left-rail">
        <Brand theme={resolvedTheme} />
        <button className="compose-button" onClick={() => openComposer()} disabled={!hasAccounts}>
          <span>Neue E-Mail</span>
          <Edit3 size={18} />
        </button>

        <section className="rail-section">
          <SectionTitle label="Konten" action={<Plus size={16} />} onAction={() => setIsAccountModalOpen(true)} />
          {hasAccounts ? (
            accounts.map((account) => (
              <AccountRow
                key={account.id}
                account={account}
                count={accountSyncLabel(account, syncStatuses, syncingAccountId, isBackgroundSyncing)}
                syncDisabled={syncingAccountId !== null || isBackgroundSyncing || isLoadingMoreMessages}
                onSync={() => void handleSyncAccount(account.id)}
                onRemove={() => void handleRemoveAccount(account.id)}
              />
            ))
          ) : (
            <button className="empty-account-button" onClick={() => setIsAccountModalOpen(true)}>
              <Plus size={15} />
              Konto verbinden
            </button>
          )}
        </section>

        <section className="rail-section">
          <SectionTitle label="Ordner" />
          <NavRow icon={<Inbox size={17} />} label="Posteingang" count={unreadCount ? unreadCount.toString() : undefined} active={activeView === "inbox"} onClick={() => setActiveView("inbox")} />
          <NavRow icon={<Star size={17} />} label="Markiert" active={activeView === "starred"} onClick={() => setActiveView("starred")} />
          <NavRow icon={<Send size={17} />} label="Gesendet" active={activeView === "sent"} onClick={() => setActiveView("sent")} />
          <NavRow icon={<MailOpen size={17} />} label="Entwürfe" active={activeView === "drafts"} onClick={() => setActiveView("drafts")} />
          <NavRow icon={<Archive size={17} />} label="Archiv" active={activeView === "archive"} onClick={() => setActiveView("archive")} />
          <NavRow icon={<ArchiveX size={17} />} label="Spam" active={activeView === "spam"} onClick={() => setActiveView("spam")} />
          <NavRow icon={<Trash2 size={17} />} label="Papierkorb" active={activeView === "trash"} onClick={() => setActiveView("trash")} />
        </section>

        <section className="rail-section">
          <SectionTitle label="Smart Views" />
          <NavRow icon={<Mail size={17} />} label="Ungelesen" active={activeView === "unread"} onClick={() => setActiveView("unread")} />
          <NavRow icon={<Paperclip size={17} />} label="Mit Anhängen" active={activeView === "attachments"} onClick={() => setActiveView("attachments")} />
          <NavRow icon={<CalendarDays size={17} />} label="Heute" active={activeView === "today"} onClick={() => setActiveView("today")} />
          <NavRow icon={<Star size={17} />} label="Wichtig" active={activeView === "important"} onClick={() => setActiveView("important")} />
        </section>

        <section className="rail-section tools-section">
          <SectionTitle label="Tools" />
          <NavRow icon={<ShieldCheck size={17} />} label="Newsletter" badge="Neu" active={activeView === "newsletters"} onClick={() => setActiveView("newsletters")} />
          <NavRow icon={<SlidersHorizontal size={17} />} label="Regeln" active={activeView === "rules"} onClick={() => setActiveView("rules")} />
          <NavRow icon={<Settings size={17} />} label="Einstellungen" active={activeView === "settings"} onClick={() => setActiveView("settings")} />
        </section>

        <div className="online-state">
          <span />
          {hasAccounts ? "Online - lokale App" : "Kein Konto verbunden"}
        </div>
      </aside>

      <section className="message-column">
        <div className="top-search">
          <div className="ai-search-mode" aria-label="Suchmodus">
            <button
              className={searchMode === "local" ? "active" : ""}
              aria-pressed={searchMode === "local"}
              title="Lokale Suche"
              onClick={() => changeSearchMode("local")}
            >
              <Search size={14} />
            </button>
            <button
              className={searchMode === "ai" ? "active" : ""}
              aria-pressed={searchMode === "ai"}
              title="KI-Suche"
              onClick={() => changeSearchMode("ai")}
            >
              <Sparkles size={14} />
            </button>
          </div>
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && searchMode === "ai") void handleAISearch();
            }}
            placeholder={searchMode === "ai" ? "Nach Bedeutung suchen" : "Suchen"}
            aria-label={searchMode === "ai" ? "KI-Suche" : "Lokale Suche"}
          />
          {searchMode === "ai" && (
            <button
              className="ai-search-submit"
              aria-label="KI-Suche starten"
              title="KI-Suche starten"
              disabled={aiSearch.status === "loading" || !aiEnabled}
              onClick={() => void handleAISearch()}
            >
              <Sparkles size={16} />
            </button>
          )}
          <button
            className="theme-orb search-theme-orb"
            aria-label="Client-Theme umschalten"
            title="Client-Theme umschalten"
            onClick={cycleTheme}
          >
            <Sun size={16} />
          </button>
          <button aria-label="Suche leeren" onClick={clearSearch}>
            <X size={16} />
          </button>
        </div>

        <section className="inbox-panel">
          <div className="list-header">
            <div>
              <h1>{viewTitle(activeView)}</h1>
              <span className="list-header-status">{searchMode === "ai" ? aiSearchStatusLabel(aiSearch) : `${unreadCount} ungelesen`}</span>
            </div>
            <div className="list-actions">
              {accounts.length > 1 && (
                <DropdownSelect
                  value={listAccountFilter}
                  ariaLabel="Konto filtern"
                  className="list-filter-control account-filter-control"
                  options={[
                    { value: "all", label: "Alle Konten", description: "Alle verbundenen Konten" },
                    ...accounts.map((account) => ({
                      value: account.id,
                      label: shortEmailLabel(account.email),
                      description: providerLabel(account),
                      icon: <ProviderIcon account={account} />
                    }))
                  ]}
                  onChange={setListAccountFilter}
                />
              )}
              {searchMode === "local" && (
                <>
                  <DropdownSelect
                    value={listFilter}
                    ariaLabel="Mail-Liste filtern"
                    className="list-filter-control"
                    options={[
                      { value: "all", label: "Alle", description: "Alle Nachrichten", icon: <Inbox size={17} /> },
                      { value: "unread", label: "Ungelesen", description: "Nur ungelesene Nachrichten", icon: <MailOpen size={17} /> },
                      { value: "flagged", label: "Wichtig", description: "Markierte Nachrichten", icon: <Flag size={17} /> },
                      { value: "attachments", label: "Anhänge", description: "Nachrichten mit Anhängen", icon: <Paperclip size={17} /> }
                    ]}
                    onChange={(value) => setListFilter(value as ListFilter)}
                  />
                  <DropdownSelect
                    value={listSort}
                    ariaLabel="Mail-Liste sortieren"
                    className="list-filter-control sort-control"
                    options={[
                      { value: "newest", label: "Neueste", description: "Neue Nachrichten zuerst", icon: <CalendarDays size={17} /> },
                      { value: "oldest", label: "Älteste", description: "Alte Nachrichten zuerst", icon: <CalendarDays size={17} /> },
                      { value: "unread", label: "Ungelesen", description: "Ungelesene zuerst", icon: <MailOpen size={17} /> },
                      { value: "sender", label: "Absender", description: "Alphabetisch nach Absender", icon: <User size={17} /> },
                      { value: "subject", label: "Betreff", description: "Alphabetisch nach Betreff", icon: <Mail size={17} /> },
                      { value: "attachments", label: "Anhänge", description: "Nachrichten mit Anhängen zuerst", icon: <Paperclip size={17} /> }
                    ]}
                    onChange={(value) => setListSort(value as MessageSortMode)}
                  />
                  <button aria-label="Listendichte umschalten" title="Listendichte umschalten" onClick={() => setListDensity((value) => value === "comfortable" ? "compact" : "comfortable")}>
                    <SlidersHorizontal size={16} />
                  </button>
                </>
              )}
              <button aria-label="Delta-Sync starten" title="Delta-Sync starten" onClick={() => void handleSyncAllAccounts()} disabled={!hasAccounts || syncingAccountId !== null || isBackgroundSyncing}>
                <LayoutList size={17} />
              </button>
            </div>
          </div>

          <div className={`mail-list scroll-area ${listDensity === "compact" ? "compact" : ""}`}>
            {searchMode === "ai" ? (
              <AISearchResults
                query={query}
                state={aiSearch}
                aiEnabled={aiEnabled}
                messageById={messageById}
                accountById={accountById}
                onSearch={() => void handleAISearch()}
                onSelect={handleSelectAISearchHit}
                onOpenSettings={() => setActiveView("settings")}
              />
            ) : hasAccounts && filteredMessages.length > 0 ? (
              groupedMessages.map((group) => (
                <section className="mail-date-group" key={group.label}>
                  <h2>{group.label}</h2>
                  {group.messages.map((message) => (
                    <button
                      className={[
                        "mail-row",
                        message.id === selectedMessageId ? "selected" : "",
                        message.unread ? "unread" : "read"
                      ].filter(Boolean).join(" ")}
                      key={message.id}
                      onClick={() => void handleSelectMessage(message)}
                      onContextMenu={(event) => openMessageContextMenu(event, message)}
                    >
                      <SenderAvatar label={message.sender} email={message.senderEmail} />
                      <div className="mail-row-content">
                        <div className="row-line">
                          <strong>{message.sender}</strong>
                          <time>{formatMessageListDate(message.date)}</time>
                        </div>
                        <h2>{message.subject}</h2>
                        <p>{message.preview}</p>
                        <div className="mail-row-badges">
                          {message.flagged && <small className="attachment-chip important-chip">Wichtig</small>}
                          {message.hasAttachments && <small className="attachment-chip">Anhang</small>}
                          {accounts.length > 1 && (
                            <small className="attachment-chip account-mail-chip">
                              {shortEmailLabel(accountById.get(message.accountId)?.email ?? "Konto")}
                            </small>
                          )}
                        </div>
                      </div>
                      {message.unread && <span className="unread-dot" />}
                    </button>
                  ))}
                </section>
              ))
            ) : (
              <EmptyList hasAccounts={hasAccounts} onConnect={() => setIsAccountModalOpen(true)} />
            )}
            {searchMode === "local" && hasAccounts && filteredMessages.length > 0 && (
              <div className="mail-load-actions">
                <span>
                  {messages.length} lokal sichtbar · {syncOverview.summary}
                  {nextMessageCursor && ` · nächste Seite ab ${nextMessageCursor}`}
                </span>
                <button onClick={() => void handleLoadMoreMessages()} disabled={isLoadingMoreMessages || syncingAccountId !== null}>
                  Ältere Mails nachladen
                </button>
                <button onClick={() => void handleFullSync()} disabled={isLoadingMoreMessages || syncingAccountId !== null}>
                  Mehr Historie laden
                </button>
                {hasMoreMessages && <small>Weitere lokale Treffer sind vorhanden. Erhöhe das Sichtlimit oder nutze die Sortierung.</small>}
              </div>
            )}
          </div>
        </section>
      </section>

      <section className="reader-shell">
        <div className="reader-grid">
          <section className="message-reader">
            <Toolbar
              hasMessage={Boolean(selectedMessage)}
              onBack={() => setSelectedMessageId(null)}
              onArchive={() => void handleMoveSelected("archive")}
              onTrash={() => void handleMoveSelected("trash")}
              onToggleRead={() => void handleToggleRead()}
              onToggleFlagged={() => void handleToggleFlagged()}
              onReply={() => openReplyComposer()}
              isMoreMenuOpen={isMoreMenuOpen}
              onMore={() => setIsMoreMenuOpen((value) => !value)}
              onSpam={() => void handleMoveSelected("spam")}
              onNotSpam={() => void handleMoveSelected("inbox")}
              isSpamMessage={Boolean(selectedMessage?.folderRole === "spam")}
              onBlockSender={() => void handleToggleSenderBlocked()}
              senderBlocked={Boolean(senderStats?.blocked)}
              onRefresh={() => selectedMessage && void handleSyncAccount(selectedMessage.accountId)}
              mailTheme={mailTheme}
              onToggleMailTheme={() => {
                setMailTheme((value) => value === "dark" ? "light" : "dark");
                setIsMoreMenuOpen(false);
              }}
            />
            <article className={`mail-detail scroll-area ${
              !selectedMessage && (activeView === "rules" || activeView === "settings" || activeView === "newsletters")
                ? "workspace-detail"
                : ""
            }`}>
              {selectedMessage && isMessageDetailLoading && selectedMessageDetail?.id !== selectedMessage.id ? (
                <MessageDetailLoading message={selectedMessage} />
              ) : selectedMessage ? (
                <MessageDetail
                  message={selectedMessage}
                  mailTheme={mailTheme}
                  aiEnabled={aiEnabled}
                  aiUnavailableReason={aiSettingsStatus === "loading"
                    ? "KI-Einstellungen werden geladen."
                    : aiSettingsError || "KI ist deaktiviert."}
                  summaryState={aiSummary}
                  onGenerateSummary={(includeAttachments) => void handleAISummary(includeAttachments)}
                  onNewsletterPreview={() => {
                    setActiveView("newsletters");
                    setToast("Newsletter-Ansicht geöffnet. Vorschau bleibt vor der Abmeldung erforderlich.");
                  }}
                />
              ) : activeView === "rules" || activeView === "settings" || activeView === "newsletters" ? (
                <WorkspacePanel
                  view={activeView}
                  newsletterCount={newsletters.length}
                  newsletters={newsletters}
                  accountCount={accounts.length}
                  settings={settings}
                  rules={rules}
                  onSettingsChange={updateSettings}
                  onRulesChange={saveRules}
                  onNewsletterChanged={() => refreshData()}
                  accounts={accounts}
                  onAccountsChanged={() => void refreshData()}
                  oauthProviders={oauthProviders}
                  oauthClientIds={oauthClientIds}
                  onOAuthClientIdChange={updateOAuthClientId}
                  securityAudit={securityAudit}
                  isSecurityBusy={isSecurityBusy}
                  onRepairCredentials={handleRepairCredentials}
                  onRunSecurityAudit={handleSecurityAudit}
                  aiSettings={aiSettings}
                  aiSettingsStatus={aiSettingsStatus}
                  aiSettingsError={aiSettingsError}
                  isAISettingsSaving={isAISettingsSaving}
                  aiIndexStatus={aiIndexStatus}
                  aiIndexError={aiIndexError}
                  isAIIndexBusy={isAIIndexBusy}
                  onSaveAISettings={handleSaveAISettings}
                  onSetAISecret={handleSetAISecret}
                  onDeleteAISecret={handleDeleteAISecret}
                  onStartAIIndex={() => void handleStartAIIndex()}
                  onCancelAIIndex={() => void handleCancelAIIndex()}
                />
              ) : (
                <SetupPanel hasAccounts={hasAccounts} onConnect={() => setIsAccountModalOpen(true)} />
              )}
            </article>

            <footer className="reply-bar">
              <button disabled={!selectedMessage} onClick={() => openReplyComposer()}>
                <Reply size={16} />
                Antworten
              </button>
              <button disabled={!selectedMessage} onClick={() => openComposer({ accountId: selectedMessage?.accountId })}>
                <Forward size={16} />
                Weiterleiten
              </button>
              <button
                className="quick-reply-trigger"
                disabled={!selectedMessage}
                onClick={() => setQuickReplyOpen((value) => !value)}
              >
                <Sparkles size={15} />
                Schnellantwort
              </button>
              <span>{toast}</span>
            </footer>
            {quickReplyOpen && selectedMessage && (
              <AIQuickReplyDock
                message={selectedMessage}
                aiEnabled={aiEnabled}
                unavailableReason={aiSettingsStatus === "loading"
                  ? "KI-Einstellungen werden geladen."
                  : aiSettingsError || "KI ist deaktiviert."}
                state={quickReplies}
                onGenerate={(includeAttachments) => void handleAIQuickReplies(includeAttachments)}
                onChange={updateQuickReply}
                onUse={(text) => {
                  openReplyComposer(text);
                  setQuickReplyOpen(false);
                }}
                onClose={() => setQuickReplyOpen(false)}
              />
            )}
          </section>

          <aside className="inspector scroll-area">
            <Inspector
              accountCount={accounts.length}
              messageCount={messages.length}
              hasAccounts={hasAccounts}
              newsletters={newsletters}
              selectedMessage={selectedMessage}
              senderStats={senderStats}
              onToggleSenderBlocked={() => void handleToggleSenderBlocked()}
              onConnect={() => setIsAccountModalOpen(true)}
              onOpenRules={() => setActiveView("rules")}
              onOpenSettings={() => setActiveView("settings")}
            />
          </aside>
        </div>
      </section>

      {isComposerOpen && (
        <ComposerModal
          accounts={accounts}
          initialDraft={composerDraft}
          selectedAccountId={selectedMessage?.accountId ?? accounts[0]?.id ?? ""}
          isSending={isSendingEmail}
          onClose={() => setIsComposerOpen(false)}
          onSubmit={(input) => void handleSendEmail(input)}
        />
      )}

      {isAccountModalOpen && (
        <AccountModal
          form={accountForm}
          isSaving={isSavingAccount}
          isTesting={isTestingAccount}
          testResult={connectionTestResult}
          onChange={setAccountForm}
          onClose={() => setIsAccountModalOpen(false)}
          onSubmit={handleCreateAccount}
          onTestConnection={handleTestConnection}
          oauthProviders={oauthProviders}
          oauthClientIds={oauthClientIds}
          oauthConnectingProvider={oauthConnectingProvider}
          onOAuthConnect={handleOAuthConnect}
          onOAuthClientIdChange={updateOAuthClientId}
        />
      )}

      {passwordRecoveryAccountId && (
        <PasswordRecoveryModal
          account={accounts.find((account) => account.id === passwordRecoveryAccountId) ?? null}
          password={recoveryPassword}
          isSaving={syncingAccountId === passwordRecoveryAccountId}
          onChange={setRecoveryPassword}
          onClose={() => {
            setPasswordRecoveryAccountId(null);
            setRecoveryPassword("");
          }}
          onSubmit={handleRecoverPassword}
        />
      )}

      {mailContextMenu && contextMessage && (
        <MailContextMenu
          x={mailContextMenu.x}
          y={mailContextMenu.y}
          message={contextMessage}
          onArchive={() => void moveMessage(contextMessage, "archive")}
          onTrash={() => void moveMessage(contextMessage, "trash")}
          onSpam={() => void moveMessage(contextMessage, "spam")}
          onNotSpam={() => void moveMessage(contextMessage, "inbox")}
          onToggleRead={() => void setMessageUnread(contextMessage, !contextMessage.unread, true)}
          onToggleFlagged={() => void setMessageFlagged(contextMessage, !contextMessage.flagged)}
          onBlockSender={() => void handleToggleSenderBlocked(contextMessage)}
          onReply={() => {
            setMailContextMenu(null);
            openComposer({
              accountId: contextMessage.accountId,
              to: contextMessage.senderEmail,
              subject: contextMessage.subject.toLowerCase().startsWith("re:") ? contextMessage.subject : `Re: ${contextMessage.subject}`
            });
          }}
        />
      )}
    </main>
  );
}

function Brand({ theme }: { theme: "dark" | "light" }) {
  return (
    <div className="brand">
      <img
        className="brand-icon"
        src={theme === "light" ? kizumailLightIcon : kizumailDarkIcon}
        alt=""
      />
      <strong>KizuMail</strong>
    </div>
  );
}

interface DropdownOption {
  value: string;
  label: string;
  description?: string;
  icon?: ReactNode;
  disabled?: boolean;
}

function DropdownSelect({
  value,
  options,
  onChange,
  ariaLabel,
  className = "",
  disabled = false
}: {
  value: string;
  options: DropdownOption[];
  onChange: (value: string) => void;
  ariaLabel: string;
  className?: string;
  disabled?: boolean;
}) {
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const listboxRef = useRef<HTMLDivElement>(null);
  const optionRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const menuId = useId().replace(/:/g, "");
  const titleId = `${menuId}-title`;
  const [isOpen, setIsOpen] = useState(false);
  const [menuStyle, setMenuStyle] = useState<CSSProperties>({
    position: "fixed",
    left: -10_000,
    top: 0,
    visibility: "hidden"
  });
  const selectedIndex = Math.max(0, options.findIndex((option) => option.value === value));
  const selectedOption = options[selectedIndex] ?? options[0];
  const [activeIndex, setActiveIndex] = useState(selectedIndex);

  useEffect(() => {
    if (!isOpen) return;

    const closeOnOutsidePointer = (event: PointerEvent) => {
      const target = event.target as Node;
      if (!rootRef.current?.contains(target) && !menuRef.current?.contains(target)) {
        setIsOpen(false);
      }
    };

    document.addEventListener("pointerdown", closeOnOutsidePointer);
    return () => document.removeEventListener("pointerdown", closeOnOutsidePointer);
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    setActiveIndex(selectedIndex);
  }, [isOpen, selectedIndex]);

  useLayoutEffect(() => {
    if (!isOpen) return;

    function updatePosition() {
      const trigger = triggerRef.current;
      const menu = menuRef.current;
      if (!trigger || !menu) return;
      const margin = 12;
      const gap = 8;
      const triggerRect = trigger.getBoundingClientRect();
      const menuWidth = Math.min(Math.max(menu.offsetWidth, triggerRect.width), window.innerWidth - margin * 2);
      const menuHeight = Math.min(menu.scrollHeight, window.innerHeight - margin * 2);
      const spaceBelow = window.innerHeight - triggerRect.bottom - margin;
      const spaceAbove = triggerRect.top - margin;
      const openAbove = menuHeight > spaceBelow && spaceAbove > spaceBelow;
      const left = Math.max(margin, Math.min(triggerRect.right - menuWidth, window.innerWidth - menuWidth - margin));
      const top = openAbove
        ? Math.max(margin, triggerRect.top - Math.min(menuHeight, spaceAbove) - gap)
        : Math.min(triggerRect.bottom + gap, window.innerHeight - Math.min(menuHeight, spaceBelow) - margin);
      setMenuStyle({
        position: "fixed",
        left,
        top,
        width: menuWidth,
        maxHeight: Math.max(180, openAbove ? spaceAbove - gap : spaceBelow - gap),
        visibility: "visible"
      });
    }

    const frame = window.requestAnimationFrame(() => {
      updatePosition();
      listboxRef.current?.focus({ preventScroll: true });
    });
    window.addEventListener("resize", updatePosition);
    window.addEventListener("scroll", updatePosition, true);
    return () => {
      window.cancelAnimationFrame(frame);
      window.removeEventListener("resize", updatePosition);
      window.removeEventListener("scroll", updatePosition, true);
    };
  }, [isOpen, options.length]);

  useEffect(() => {
    if (isOpen) optionRefs.current[activeIndex]?.scrollIntoView({ block: "nearest" });
  }, [activeIndex, isOpen]);

  function findNextEnabled(start: number, direction: 1 | -1) {
    for (let offset = 1; offset <= options.length; offset += 1) {
      const index = (start + offset * direction + options.length) % options.length;
      if (!options[index]?.disabled) return index;
    }
    return start;
  }

  function selectOption(option: DropdownOption) {
    if (option.disabled) return;
    onChange(option.value);
    setIsOpen(false);
    triggerRef.current?.focus();
  }

  function handleTriggerKeyDown(event: React.KeyboardEvent<HTMLButtonElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      setIsOpen(false);
      return;
    }

    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      setIsOpen((current) => !current);
      return;
    }

    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const direction = event.key === "ArrowDown" ? 1 : -1;
      if (!isOpen) {
        setIsOpen(true);
        setActiveIndex(selectedIndex);
      } else {
        setActiveIndex(findNextEnabled(activeIndex, direction));
      }
    }
  }

  function handleMenuKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      setIsOpen(false);
      triggerRef.current?.focus();
    } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      setActiveIndex(findNextEnabled(activeIndex, event.key === "ArrowDown" ? 1 : -1));
    } else if (event.key === "Home" || event.key === "End") {
      event.preventDefault();
      const start = event.key === "Home" ? -1 : 0;
      setActiveIndex(findNextEnabled(start, event.key === "Home" ? 1 : -1));
    } else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      const option = options[activeIndex];
      if (option) selectOption(option);
    } else if (event.key.length === 1 && /\S/.test(event.key)) {
      const query = event.key.toLocaleLowerCase("de");
      const nextIndex = options.findIndex((option, index) =>
        index !== activeIndex && !option.disabled && option.label.toLocaleLowerCase("de").startsWith(query)
      );
      if (nextIndex >= 0) setActiveIndex(nextIndex);
    }
  }

  return (
    <div
      ref={rootRef}
      className={["km-dropdown", className, isOpen ? "is-open" : ""].filter(Boolean).join(" ")}
      onClick={(event) => event.stopPropagation()}
    >
      <button
        ref={triggerRef}
        type="button"
        className="km-dropdown-trigger"
        aria-label={ariaLabel}
        aria-haspopup="listbox"
        aria-expanded={isOpen}
        aria-controls={isOpen ? menuId : undefined}
        disabled={disabled || !selectedOption}
        onClick={() => setIsOpen((current) => !current)}
        onKeyDown={handleTriggerKeyDown}
      >
        {selectedOption?.icon && <span className="km-dropdown-trigger-icon">{selectedOption.icon}</span>}
        <span className="km-dropdown-trigger-label">{selectedOption?.label ?? "Auswählen"}</span>
        <ChevronDown size={14} aria-hidden="true" />
      </button>
      {isOpen && createPortal(
        <div
          ref={menuRef}
          className={["km-dropdown-menu", className.includes("account-filter-control") ? "account-dropdown-menu" : ""].filter(Boolean).join(" ")}
          style={menuStyle}
          onPointerDown={(event) => event.stopPropagation()}
        >
          <div className="km-dropdown-menu-title" id={titleId}>{ariaLabel}</div>
          <div
            ref={listboxRef}
            className="km-dropdown-listbox"
            id={menuId}
            role="listbox"
            tabIndex={0}
            aria-labelledby={titleId}
            aria-activedescendant={`${menuId}-option-${activeIndex}`}
            onKeyDown={handleMenuKeyDown}
          >
            {options.map((option, index) => (
              <button
                ref={(node) => { optionRefs.current[index] = node; }}
                type="button"
                role="option"
                id={`${menuId}-option-${index}`}
                tabIndex={-1}
                aria-selected={option.value === value}
                className={["km-dropdown-option", index === activeIndex ? "is-active" : "", option.value === value ? "is-selected" : ""].filter(Boolean).join(" ")}
                key={option.value}
                disabled={option.disabled}
                onMouseEnter={() => setActiveIndex(index)}
                onClick={() => selectOption(option)}
              >
                {option.icon && <span className="km-dropdown-option-icon">{option.icon}</span>}
                <span className="km-dropdown-option-copy">
                  <strong>{option.label}</strong>
                  {option.description && <small>{option.description}</small>}
                </span>
                {option.value === value && <Check size={15} aria-hidden="true" />}
              </button>
            ))}
          </div>
        </div>,
        document.body
      )}
    </div>
  );
}

function SectionTitle({ label, action, onAction }: { label: string; action?: ReactNode; onAction?: () => void }) {
  return (
    <div className="section-title">
      <span>{label}</span>
      {action && <button onClick={onAction}>{action}</button>}
    </div>
  );
}

function AccountRow({
  account,
  count,
  syncDisabled,
  onSync,
  onRemove
}: {
  account: Account;
  count: string;
  syncDisabled: boolean;
  onSync: () => void;
  onRemove: () => void;
}) {
  return (
    <div className="account-row">
      <button className="account-sync-button" onClick={onSync} disabled={syncDisabled}>
        <ProviderIcon account={account} />
        <span className="account-label">
          <strong>{shortEmailLabel(account.email)}</strong>
          <small>{providerLabel(account)}</small>
        </span>
        <em>{count}</em>
      </button>
      <button className="account-remove-button" aria-label={`Konto ${account.email} entfernen`} onClick={onRemove}>
        <X size={13} />
      </button>
    </div>
  );
}

function ProviderIcon({ account }: { account: Account }) {
  const emailDomain = senderDomain(account.email);
  const domain =
    account.provider === "gmail" || emailDomain.includes("gmail") ? "gmail.com" :
    account.provider === "microsoft" || emailDomain.includes("hotmail") || emailDomain.includes("outlook") || emailDomain.includes("live") ? "outlook.com" :
    emailDomain.includes("icloud") || emailDomain.includes("me.com") || emailDomain.includes("mac.com") ? "icloud.com" :
    emailDomain;
  return <SenderAvatar label={providerLabel(account)} email={`icon@${domain}`} variant="small" />;
}

function NavRow({
  icon,
  label,
  count,
  active,
  badge,
  onClick
}: {
  icon: ReactNode;
  label: string;
  count?: string;
  active?: boolean;
  badge?: string;
  onClick?: () => void;
}) {
  return (
    <button className={active ? "nav-row active" : "nav-row"} onClick={onClick}>
      {icon}
      <span>{label}</span>
      {count && <em>{count}</em>}
      {badge && <b>{badge}</b>}
    </button>
  );
}

function SenderAvatar({ label, email, variant = "default" }: { label: string; email?: string; variant?: "default" | "hero" | "small" }) {
  const [sourceIndex, setSourceIndex] = useState(0);
  const domain = senderDomain(email);
  const initial = label.trim().slice(0, 1).toUpperCase() || "?";
  const sources = senderIconSources(domain);
  const src = sources[sourceIndex] ?? "";

  useEffect(() => {
    setSourceIndex(0);
  }, [domain]);

  return (
    <div className={[
      "avatar",
      "sender-avatar",
      src ? "has-image" : "fallback",
      variant === "hero" ? "hero-avatar" : "",
      variant === "small" ? "small-avatar" : ""
    ].filter(Boolean).join(" ")}>
      {src ? (
        <img
          src={src}
          alt=""
          loading="lazy"
          decoding="async"
          referrerPolicy="no-referrer"
          onError={() => setSourceIndex((current) => current + 1)}
        />
      ) : (
        <span>{initial}</span>
      )}
    </div>
  );
}

function Toolbar({
  hasMessage,
  onBack,
  onArchive,
  onTrash,
  onToggleRead,
  onToggleFlagged,
  onReply,
  onMore,
  isMoreMenuOpen,
  onSpam,
  onNotSpam,
  isSpamMessage,
  onBlockSender,
  senderBlocked,
  onRefresh,
  mailTheme,
  onToggleMailTheme
}: {
  hasMessage: boolean;
  onBack: () => void;
  onArchive: () => void;
  onTrash: () => void;
  onToggleRead: () => void;
  onToggleFlagged: () => void;
  onReply: () => void;
  onMore: () => void;
  isMoreMenuOpen: boolean;
  onSpam: () => void;
  onNotSpam: () => void;
  isSpamMessage: boolean;
  onBlockSender: () => void;
  senderBlocked: boolean;
  onRefresh: () => void;
  mailTheme: "dark" | "light";
  onToggleMailTheme: () => void;
}) {
  const moreMenuId = useId().replace(/:/g, "");
  const runMenuAction = (action: () => void) => {
    onMore();
    action();
  };
  return (
    <div className="reader-toolbar">
      <button aria-label="Zurück" onClick={onBack}>←</button>
      <button aria-label="Archivieren" disabled={!hasMessage} onClick={onArchive}><Archive size={17} /></button>
      <button aria-label="Löschen" disabled={!hasMessage} onClick={onTrash}><Trash2 size={17} /></button>
      <button aria-label="Antworten" disabled={!hasMessage} onClick={onReply}><Reply size={17} /></button>
      <button aria-label="Weiterleiten" disabled={!hasMessage} onClick={onReply}><Forward size={17} /></button>
      <button aria-label="Wichtig markieren" disabled={!hasMessage} onClick={onToggleFlagged}><Flag size={17} /></button>
      <div className="toolbar-more">
        <button
          aria-label="Mehr"
          aria-haspopup="menu"
          aria-expanded={isMoreMenuOpen}
          aria-controls={isMoreMenuOpen ? moreMenuId : undefined}
          disabled={!hasMessage}
          onClick={(event) => {
            event.stopPropagation();
            onMore();
          }}
        >
          <MoreHorizontal size={18} />
        </button>
        {isMoreMenuOpen && hasMessage && (
          <div className="toolbar-menu" id={moreMenuId} role="menu" onClick={(event) => event.stopPropagation()}>
            {isSpamMessage ? (
              <button role="menuitem" onClick={() => runMenuAction(onNotSpam)}>Kein Spam · dauerhaft in Posteingang</button>
            ) : (
              <button role="menuitem" onClick={() => runMenuAction(onSpam)}>Als Spam verschieben</button>
            )}
            <button role="menuitem" onClick={() => runMenuAction(onBlockSender)}>{senderBlocked ? "Absender entsperren" : "Absender blockieren"}</button>
            <button role="menuitem" onClick={() => runMenuAction(onToggleRead)}>Gelesen/Ungelesen</button>
            <button role="menuitem" onClick={() => runMenuAction(onToggleFlagged)}>Wichtig markieren/entfernen</button>
            <button role="menuitem" onClick={() => runMenuAction(onRefresh)}>Konto neu synchronisieren</button>
            <button role="menuitem" onClick={() => runMenuAction(onToggleMailTheme)}>Mail-Theme: {mailTheme === "dark" ? "Dunkel" : "Hell"}</button>
          </div>
        )}
      </div>
    </div>
  );
}

function AISearchResults({
  query,
  state,
  aiEnabled,
  messageById,
  accountById,
  onSearch,
  onSelect,
  onOpenSettings
}: {
  query: string;
  state: AIRequestState<AISearchResult>;
  aiEnabled: boolean;
  messageById: Map<string, MessageSummary>;
  accountById: Map<string, Account>;
  onSearch: () => void;
  onSelect: (hit: AISearchHit) => void;
  onOpenSettings: () => void;
}) {
  if (!aiEnabled) {
    return (
      <AIStateCard title="KI-Suche ist deaktiviert" description="Aktiviere einen Anbieter und Modelle in den Einstellungen.">
        <button onClick={onOpenSettings}>Einstellungen öffnen</button>
      </AIStateCard>
    );
  }
  if (!query.trim()) {
    return <AIStateCard title="KI-Suche" description="Beschreibe Inhalt, Kontext oder Bedeutung der gesuchten Nachricht." />;
  }
  if (state.status === "loading") {
    return <AIStateCard busy title="KI-Suche läuft" description="Der lokale Index wird nach belegbaren Treffern durchsucht." />;
  }
  if (state.status === "error") {
    return (
      <AIStateCard title="KI-Suche fehlgeschlagen" description={state.error} tone="error">
        <button onClick={onSearch}>Erneut versuchen</button>
      </AIStateCard>
    );
  }
  if (state.status !== "success") {
    return (
      <AIStateCard title="Bereit für KI-Suche" description="Starte die Suche mit Enter oder dem Sparkles-Button.">
        <button onClick={onSearch}>Suchen</button>
      </AIStateCard>
    );
  }
  const hits = state.data?.hits ?? [];
  if (!hits.length) {
    return <AIStateCard title="Keine belegten Treffer" description="Der aktuelle Index enthält keine passenden Nachrichten." />;
  }

  return (
    <div className="ai-search-results" aria-live="polite">
      {state.data?.summary && <p className="ai-search-summary">{state.data.summary}</p>}
      {hits.map((hit, index) => {
        const message = messageById.get(hit.messageId);
        const account = accountById.get(hit.accountId);
        return (
          <article className="ai-search-hit" key={`${hit.messageId}-${index}`}>
            <button className="ai-search-hit-main" onClick={() => onSelect(hit)}>
              <span className="ai-search-hit-icon"><Sparkles size={15} /></span>
              <span>
                <strong>{hit.subject || message?.subject || "Ohne Betreff"}</strong>
                <small>
                  {hit.sender || message?.sender || hit.senderEmail || message?.senderEmail || "Unbekannter Absender"}
                  {account ? ` · ${shortEmailLabel(account.email)}` : ""}
                </small>
                <p>{hit.snippet}</p>
              </span>
              {typeof hit.score === "number" && <em>{formatAIScore(hit.score)}</em>}
            </button>
            <AIEvidenceList evidence={hit.evidence} />
          </article>
        );
      })}
    </div>
  );
}

function AISummaryPanel({
  state,
  aiEnabled,
  unavailableReason,
  hasAttachments,
  includeAttachments,
  onIncludeAttachments,
  onGenerate,
  onClose
}: {
  state: AIRequestState<AISummaryResult>;
  aiEnabled: boolean;
  unavailableReason: string;
  hasAttachments: boolean;
  includeAttachments: boolean;
  onIncludeAttachments: (value: boolean) => void;
  onGenerate: () => void;
  onClose: () => void;
}) {
  return (
    <section className="ai-summary-panel" aria-live="polite">
      <header>
        <div>
          <Sparkles size={16} />
          <strong>KI-Zusammenfassung</strong>
        </div>
        <button aria-label="Zusammenfassung schließen" onClick={onClose}><X size={15} /></button>
      </header>
      {!aiEnabled ? (
        <p className="ai-inline-error">{unavailableReason}</p>
      ) : state.status === "loading" ? (
        <div className="ai-loading-row"><span className="ai-spinner" /> Nachricht wird zusammengefasst...</div>
      ) : state.status === "error" ? (
        <p className="ai-inline-error">{state.error}</p>
      ) : state.data ? (
        <div className="ai-generated-copy">
          <p>{state.data.summary}</p>
          <AIEvidenceList evidence={state.data.evidence} />
        </div>
      ) : (
        <p>Erstellt eine Zusammenfassung dieser Nachricht. Inhalte werden nur für diese Aktion verarbeitet.</p>
      )}
      <footer>
        <label className="ai-attachment-consent">
          <input
            type="checkbox"
            checked={includeAttachments}
            disabled={!hasAttachments || state.status === "loading"}
            onChange={(event) => onIncludeAttachments(event.target.checked)}
          />
          Anhänge einmalig einbeziehen
        </label>
        <button disabled={!aiEnabled || state.status === "loading"} onClick={onGenerate}>
          <Sparkles size={14} />
          {state.data ? "Neu erstellen" : "Zusammenfassen"}
        </button>
      </footer>
    </section>
  );
}

function AIQuickReplyDock({
  message,
  aiEnabled,
  unavailableReason,
  state,
  onGenerate,
  onChange,
  onUse,
  onClose
}: {
  message: MessageSummary;
  aiEnabled: boolean;
  unavailableReason: string;
  state: AIRequestState<AIQuickReply[]>;
  onGenerate: (includeAttachments: boolean) => void;
  onChange: (index: number, text: string) => void;
  onUse: (text: string) => void;
  onClose: () => void;
}) {
  const [includeAttachments, setIncludeAttachments] = useState(false);
  const replies = state.data ?? [];
  return (
    <section className="quick-reply-dock ai-quick-reply-dock" aria-live="polite">
      <header>
        <div>
          <Sparkles size={16} />
          <span><strong>Schnellantworten</strong><small>An {message.sender}</small></span>
        </div>
        <button aria-label="Schnellantwort schließen" onClick={onClose}><X size={15} /></button>
      </header>
      {!aiEnabled ? (
        <p className="ai-inline-error">{unavailableReason}</p>
      ) : state.status === "loading" ? (
        <div className="ai-loading-row"><span className="ai-spinner" /> Drei Varianten werden erstellt...</div>
      ) : state.status === "error" ? (
        <p className="ai-inline-error">{state.error}</p>
      ) : replies.length ? (
        <div className="ai-reply-options">
          {replies.map((reply, index) => (
            <article key={reply.id || `${reply.tone || reply.label || "reply"}-${index}`}>
              <label htmlFor={`ai-reply-${index}`}>{reply.label || reply.tone || `Variante ${index + 1}`}</label>
              <textarea
                id={`ai-reply-${index}`}
                value={reply.text}
                rows={3}
                onChange={(event) => onChange(index, event.target.value)}
              />
              <button disabled={!reply.text.trim()} onClick={() => onUse(reply.text)}>
                In Composer übernehmen
              </button>
            </article>
          ))}
        </div>
      ) : (
        <p>Erstellt drei editierbare Varianten. Erst „In Composer übernehmen“ öffnet einen Entwurf.</p>
      )}
      <footer>
        <label className="ai-attachment-consent">
          <input
            type="checkbox"
            checked={includeAttachments}
            disabled={!message.hasAttachments || state.status === "loading"}
            onChange={(event) => setIncludeAttachments(event.target.checked)}
          />
          Anhänge einmalig einbeziehen
        </label>
        <button disabled={!aiEnabled || state.status === "loading"} onClick={() => onGenerate(includeAttachments)}>
          <Sparkles size={14} />
          {replies.length ? "Neu erstellen" : "Varianten erstellen"}
        </button>
      </footer>
    </section>
  );
}

function AIStateCard({
  title,
  description,
  busy = false,
  tone = "neutral",
  children
}: {
  title: string;
  description: string;
  busy?: boolean;
  tone?: "neutral" | "error";
  children?: ReactNode;
}) {
  return (
    <div className={`ai-state-card ${tone === "error" ? "error" : ""}`}>
      <span className="ai-state-icon">{busy ? <span className="ai-spinner" /> : <Sparkles size={19} />}</span>
      <strong>{title}</strong>
      <p>{description}</p>
      {children}
    </div>
  );
}

function AIEvidenceList({ evidence }: { evidence?: AIEvidence[] }) {
  if (!evidence?.length) return null;
  return (
    <ul className="ai-evidence-list" aria-label="Belege">
      {evidence.map((item, index) => {
        const excerpt = typeof item === "string" ? item : item.excerpt;
        const label = typeof item === "string" ? undefined : item.label;
        return (
          <li key={`${label || "evidence"}-${index}`}>
            <span>{label || `Beleg ${index + 1}`}</span>
            <q>{excerpt}</q>
          </li>
        );
      })}
    </ul>
  );
}

function EmptyList({ hasAccounts, onConnect }: { hasAccounts: boolean; onConnect: () => void }) {
  return (
    <div className="empty-mail-list">
      <Mail size={30} />
      <h2>{hasAccounts ? "Noch keine E-Mails synchronisiert" : "Noch kein Konto verbunden"}</h2>
      <p>
        {hasAccounts
          ? "Der nächste Schritt verbindet IMAP und lädt Ordner sowie Nachrichten lokal."
          : "Verbinde zuerst ein bestehendes IMAP-Konto. Danach synchronisiert KizuMail lokal."}
      </p>
      {!hasAccounts && <button onClick={onConnect}>Konto verbinden</button>}
    </div>
  );
}

function SetupPanel({ hasAccounts, onConnect }: { hasAccounts: boolean; onConnect: () => void }) {
  return (
    <section className="setup-panel">
      <div className="setup-icon">
        <Mail size={42} />
      </div>
      <h2>{hasAccounts ? "Bereit für den ersten Sync" : "Willkommen in KizuMail"}</h2>
      <p>
        {hasAccounts
          ? "Das Konto ist lokal angelegt. Starte den Sync, damit KizuMail Ordner und Inbox-Nachrichten lokal lädt."
          : "Keine Beispielmails mehr: KizuMail startet leer und zeigt Inhalte erst nach Konto-Setup und lokalem Sync."}
      </p>
      <button onClick={onConnect}>{hasAccounts ? "Weiteres Konto hinzufügen" : "Erstes Konto verbinden"}</button>
    </section>
  );
}

function MessageDetailLoading({ message }: { message: MessageSummary }) {
  return (
    <section className="message-detail-loading" aria-live="polite" aria-busy="true">
      <SenderAvatar label={message.sender} email={message.senderEmail} />
      <div>
        <span className="detail-loading-kicker">Nachricht wird geöffnet</span>
        <h2>{message.subject}</h2>
        <p>{message.sender}</p>
      </div>
      <span className="detail-loading-bar" />
      <span className="detail-loading-bar short" />
    </section>
  );
}

function MessageDetail({
  message,
  mailTheme,
  aiEnabled,
  aiUnavailableReason,
  summaryState,
  onGenerateSummary,
  onNewsletterPreview
}: {
  message: MessageSummary;
  mailTheme: "dark" | "light";
  aiEnabled: boolean;
  aiUnavailableReason: string;
  summaryState: AIRequestState<AISummaryResult>;
  onGenerateSummary: (includeAttachments: boolean) => void;
  onNewsletterPreview: () => void;
}) {
  const [readerMode, setReaderMode] = useState<ReaderMode>("readable");
  const [allowRemoteImages, setAllowRemoteImages] = useState(false);
  const [isSummaryOpen, setIsSummaryOpen] = useState(false);
  const [includeAttachments, setIncludeAttachments] = useState(false);
  const content = useMemo(() => buildReadableEmailContent(message), [message]);
  const domain = senderDomain(message.senderEmail);
  const originalDocument = useMemo(
    () => buildOriginalEmailDocument(message, allowRemoteImages),
    [message, allowRemoteImages]
  );
  const remoteMedia = useMemo(() => extractRemoteEmailMedia(message), [message]);
  const { inlineImages, regularAttachments } = useMemo(() => ({
    inlineImages: message.attachments.filter((attachment) =>
      attachment.inline && attachment.mimeType.startsWith("image/") && attachment.storageRef && !isBodyLikeAttachment(attachment)
    ),
    regularAttachments: message.attachments.filter((attachment) =>
      attachment.disposition !== "body" &&
      (!attachment.inline || !attachment.mimeType.startsWith("image/")) &&
      !isBodyLikeAttachment(attachment)
    )
  }), [message.attachments]);
  const hasOriginalHtml = Boolean(message.bodyHtml?.trim());
  const heroImage = inlineImages[0];
  const senderAccentColor = senderAccent(domain);

  useEffect(() => {
    setReaderMode("readable");
    setAllowRemoteImages(false);
    setIsSummaryOpen(false);
    setIncludeAttachments(false);
  }, [message.id]);

  return (
    <>
      <div className="subject-row reader-title-row">
        <div>
          <h2>{message.subject}</h2>
          <div className="subject-meta">
            {message.hasNewsletterSignals && <span>Newsletter</span>}
            {regularAttachments.length > 0 && <span>{regularAttachments.length} Anhänge</span>}
            {hasOriginalHtml && <span>HTML</span>}
          </div>
        </div>
        <div className="reader-title-actions">
          <div className="reader-mode-toggle" aria-label="Mail-Darstellung">
            <button className={readerMode === "readable" ? "active" : ""} onClick={() => setReaderMode("readable")}>
              Lesbar
            </button>
            <button className={readerMode === "original" ? "active" : ""} onClick={() => setReaderMode("original")}>
              Original
            </button>
          </div>
        </div>
      </div>
      <div className="sender-row">
        <SenderAvatar label={message.sender} email={message.senderEmail} />
        <div>
          <strong>
            {message.sender} <span>&lt;{message.senderEmail}&gt;</span>
          </strong>
          <p>{formatFullDateTime(message.date)} <ChevronDown size={13} /></p>
        </div>
        <div className="sender-row-actions">
          <button
            className={`ai-icon-button ${isSummaryOpen ? "active" : ""}`}
            aria-label="KI-Zusammenfassung"
            title={aiEnabled ? "KI-Zusammenfassung" : aiUnavailableReason}
            onClick={() => setIsSummaryOpen((value) => !value)}
          >
            <Sparkles size={16} />
          </button>
          <time>{formatShortTime(message.date)}</time>
        </div>
      </div>
      {isSummaryOpen && (
        <AISummaryPanel
          state={summaryState}
          aiEnabled={aiEnabled}
          unavailableReason={aiUnavailableReason}
          hasAttachments={message.attachments.length > 0}
          includeAttachments={includeAttachments}
          onIncludeAttachments={setIncludeAttachments}
          onGenerate={() => onGenerateSummary(includeAttachments)}
          onClose={() => setIsSummaryOpen(false)}
        />
      )}
      {message.hasNewsletterSignals && (
        <div className="newsletter-alert">
          <p>KizuMail hat Newsletter-Signale erkannt. Abmelden bleibt preview-first.</p>
          <button type="button" onClick={onNewsletterPreview}>Abmelde-Vorschau</button>
        </div>
      )}
      <section
        className={`message-body-card reader-message-card mail-theme-${mailTheme}`}
        style={{ "--sender-accent": senderAccentColor } as CSSProperties}
      >
        {readerMode === "readable" ? (
          <>
            <div className={`reader-message-hero ${heroImage ? "has-media" : ""}`}>
              <div className="reader-hero-copy">
                <div className="reader-brand-lockup">
                  <SenderAvatar label={message.sender} email={message.senderEmail} variant="hero" />
                  <div>
                    <strong>{message.sender}</strong>
                    <span>{domain || "E-Mail"}</span>
                  </div>
                </div>
                <h3>{message.subject}</h3>
                <p>{content.summary}</p>
              </div>
              {heroImage && (
                <figure className="reader-hero-media">
                  <img src={safeAssetSrc(heroImage.storageRef)} alt={heroImage.filename} loading="lazy" />
                </figure>
              )}
            </div>
            {inlineImages.length > 1 && (
              <div className="inline-image-strip">
                {inlineImages.slice(1, 5).map((attachment, index) => (
                  <figure key={`${attachment.storageRef}-${index}`}>
                    <img src={safeAssetSrc(attachment.storageRef)} alt={attachment.filename} loading="lazy" />
                    <figcaption>{attachment.filename}</figcaption>
                  </figure>
                ))}
              </div>
            )}
            {remoteMedia.length > 0 && (
              <RemoteMediaGallery
                media={remoteMedia}
                enabled={allowRemoteImages}
                onToggle={() => setAllowRemoteImages((value) => !value)}
              />
            )}
            <EmailContent paragraphs={content.paragraphs} links={content.links} />
            <div className="mail-insight-grid" aria-label="KizuMail Mail-Zusammenfassung">
              <article>
                <span>Empfangen</span>
                <strong>{formatMessageListDate(message.date)}</strong>
                <p>{formatShortTime(message.date)}</p>
              </article>
              <article>
                <span>Absender</span>
                <strong>{domain || "E-Mail"}</strong>
                <p>{message.hasNewsletterSignals ? "Newsletter erkannt" : "Direkte Nachricht"}</p>
              </article>
              <article>
                <span>Inhalt</span>
                <strong>{content.links.length}</strong>
                <p>{content.links.length === 1 ? "Link erkannt" : "Links erkannt"}</p>
              </article>
            </div>
            {regularAttachments.length > 0 && <AttachmentGallery attachments={regularAttachments} />}
          </>
        ) : (
          <div className="original-mail-view">
            <div className="original-mail-toolbar">
              <div>
                <strong>Originalansicht</strong>
                <span>{allowRemoteImages ? "Externe Inhalte erlaubt" : "Externe Inhalte zum Schutz deiner Privatsphäre blockiert"}</span>
              </div>
              <button onClick={() => setAllowRemoteImages((value) => !value)}>
                {allowRemoteImages ? "Externe Inhalte blockieren" : "Externe Inhalte laden"}
              </button>
            </div>
            <iframe
              className="message-html-frame"
              title={`Originalansicht: ${message.subject}`}
              sandbox="allow-popups allow-popups-to-escape-sandbox"
              srcDoc={originalDocument}
            />
            {regularAttachments.length > 0 && <AttachmentGallery attachments={regularAttachments} />}
          </div>
        )}
      </section>
    </>
  );
}

function AttachmentGallery({ attachments }: { attachments: MessageAttachment[] }) {
  return (
    <section className="attachment-gallery">
      <div className="attachment-gallery-heading">
        <Paperclip size={16} />
        <strong>Anhänge</strong>
      </div>
      <div className="attachment-gallery-grid">
        {attachments.map((attachment, index) => (
          <article className="attachment-preview-card" key={`${attachment.filename}-${index}`}>
            {renderAttachmentPreview(attachment)}
            <div>
              <strong>{attachment.filename}</strong>
              <span>{attachment.mimeType} · {formatFileSize(attachment.size)}</span>
            </div>
            {attachment.storageRef && <a href={safeAssetSrc(attachment.storageRef)} target="_blank" rel="noreferrer">Öffnen</a>}
          </article>
        ))}
      </div>
    </section>
  );
}

function RemoteMediaGallery({
  media,
  enabled,
  onToggle
}: {
  media: ReturnType<typeof extractRemoteEmailMedia>;
  enabled: boolean;
  onToggle: () => void;
}) {
  return (
    <section className="remote-media-gallery">
      <div className="remote-media-notice">
        <div>
          <strong>Externe Inhalte</strong>
          <span>
            {enabled
              ? `${media.length} externe Inhalte werden angezeigt.`
              : `${media.length} externe Inhalte sind zum Schutz vor Tracking blockiert.`}
          </span>
        </div>
        <button type="button" onClick={onToggle}>
          {enabled ? "Wieder blockieren" : "Sicher laden"}
        </button>
      </div>
      {enabled && (
        <div className="remote-media-grid">
          {media.map((item) => (
            <article className="remote-media-card" key={`${item.kind}-${item.url}`}>
              {item.kind === "image" && (
                <img src={item.url} alt={item.label} loading="lazy" referrerPolicy="no-referrer" />
              )}
              {item.kind === "video" && (
                <video src={item.url} controls preload="metadata" />
              )}
              {item.kind === "pdf" && (
                <iframe src={item.url} title={item.label} referrerPolicy="no-referrer" />
              )}
              <div>
                <strong>{item.label}</strong>
                <a href={item.url} target="_blank" rel="noreferrer">Extern öffnen</a>
              </div>
            </article>
          ))}
        </div>
      )}
    </section>
  );
}

function renderAttachmentPreview(attachment: MessageAttachment) {
  if (!attachment.storageRef) {
    return <div className="attachment-preview-placeholder"><Paperclip size={24} /></div>;
  }
  const src = safeAssetSrc(attachment.storageRef);
  const kind = attachmentPreviewKind(attachment);
  if (kind === "image") {
    return <img className="attachment-preview-media" src={src} alt={attachment.filename} loading="lazy" />;
  }
  if (kind === "video") {
    return <video className="attachment-preview-media" src={src} controls preload="metadata" />;
  }
  if (kind === "pdf") {
    return <iframe className="attachment-preview-frame" src={src} title={attachment.filename} />;
  }
  return <div className="attachment-preview-placeholder"><Paperclip size={24} /></div>;
}

function EmailContent({ paragraphs, links }: { paragraphs: string[]; links: string[] }) {
  if (paragraphs.length === 0) {
    return <p className="empty-message-body">Diese Nachricht hat noch keinen lokal extrahierten lesbaren Text.</p>;
  }

  return (
    <div className="curated-mail-body">
      {paragraphs.map((paragraph, index) => (
        <p key={`${paragraph.slice(0, 18)}-${index}`}>{renderLinkedText(paragraph)}</p>
      ))}
      {links.length > 0 && (
        <div className="curated-link-list">
          <strong>Links in dieser E-Mail</strong>
          {links.slice(0, 6).map((link) => (
            <a href={link} target="_blank" rel="noreferrer" key={link}>{shortenLink(link)}</a>
          ))}
        </div>
      )}
    </div>
  );
}

function MailContextMenu({
  x,
  y,
  message,
  onArchive,
  onTrash,
  onSpam,
  onNotSpam,
  onToggleRead,
  onToggleFlagged,
  onBlockSender,
  onReply
}: {
  x: number;
  y: number;
  message: MessageSummary;
  onArchive: () => void;
  onTrash: () => void;
  onSpam: () => void;
  onNotSpam: () => void;
  onToggleRead: () => void;
  onToggleFlagged: () => void;
  onBlockSender: () => void;
  onReply: () => void;
}) {
  const left = Math.max(8, Math.min(x, window.innerWidth - 258));
  const top = Math.max(8, Math.min(y, window.innerHeight - 346));
  return (
    <div className="mail-context-menu" role="menu" style={{ left, top }} onClick={(event) => event.stopPropagation()}>
      <button role="menuitem" onClick={onReply}><Reply size={15} /> Antworten</button>
      <button role="menuitem" onClick={onArchive}><Archive size={15} /> Archivieren</button>
      <button role="menuitem" onClick={onTrash}><Trash2 size={15} /> Löschen</button>
      {message.folderRole === "spam" ? (
        <button role="menuitem" onClick={onNotSpam}><Inbox size={15} /> Kein Spam · dauerhaft in Posteingang</button>
      ) : (
        <button role="menuitem" onClick={onSpam}><ArchiveX size={15} /> In Spam verschieben</button>
      )}
      <button role="menuitem" onClick={onBlockSender}><Ban size={15} /> Absender blockieren</button>
      <button role="menuitem" onClick={onToggleFlagged}><Flag size={15} /> {message.flagged ? "Wichtig entfernen" : "Als wichtig markieren"}</button>
      <button role="menuitem" onClick={onToggleRead}><MailOpen size={15} /> {message.unread ? "Als gelesen markieren" : "Als ungelesen markieren"}</button>
    </div>
  );
}

function WorkspacePanel({
  view,
  newsletterCount,
  newsletters,
  accountCount,
  settings,
  rules,
  onSettingsChange,
  onRulesChange,
  onNewsletterChanged,
  accounts,
  onAccountsChanged,
  oauthProviders,
  oauthClientIds,
  onOAuthClientIdChange,
  securityAudit,
  isSecurityBusy,
  onRepairCredentials,
  onRunSecurityAudit,
  aiSettings,
  aiSettingsStatus,
  aiSettingsError,
  isAISettingsSaving,
  aiIndexStatus,
  aiIndexError,
  isAIIndexBusy,
  onSaveAISettings,
  onSetAISecret,
  onDeleteAISecret,
  onStartAIIndex,
  onCancelAIIndex
}: {
  view: "newsletters" | "rules" | "settings";
  newsletterCount: number;
  newsletters: NewsletterSubscription[];
  accountCount: number;
  settings: LocalSettings;
  rules: LocalRule[];
  onSettingsChange: (settings: LocalSettings) => void;
  onRulesChange: (rules: LocalRule[]) => void;
  onNewsletterChanged: () => void | Promise<unknown>;
  accounts: Account[];
  onAccountsChanged: () => void;
  oauthProviders: OAuthProviderStatus[];
  oauthClientIds: Record<string, string>;
  onOAuthClientIdChange: (provider: string, clientId: string) => void;
  securityAudit: SecurityAuditResult | null;
  isSecurityBusy: boolean;
  onRepairCredentials: () => void;
  onRunSecurityAudit: () => void;
  aiSettings: AISettings | null;
  aiSettingsStatus: RequestStatus;
  aiSettingsError: string;
  isAISettingsSaving: boolean;
  aiIndexStatus: AIIndexStatus | null;
  aiIndexError: string;
  isAIIndexBusy: boolean;
  onSaveAISettings: (input: AISettingsUpdateInput) => Promise<AISettings>;
  onSetAISecret: (secret: string) => Promise<void>;
  onDeleteAISecret: () => Promise<void>;
  onStartAIIndex: () => void;
  onCancelAIIndex: () => void;
}) {
  const [ruleName, setRuleName] = useState("");
  const [ruleContains, setRuleContains] = useState("");
  const [ruleTarget, setRuleTarget] = useState("Archiv");

  function addRule(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const name = ruleName.trim();
    const contains = ruleContains.trim();
    if (!name || !contains) return;
    onRulesChange([
      ...rules,
      {
        id: `rule-${crypto.randomUUID()}`,
        name,
        contains,
        target: ruleTarget,
        enabled: true
      }
    ]);
    setRuleName("");
    setRuleContains("");
    setRuleTarget("Archiv");
  }

  if (view === "settings") {
    return (
      <section className="workspace-panel settings-panel">
        <div className="workspace-heading">
          <h2>Einstellungen</h2>
          <p>{accountCount} Konto/Konten lokal eingerichtet. Passwörter bleiben im Windows Credential Manager.</p>
        </div>
        <SettingSwitch label="Automatisch nach App-Start synchronisieren" checked={settings.autoSync} onChange={(value) => onSettingsChange({ ...settings, autoSync: value })} />
        <SettingSwitch label="Desktop-Benachrichtigungen" checked={settings.notifications} onChange={(value) => onSettingsChange({ ...settings, notifications: value })} />
        <SettingSwitch label="Mit Windows starten" checked={settings.launchAtLogin} onChange={(value) => onSettingsChange({ ...settings, launchAtLogin: value })} />
        <SettingSwitch label="Beim Schließen im Infobereich weiterlaufen" checked={settings.minimizeToTray} onChange={(value) => onSettingsChange({ ...settings, minimizeToTray: value })} />
        <SettingSwitch label="Kompakte Nachrichtenliste" checked={settings.compactList} onChange={(value) => onSettingsChange({ ...settings, compactList: value })} />
        <SettingSwitch label="Privatsphäre-Modus für Previews" checked={settings.privacyMode} onChange={(value) => onSettingsChange({ ...settings, privacyMode: value })} />
        <AISettingsPanel
          accounts={accounts}
          settings={aiSettings}
          status={aiSettingsStatus}
          error={aiSettingsError}
          isSaving={isAISettingsSaving}
          indexStatus={aiIndexStatus}
          indexError={aiIndexError}
          isIndexBusy={isAIIndexBusy}
          onSave={onSaveAISettings}
          onSetSecret={onSetAISecret}
          onDeleteSecret={onDeleteAISecret}
          onStartIndex={onStartAIIndex}
          onCancelIndex={onCancelAIIndex}
        />
        <SecurityPanel
          audit={securityAudit}
          isBusy={isSecurityBusy}
          onRepairCredentials={onRepairCredentials}
          onRunAudit={onRunSecurityAudit}
        />
        <OAuthPanel
          accounts={accounts}
          onAccountsChanged={onAccountsChanged}
          providers={oauthProviders}
          clientIds={oauthClientIds}
          onClientIdChange={onOAuthClientIdChange}
        />
      </section>
    );
  }

  if (view === "rules") {
    return (
      <section className="workspace-panel rules-panel">
        <div className="workspace-heading">
          <h2>Regeln</h2>
          <p>Erstelle lokale Regelentwürfe. Sie werden gespeichert und später vor Ausführung bestätigt.</p>
        </div>
        <form className="rule-form" onSubmit={addRule}>
          <input value={ruleName} onChange={(event) => setRuleName(event.target.value)} placeholder="Regelname" />
          <input value={ruleContains} onChange={(event) => setRuleContains(event.target.value)} placeholder="Wenn Betreff/Absender enthält..." />
          <DropdownSelect
            value={ruleTarget}
            ariaLabel="Zielordner der Regel"
            className="form-dropdown"
            options={[
              { value: "Archiv", label: "Archiv", description: "Nachrichten archivieren" },
              { value: "Spam", label: "Spam", description: "Nachrichten als Spam markieren" },
              { value: "Papierkorb", label: "Papierkorb", description: "Nachrichten in den Papierkorb verschieben" },
              { value: "Wichtig", label: "Wichtig", description: "Nachrichten als wichtig markieren" }
            ]}
            onChange={setRuleTarget}
          />
          <button type="submit">Regel hinzufügen</button>
        </form>
        <div className="rule-list">
          {rules.length ? (
            rules.map((rule) => (
              <div className="rule-item" key={rule.id}>
                <div>
                  <strong>{rule.name}</strong>
                  <span>Wenn "{rule.contains}", dann nach {rule.target}</span>
                </div>
                <button onClick={() => onRulesChange(rules.map((item) => item.id === rule.id ? { ...item, enabled: !item.enabled } : item))}>
                  {rule.enabled ? "Aktiv" : "Pausiert"}
                </button>
              </div>
            ))
          ) : (
            <p className="muted-note">Noch keine Regeln erstellt.</p>
          )}
        </div>
      </section>
    );
  }

  return <NewsletterPanel newsletters={newsletters} newsletterCount={newsletterCount} onChanged={onNewsletterChanged} />;
}

function NewsletterPanel({
  newsletters,
  newsletterCount,
  onChanged
}: {
  newsletters: NewsletterSubscription[];
  newsletterCount: number;
  onChanged: () => void | Promise<unknown>;
}) {
  const [busyAction, setBusyAction] = useState<{ id: string; kind: "preview" | "unsubscribe" } | null>(null);
  const [preview, setPreview] = useState<Record<string, string>>({});

  async function previewUnsubscribe(subscription: NewsletterSubscription) {
    if (busyAction) return;
    setBusyAction({ id: subscription.id, kind: "preview" });
    try {
      const result = await api.previewNewsletterUnsubscribe(subscription.id);
      setPreview((current) => ({ ...current, [subscription.id]: result.summary }));
    } catch (error) {
      setPreview((current) => ({
        ...current,
        [subscription.id]: error instanceof Error ? error.message : String(error)
      }));
    } finally {
      setBusyAction(null);
    }
  }

  async function executeUnsubscribe(subscription: NewsletterSubscription) {
    if (busyAction) return;
    const confirmed = window.confirm(`Von ${subscription.sender} abmelden? KizuMail nutzt den erkannten Abmeldeweg und protokolliert die Aktion lokal.`);
    if (!confirmed) return;
    setBusyAction({ id: subscription.id, kind: "unsubscribe" });
    try {
      const result = await api.executeNewsletterUnsubscribe(subscription.id);
      setPreview((current) => ({ ...current, [subscription.id]: result.summary }));
      await onChanged();
    } catch (error) {
      setPreview((current) => ({
        ...current,
        [subscription.id]: error instanceof Error ? error.message : String(error)
      }));
    } finally {
      setBusyAction(null);
    }
  }

  return (
    <section className="workspace-panel newsletter-panel">
      <div className="workspace-heading">
        <h2>Newsletter</h2>
        <p>{newsletterCount} Absender mit Newsletter-Signalen erkannt. Abmeldungen werden erst nach deiner Bestätigung ausgeführt.</p>
      </div>
      <div className="newsletter-subscription-list">
        {newsletters.length ? newsletters.map((subscription) => (
          <article className="newsletter-subscription" key={subscription.id}>
            <SenderAvatar label={subscription.sender} email={subscription.senderEmail} />
            <div>
              <strong>{subscription.sender}</strong>
              <span>{subscription.senderEmail}</span>
              <small>{subscription.messageCount} E-Mails · Methode: {unsubscribeMethodLabel(subscription.unsubscribeMethod)}</small>
              {preview[subscription.id] && <p>{preview[subscription.id]}</p>}
            </div>
            <div className="newsletter-actions">
              <button
                type="button"
                disabled={busyAction !== null}
                onClick={() => void previewUnsubscribe(subscription)}
              >
                {busyAction?.id === subscription.id && busyAction.kind === "preview" ? "Vorschau läuft..." : "Vorschau"}
              </button>
              <button
                type="button"
                disabled={busyAction !== null}
                onClick={() => void executeUnsubscribe(subscription)}
              >
                {busyAction?.id === subscription.id && busyAction.kind === "unsubscribe" ? "Abmeldung läuft..." : "Abmelden"}
              </button>
            </div>
          </article>
        )) : (
          <div className="empty-newsletter-state">
            <ShieldCheck size={34} />
            <strong>Noch keine Newsletter erkannt</strong>
            <span>Nach dem nächsten Sync erscheinen hier Absender mit List-Unsubscribe-Headern.</span>
          </div>
        )}
      </div>
    </section>
  );
}

function SettingSwitch({
  label,
  checked,
  onChange,
  disabled = false
}: {
  label: string;
  checked: boolean;
  onChange: (value: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <label className="setting-switch">
      <span>{label}</span>
      <input type="checkbox" checked={checked} disabled={disabled} onChange={(event) => onChange(event.target.checked)} />
    </label>
  );
}

function AISettingsPanel({
  accounts,
  settings,
  status,
  error,
  isSaving,
  indexStatus,
  indexError,
  isIndexBusy,
  onSave,
  onSetSecret,
  onDeleteSecret,
  onStartIndex,
  onCancelIndex
}: {
  accounts: Account[];
  settings: AISettings | null;
  status: RequestStatus;
  error: string;
  isSaving: boolean;
  indexStatus: AIIndexStatus | null;
  indexError: string;
  isIndexBusy: boolean;
  onSave: (input: AISettingsUpdateInput) => Promise<AISettings>;
  onSetSecret: (secret: string) => Promise<void>;
  onDeleteSecret: () => Promise<void>;
  onStartIndex: () => void;
  onCancelIndex: () => void;
}) {
  const [draft, setDraft] = useState<AISettingsUpdateInput | null>(null);
  const [geminiKey, setGeminiKey] = useState("");
  const [isSecretBusy, setIsSecretBusy] = useState(false);

  useEffect(() => {
    if (!settings) return;
    setDraft(aiSettingsToInput(settings));
  }, [settings]);

  async function persistDraft(next: AISettingsUpdateInput) {
    const normalized = {
      ...next,
      generationModel: next.generationModel.trim(),
      embeddingModel: next.embeddingModel.trim(),
      ollamaBaseUrl: next.ollamaBaseUrl?.trim() || undefined
    };
    setDraft(normalized);
    const saved = await onSave(normalized);
    setDraft(aiSettingsToInput(saved));
  }

  async function saveSecret() {
    const secret = geminiKey.trim();
    if (!secret || !draft) return;
    setIsSecretBusy(true);
    try {
      await onSetSecret(secret);
      await persistDraft(draft);
      setGeminiKey("");
    } catch {
      // Parent state exposes the backend error without retaining the secret.
    } finally {
      setIsSecretBusy(false);
    }
  }

  async function deleteSecret() {
    setIsSecretBusy(true);
    try {
      await onDeleteSecret();
      setGeminiKey("");
    } catch {
      // Parent state exposes the backend error.
    } finally {
      setIsSecretBusy(false);
    }
  }

  if (status === "loading") {
    return <AIStateCard busy title="KI-Einstellungen werden geladen" description="Konfiguration wird aus dem lokalen Backend gelesen." />;
  }
  if (!draft || !settings) {
    return <AIStateCard title="KI-Einstellungen nicht verfügbar" description={error || "Das Backend hat keine Konfiguration geliefert."} tone="error" />;
  }

  const progress = normalizeIndexProgress(indexStatus);
  const indexRunning = indexStatus?.state === "running";
  return (
    <section className="ai-settings-panel">
      <div className="ai-settings-heading">
        <span><Sparkles size={17} /></span>
        <div>
          <h3>KI-Funktionen</h3>
          <p>Zusammenfassen, Antworten, semantisch suchen und lokale Nachrichten indexieren.</p>
        </div>
        <em className={`ai-settings-status ${settings.enabled ? "active" : "inactive"}`}>
          {settings.enabled ? "Aktiv" : "Deaktiviert"}
        </em>
      </div>

      <SettingSwitch
        label="KI global aktivieren"
        checked={draft.enabled}
        disabled={isSaving || isSecretBusy}
        onChange={(enabled) => {
          const next = { ...draft, enabled };
          setDraft(next);
          void persistDraft(next).catch(() => setDraft(aiSettingsToInput(settings)));
        }}
      />

      <div className="ai-settings-grid">
        <label>
          Kontowahl
          <DropdownSelect
            value={draft.accountId || "all"}
            ariaLabel="KI-Konto auswählen"
            className="ai-settings-dropdown"
            options={[
              { value: "all", label: "Alle Konten", description: "Kontofilter pro Aktion möglich" },
              ...accounts.map((account) => ({
                value: account.id,
                label: account.email,
                description: providerLabel(account),
                icon: <ProviderIcon account={account} />
              }))
            ]}
            onChange={(accountId) => setDraft((current) => current
              ? { ...current, accountId: accountId === "all" ? undefined : accountId }
              : current)}
          />
        </label>

        <div className="ai-provider-field">
          <span>Anbieter</span>
          <div className="ai-provider-switch" role="group" aria-label="KI-Anbieter">
            {(["gemini", "ollama"] as AIProvider[]).map((provider) => (
              <button
                type="button"
                key={provider}
                className={draft.provider === provider ? "active" : ""}
                aria-pressed={draft.provider === provider}
                onClick={() => setDraft((current) => current ? { ...current, provider } : current)}
              >
                <Sparkles size={14} />
                {provider === "gemini" ? "Gemini" : "Ollama"}
              </button>
            ))}
          </div>
        </div>

        <label>
          Generierungsmodell
          <input
            value={draft.generationModel}
            onChange={(event) => setDraft((current) => current ? { ...current, generationModel: event.target.value } : current)}
            placeholder="Modellname"
          />
        </label>
        <label>
          Embeddingmodell
          <input
            value={draft.embeddingModel}
            onChange={(event) => setDraft((current) => current ? { ...current, embeddingModel: event.target.value } : current)}
            placeholder="Embeddingmodell"
          />
        </label>
        {draft.provider === "ollama" && (
          <label className="ai-settings-span">
            Ollama-Adresse
            <input
              value={draft.ollamaBaseUrl || ""}
              onChange={(event) => setDraft((current) => current ? { ...current, ollamaBaseUrl: event.target.value } : current)}
              placeholder="http://127.0.0.1:11434"
            />
          </label>
        )}
      </div>

      {draft.provider === "gemini" && (
        <div className="ai-secret-row">
          <label>
            Gemini-Key
            <input
              type="password"
              autoComplete="off"
              value={geminiKey}
              onChange={(event) => setGeminiKey(event.target.value)}
              placeholder={settings.geminiKeyConfigured ? "Key ist im Backend gespeichert" : "Gemini API-Key"}
            />
          </label>
          <button type="button" disabled={isSecretBusy || isSaving || !geminiKey.trim()} onClick={() => void saveSecret()}>
            {isSecretBusy ? "Speichert..." : "Key speichern"}
          </button>
          <button type="button" disabled={isSecretBusy || !settings.geminiKeyConfigured} onClick={() => void deleteSecret()}>
            Key löschen
          </button>
          <small>Der Key wird direkt ans Backend gesendet und nie in localStorage geschrieben.</small>
        </div>
      )}

      <div className="ai-privacy-note">
        <Paperclip size={15} />
        <span><strong>Anhänge sind standardmäßig aus.</strong> Jede Summary- oder Antwortaktion muss sie einmalig freigeben.</span>
      </div>

      {(error || indexError) && <p className="ai-inline-error">{error || indexError}</p>}
      <div className="ai-settings-actions">
        <button
          type="button"
          disabled={isSaving || !draft.generationModel.trim() || !draft.embeddingModel.trim()}
          onClick={() => void persistDraft(draft).catch(() => undefined)}
        >
          {isSaving ? "Speichert..." : "KI-Einstellungen speichern"}
        </button>
      </div>

      <div className={`ai-index-panel state-${indexStatus?.state || "not_started"}`}>
        <header>
          <div>
            <strong>Lokaler Nachrichtenindex</strong>
            <span>{indexStatusLabel(indexStatus)}</span>
          </div>
          <b>{Math.round(progress)}%</b>
        </header>
        <div className="ai-index-track" aria-label={`Indexfortschritt ${Math.round(progress)} Prozent`}>
          <span style={{ width: `${progress}%` }} />
        </div>
        <p>
          {indexStatus?.message || (indexStatus?.total
            ? `${indexStatus.processed} von ${indexStatus.total} Nachrichten verarbeitet.`
            : "Noch keine Nachrichten indexiert.")}
        </p>
        <div>
          <button type="button" disabled={!settings.enabled || isIndexBusy || indexRunning} onClick={onStartIndex}>
            <Sparkles size={14} />
            {indexStatus?.state === "complete" ? "Neu indexieren" : "Index starten"}
          </button>
          <button type="button" disabled={!indexRunning || isIndexBusy} onClick={onCancelIndex}>
            Abbrechen
          </button>
        </div>
      </div>
    </section>
  );
}

function SecurityPanel({
  audit,
  isBusy,
  onRepairCredentials,
  onRunAudit
}: {
  audit: SecurityAuditResult | null;
  isBusy: boolean;
  onRepairCredentials: () => void;
  onRunAudit: () => void;
}) {
  return (
    <div className="security-panel">
      <div>
        <h3>Sicherheit & Anmeldung</h3>
        <p>Prüft lokal, ob Passwörter und OAuth-Tokens im Windows Credential Manager wiedergefunden werden. Secrets werden nicht angezeigt.</p>
      </div>
      <div className="security-actions">
        <button type="button" onClick={onRepairCredentials} disabled={isBusy}>
          {isBusy ? "Prüft..." : "Anmeldungen reparieren"}
        </button>
        <button type="button" onClick={onRunAudit} disabled={isBusy}>
          Sicherheitscheck starten
        </button>
      </div>
      {audit && (
        <div className={`security-audit ${audit.ok ? "ok" : "warn"}`}>
          <strong>{audit.summary}</strong>
          {audit.checks.map((check) => (
            <div className="security-check" key={check.label}>
              <span>{check.ok ? <Check size={14} /> : <X size={14} />}</span>
              <div>
                <b>{check.label}</b>
                <small>{check.details}</small>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function OAuthPanel({
  accounts,
  onAccountsChanged,
  providers,
  clientIds,
  onClientIdChange
}: {
  accounts: Account[];
  onAccountsChanged: () => void;
  providers: OAuthProviderStatus[];
  clientIds: Record<string, string>;
  onClientIdChange: (provider: string, clientId: string) => void;
}) {
  const [showDeveloperOptions, setShowDeveloperOptions] = useState(false);
  const oauthAccountCount = accounts.filter((account) => account.authType === "oauth").length;
  const missingRegistration = providers.some((provider) => !provider.clientIdConfigured && !clientIds[provider.provider]?.trim());

  return (
    <div className="oauth-panel">
      <div>
        <h3>Anmeldung über Anbieter</h3>
        <p>Gmail und Microsoft/Hotmail verwenden die sichere Browser-Anmeldung. Wenn eine Anbieter-App noch nicht eingerichtet ist, bleibt nur dieser jeweilige Anbieter-Button deaktiviert.</p>
      </div>
      <div className="oauth-status-list">
        {providers.map((provider) => {
          const configured = provider.clientIdConfigured || Boolean(clientIds[provider.provider]?.trim());
          return (
            <div className={configured ? "oauth-status ready" : "oauth-status missing"} key={provider.provider}>
              <strong>{provider.label}</strong>
              <span>{configured ? "Browser-Anmeldung bereit" : provider.summary}</span>
            </div>
          );
        })}
      </div>
      <div className="connection-result ok">
        <strong>{oauthAccountCount} OAuth-Konto/Konten verbunden</strong>
        <span>Neue Konten verbindest du über “Konto verbinden” links oder im rechten Optionenbereich.</span>
      </div>
      {missingRegistration && (
        <>
          <button type="button" className="advanced-toggle" onClick={() => setShowDeveloperOptions((value) => !value)}>
            {showDeveloperOptions ? "Entwickleroptionen ausblenden" : "Entwickleroptionen anzeigen"}
            <ChevronDown size={15} />
          </button>
          {showDeveloperOptions && (
            <div className="developer-oauth-grid">
              {providers.map((provider) => (
                <label key={provider.provider}>
                  {provider.label} Client-ID
                  <input
                    value={clientIds[provider.provider] ?? ""}
                    onChange={(event) => onClientIdChange(provider.provider, event.target.value)}
                    placeholder="Nur für Entwicklungs-Builds nötig"
                  />
                </label>
              ))}
            </div>
          )}
        </>
      )}
      <button type="button" onClick={onAccountsChanged}>Kontostatus aktualisieren</button>
    </div>
  );
}

function Inspector({
  accountCount,
  messageCount,
  hasAccounts,
  newsletters,
  selectedMessage,
  senderStats,
  onToggleSenderBlocked,
  onConnect,
  onOpenRules,
  onOpenSettings
}: {
  accountCount: number;
  messageCount: number;
  hasAccounts: boolean;
  newsletters: NewsletterSubscription[];
  selectedMessage: MessageSummary | null;
  senderStats: SenderStats | null;
  onToggleSenderBlocked: () => void;
  onConnect: () => void;
  onOpenRules: () => void;
  onOpenSettings: () => void;
}) {
  const newsletterCount = newsletters.length;
  const activeSender = senderStats ?? (selectedMessage ? {
    sender: selectedMessage.sender,
    senderEmail: selectedMessage.senderEmail,
    domain: senderDomain(selectedMessage.senderEmail),
    firstMessageAt: undefined,
    lastMessageAt: selectedMessage.date,
    messageCount: 1,
    newsletter: selectedMessage.hasNewsletterSignals,
    trusted: true,
    blocked: false
  } : null);
  return (
    <div className="inspector-content">
      <section>
        <h3>{activeSender ? "Absender" : "Status"}</h3>
        {activeSender ? (
          <div className="sender-card sender-profile-card">
            <SenderAvatar label={activeSender.sender} email={activeSender.senderEmail} variant="hero" />
            <div>
              <strong>{activeSender.sender}</strong>
              <span>{activeSender.senderEmail}</span>
              <div className="sender-profile-actions">
                <button title="Kontakt" aria-label="Kontakt"><User size={14} /></button>
                <button title="E-Mail" aria-label="E-Mail"><Mail size={14} /></button>
                <button
                  className={activeSender.blocked ? "danger active" : "danger"}
                  onClick={onToggleSenderBlocked}
                  title={activeSender.blocked ? "Blockierung aufheben" : "Absender blockieren"}
                  aria-label={activeSender.blocked ? "Blockierung aufheben" : "Absender blockieren"}
                >
                  <Ban size={14} />
                </button>
              </div>
            </div>
          </div>
        ) : (
          <div className="sender-card empty">
            <ShieldCheck size={26} />
            <div>
              <strong>{hasAccounts ? "Konto verbunden" : "Kein Konto"}</strong>
              <span>{hasAccounts ? "IMAP-Sync lokal verfügbar" : "Noch keine lokalen Maildaten"}</span>
            </div>
          </div>
        )}
      </section>

      {activeSender ? (
        <section className="info-block">
          <h3>Informationen</h3>
          <div className="sender-intelligence-summary">
            <div>
              <strong>{activeSender.messageCount}</strong>
              <span>E-Mails dieses Absenders</span>
            </div>
            <small className={activeSender.blocked ? "blocked" : activeSender.trusted ? "trusted" : ""}>
              {activeSender.blocked ? "Blockiert" : activeSender.trusted ? "Vertrauenswürdig" : "Unbekannt"}
            </small>
          </div>
          <InfoLine label="Erste E-Mail" value={activeSender.firstMessageAt ? formatMessageListDate(activeSender.firstMessageAt) : "Unbekannt"} />
          <InfoLine label="Letzte E-Mail" value={activeSender.lastMessageAt ? formatFullDateTime(activeSender.lastMessageAt) : "Unbekannt"} />
          <InfoLine label="Gesamt" value={`${activeSender.messageCount} E-Mails`} strong={activeSender.messageCount > 1} />
          <InfoLine label="Newsletter" value={activeSender.newsletter ? "Aktiv" : "Nein"} strong={activeSender.newsletter} />
          <InfoLine label="Vertrauen" value={activeSender.blocked ? "Blockiert" : activeSender.trusted ? "Hoch" : "Normal"} strong={!activeSender.blocked} />
        </section>
      ) : (
        <section className="info-block">
          <h3>Lokale Daten</h3>
          <InfoLine label="Konten" value={accountCount.toString()} strong={hasAccounts} />
          <InfoLine label="Newsletter" value={newsletterCount.toString()} />
          <InfoLine label="Abmeldelinks" value="0" />
        </section>
      )}

      <section className="chart-block">
        <div className="chart-header">
          <span>E-Mails</span>
          <strong>{messageCount}</strong>
        </div>
        <div className="bars empty-bars">
          {Array.from({ length: 12 }).map((_, index) => (
            <span key={index} style={{ height: `${messageCount ? 18 + (index % 5) * 9 : 8}%` }} />
          ))}
        </div>
        <div className="months">
          <span>Mär</span>
          <span>Apr</span>
          <span>Mai</span>
          <span>Jun</span>
          <span>Jul</span>
        </div>
      </section>

      <section className="option-block">
        <h3>Optionen</h3>
        <button onClick={onConnect}><UserPlus size={15} />Konto verbinden</button>
        {activeSender && <button onClick={onToggleSenderBlocked}><Ban size={15} />{activeSender.blocked ? "Blockierung aufheben" : "Absender blockieren"}</button>}
        <button onClick={onOpenRules}><SlidersHorizontal size={15} />Regel erstellen</button>
        <button onClick={onOpenSettings}><Settings size={15} />Einstellungen öffnen</button>
      </section>
    </div>
  );
}

function InfoLine({ label, value, strong }: { label: string; value: string; strong?: boolean }) {
  return (
    <div className="info-line">
      <span>{label}</span>
      <strong className={strong ? "success" : ""}>
        {value}
        {strong && <Check size={13} />}
      </strong>
    </div>
  );
}

function PasswordRecoveryModal({
  account,
  password,
  isSaving,
  onChange,
  onClose,
  onSubmit
}: {
  account: Account | null;
  password: string;
  isSaving: boolean;
  onChange: (value: string) => void;
  onClose: () => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
}) {
  return (
    <div className="modal-backdrop" role="presentation">
      <form className="account-modal" onSubmit={onSubmit}>
        <div className="modal-header">
          <div>
            <h2>Passwort erneut speichern</h2>
            <p>
              Für {account?.email ?? "dieses Konto"} fehlt der Eintrag im Windows Credential Manager.
              Gib das Passwort oder App-Passwort erneut ein, danach startet KizuMail den Sync sofort neu.
            </p>
          </div>
          <button type="button" onClick={onClose} aria-label="Schließen"><X size={18} /></button>
        </div>
        <label>
          Passwort oder App-Passwort
          <input
            value={password}
            onChange={(event) => onChange(event.target.value)}
            type="password"
            autoComplete="current-password"
            required
          />
        </label>
        <div className="modal-actions">
          <button type="button" onClick={onClose}>Abbrechen</button>
          <button type="submit" disabled={isSaving}>{isSaving ? "Synchronisiere..." : "Speichern & synchronisieren"}</button>
        </div>
      </form>
    </div>
  );
}

function AccountModal({
  form,
  isSaving,
  isTesting,
  testResult,
  oauthProviders,
  oauthClientIds,
  oauthConnectingProvider,
  onChange,
  onClose,
  onSubmit,
  onTestConnection,
  onOAuthConnect,
  onOAuthClientIdChange
}: {
  form: CreateImapAccountInput;
  isSaving: boolean;
  isTesting: boolean;
  testResult: ImapConnectionTestResult | null;
  oauthProviders: OAuthProviderStatus[];
  oauthClientIds: Record<string, string>;
  oauthConnectingProvider: string | null;
  onChange: (value: CreateImapAccountInput) => void;
  onClose: () => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
  onTestConnection: () => void;
  onOAuthConnect: (provider: "gmail" | "microsoft") => void;
  onOAuthClientIdChange: (provider: string, clientId: string) => void;
}) {
  const providerPreset = getProviderPreset(form.email);
  const [showAdvanced, setShowAdvanced] = useState(!providerPreset);
  const [showManualLogin, setShowManualLogin] = useState(false);
  const [showDeveloperOptions, setShowDeveloperOptions] = useState(false);
  const gmailStatus = oauthProviders.find((provider) => provider.provider === "gmail");
  const microsoftStatus = oauthProviders.find((provider) => provider.provider === "microsoft");
  const gmailReady = Boolean(gmailStatus?.clientIdConfigured || oauthClientIds.gmail?.trim());
  const microsoftReady = Boolean(microsoftStatus?.clientIdConfigured || oauthClientIds.microsoft?.trim());
  const emailDomain = form.email.split("@")[1]?.trim().toLowerCase() ?? "";
  const isGmailAddress = emailDomain === "gmail.com" || emailDomain === "googlemail.com";
  const isMicrosoftAddress = ["hotmail.com", "hotmail.de", "outlook.com", "outlook.de", "live.com", "live.de", "msn.com"].includes(emailDomain);
  const microsoftBlocked = isMicrosoftAddress && !microsoftReady;
  const isIcloudAddress = ["icloud.com", "me.com", "mac.com"].includes(emailDomain);
  const hasEmailAddress = form.email.includes("@") && emailDomain.length > 0;
  const preferredOAuthProvider: "gmail" | "microsoft" | null = isGmailAddress && gmailReady
    ? "gmail"
    : isMicrosoftAddress && microsoftReady
      ? "microsoft"
      : null;
  const showManualFields = hasEmailAddress && (!preferredOAuthProvider || showManualLogin);

  useEffect(() => {
    setShowManualLogin(false);
  }, [preferredOAuthProvider, emailDomain]);

  function handleEmailChange(email: string) {
    const next = applyProviderPreset({ ...form, email });
    onChange(next);
    setShowAdvanced(!getProviderPreset(email));
  }

  function handleModalSubmit(event: FormEvent<HTMLFormElement>) {
    if (preferredOAuthProvider && !showManualLogin) {
      event.preventDefault();
      onOAuthConnect(preferredOAuthProvider);
      return;
    }
    onSubmit(event);
  }

  return (
    <div className="modal-backdrop" role="presentation">
      <form className="account-modal" onSubmit={handleModalSubmit}>
        <div className="modal-header">
          <div>
            <h2>E-Mail-Konto verbinden</h2>
            <p>Adresse eingeben, Anbieter bestätigen, fertig.</p>
          </div>
          <button type="button" onClick={onClose} aria-label="Schließen"><X size={18} /></button>
        </div>

        <label>
          E-Mail-Adresse
          <input value={form.email} onChange={(event) => handleEmailChange(event.target.value)} placeholder="name@hotmail.com" type="email" required />
        </label>
        {hasEmailAddress && (
          <label>
            Anzeigename <span className="optional-label">Optional</span>
            <input value={form.displayName} onChange={(event) => onChange({ ...form, displayName: event.target.value })} placeholder="Privat" />
          </label>
        )}

        {hasEmailAddress && preferredOAuthProvider ? (
          <div className="provider-login-primary">
            <button
              type="button"
              className="provider-login-card primary"
              onClick={() => onOAuthConnect(preferredOAuthProvider)}
              disabled={oauthConnectingProvider !== null || !form.email.trim()}
            >
              {preferredOAuthProvider === "gmail" ? <Mail size={22} /> : <MailOpen size={22} />}
              <span>
                <strong>
                  {oauthConnectingProvider === preferredOAuthProvider
                    ? "Anmeldung wird geöffnet..."
                    : preferredOAuthProvider === "gmail"
                      ? "Mit Gmail anmelden"
                      : "Mit Outlook/Hotmail anmelden"}
                </strong>
                <small>Browser-Anmeldung mit Zwei-Faktor-Bestätigung. Kein Passwort in KizuMail eingeben.</small>
              </span>
            </button>
            <button className="advanced-toggle" type="button" onClick={() => setShowManualLogin((value) => !value)}>
              {showManualLogin ? "App-Passwort/IMAP ausblenden" : "Stattdessen App-Passwort/IMAP verwenden"}
              <ChevronDown size={15} />
            </button>
          </div>
        ) : hasEmailAddress && (isGmailAddress || isMicrosoftAddress) ? (
          <div className="provider-login-grid">
            {isGmailAddress && (
              <button type="button" className="provider-login-card" onClick={() => onOAuthConnect("gmail")} disabled={oauthConnectingProvider !== null || !gmailReady}>
                <Mail size={22} />
                <span>
                  <strong>{oauthConnectingProvider === "gmail" ? "Gmail wird geöffnet..." : "Mit Gmail verbinden"}</strong>
                  <small>{gmailReady ? "Im Browser bestätigen" : "Anbieter-Login noch nicht eingerichtet"}</small>
                </span>
              </button>
            )}
            {isMicrosoftAddress && (
              <button type="button" className="provider-login-card" onClick={() => onOAuthConnect("microsoft")} disabled={oauthConnectingProvider !== null || !microsoftReady}>
                <MailOpen size={22} />
                <span>
                  <strong>{oauthConnectingProvider === "microsoft" ? "Microsoft wird geöffnet..." : "Mit Outlook verbinden"}</strong>
                  <small>{microsoftReady ? "Im Browser bestätigen" : "Anbieter-Login noch nicht eingerichtet"}</small>
                </span>
              </button>
            )}
          </div>
        ) : hasEmailAddress && !isIcloudAddress ? (
          <div className="provider-login-grid provider-choice-grid">
            <button type="button" className="provider-login-card" onClick={() => onOAuthConnect("microsoft")} disabled={oauthConnectingProvider !== null || !microsoftReady}>
              <MailOpen size={22} />
              <span>
                <strong>{oauthConnectingProvider === "microsoft" ? "Microsoft wird geöffnet..." : "Microsoft 365 / Outlook"}</strong>
                <small>{microsoftReady ? "Für Hotmail, Outlook und Microsoft-365-Konten" : "Client-ID später in Einstellungen hinterlegen"}</small>
              </span>
            </button>
            <button type="button" className="provider-login-card" onClick={() => onOAuthConnect("gmail")} disabled={oauthConnectingProvider !== null || !gmailReady}>
              <Mail size={22} />
              <span>
                <strong>{oauthConnectingProvider === "gmail" ? "Google wird geöffnet..." : "Google Workspace"}</strong>
                <small>{gmailReady ? "Für Gmail und Workspace-Domains" : "Client-ID später in Einstellungen hinterlegen"}</small>
              </span>
            </button>
          </div>
        ) : null}

        {hasEmailAddress && microsoftBlocked && (
          <div className="connection-result warn">
            <strong>Outlook-Anmeldung ist in diesem Build noch nicht verfügbar</strong>
            <span>Du kannst die Einrichtung später unter den erweiterten Optionen ergänzen.</span>
          </div>
        )}

        {showManualFields && (
          <>
            <div className="login-separator"><span>{isIcloudAddress ? "iCloud app-spezifisches Passwort" : "App-Passwort/IMAP"}</span></div>

            <label>
              Passwort oder App-Passwort
              <input
                value={form.password}
                onChange={(event) => onChange({ ...form, password: event.target.value })}
                placeholder={isIcloudAddress ? "iCloud App-Passwort" : "App-Passwort, nicht normales Konto-Passwort"}
                type="password"
                autoComplete="current-password"
                required={showManualFields}
              />
            </label>
          </>
        )}

        {hasEmailAddress && ((isGmailAddress && !gmailReady) || (isMicrosoftAddress && !microsoftReady)) && (
          <>
            <button className="advanced-toggle" type="button" onClick={() => setShowDeveloperOptions((value) => !value)}>
              {showDeveloperOptions ? "Entwickleroptionen ausblenden" : "Entwickleroptionen für Anbieter-Login"}
              <ChevronDown size={15} />
            </button>
            {showDeveloperOptions && (
              <div className="developer-oauth-grid">
                <label>
                  Gmail Client-ID
                  <input
                    value={oauthClientIds.gmail ?? ""}
                    onChange={(event) => onOAuthClientIdChange("gmail", event.target.value)}
                    placeholder="Nur nötig, wenn diese Build-Version nicht registriert ist"
                  />
                </label>
                <label>
                  Microsoft Client-ID
                  <input
                    value={oauthClientIds.microsoft ?? ""}
                    onChange={(event) => onOAuthClientIdChange("microsoft", event.target.value)}
                    placeholder="Nur nötig, wenn diese Build-Version nicht registriert ist"
                  />
                </label>
              </div>
            )}
          </>
        )}

        {showManualFields && (
          <button className="advanced-toggle" type="button" onClick={() => setShowAdvanced((value) => !value)}>
            {showAdvanced ? "Serverdaten ausblenden" : "Serverdaten anzeigen"}
            <ChevronDown size={15} />
          </button>
        )}

        {showManualFields && showAdvanced && (
          <>
            <div className="modal-grid">
              <label>
                IMAP Host
                <input value={form.imapHost} onChange={(event) => onChange({ ...form, imapHost: event.target.value })} placeholder="imap.example.com" required />
              </label>
              <label>
                Port
                <input value={form.imapPort} onChange={(event) => onChange({ ...form, imapPort: Number(event.target.value) })} type="number" min={1} required />
              </label>
            </div>
            <div className="modal-grid">
              <label>
                SMTP Host
                <input value={form.smtpHost} onChange={(event) => onChange({ ...form, smtpHost: event.target.value })} placeholder="smtp.example.com" required />
              </label>
              <label>
                Port
                <input value={form.smtpPort} onChange={(event) => onChange({ ...form, smtpPort: Number(event.target.value) })} type="number" min={1} required />
              </label>
            </div>
          </>
        )}

        {testResult && (
          <div className={testResult.ok ? "connection-result ok" : "connection-result error"}>
            <strong>{testResult.summary}</strong>
            <span>{testResult.details}</span>
          </div>
        )}

        <div className="modal-actions">
          {showManualFields && (
            <button type="button" onClick={onTestConnection} disabled={isTesting}>
              {isTesting ? "Teste..." : "Login testen"}
            </button>
          )}
          <button type="button" onClick={onClose}>Abbrechen</button>
          <button type="submit" disabled={isSaving || oauthConnectingProvider !== null || !form.email.trim()}>
            {!hasEmailAddress
              ? "Weiter"
              : preferredOAuthProvider && !showManualLogin
              ? "Browser-Anmeldung starten"
              : isSaving
                ? "Synchronisiere..."
                : "Speichern & synchronisieren"}
          </button>
        </div>
      </form>
    </div>
  );
}

function ComposerModal({
  accounts,
  initialDraft,
  selectedAccountId,
  isSending,
  onClose,
  onSubmit
}: {
  accounts: Account[];
  initialDraft: ComposerDraft;
  selectedAccountId: string;
  isSending: boolean;
  onClose: () => void;
  onSubmit: (input: { accountId: string; to: string; subject: string; body: string }) => void;
}) {
  const [accountId, setAccountId] = useState(initialDraft.accountId || selectedAccountId);
  const [to, setTo] = useState(initialDraft.to);
  const [subject, setSubject] = useState(initialDraft.subject);
  const [body, setBody] = useState(initialDraft.body);

  return (
    <div className="modal-backdrop" role="presentation">
      <form
        className="account-modal composer-modal"
        onSubmit={(event) => {
          event.preventDefault();
          onSubmit({ accountId, to, subject, body });
        }}
      >
        <div className="modal-header">
          <div>
            <h2>Neue E-Mail</h2>
            <p>Versand läuft über SMTP mit dem ausgewählten Konto. Passwörter bleiben im Credential Manager oder Sitzungsspeicher.</p>
          </div>
          <button type="button" onClick={onClose} aria-label="Schließen"><X size={18} /></button>
        </div>
        <label>
          Von
          <DropdownSelect
            value={accountId}
            ariaLabel="Absenderkonto auswählen"
            className="composer-account-dropdown"
            options={accounts.map((account) => ({
              value: account.id,
              label: account.email,
              description: providerLabel(account),
              icon: <ProviderIcon account={account} />
            }))}
            onChange={setAccountId}
          />
        </label>
        <label>
          An
          <input value={to} onChange={(event) => setTo(event.target.value)} placeholder="empfaenger@example.com" type="email" required />
        </label>
        <label>
          Betreff
          <input value={subject} onChange={(event) => setSubject(event.target.value)} placeholder="Betreff" required />
        </label>
        <label>
          Nachricht
          <textarea value={body} onChange={(event) => setBody(event.target.value)} placeholder="Nachricht schreiben..." rows={7} />
        </label>
        <div className="modal-actions">
          <button type="button" onClick={onClose}>Schließen</button>
          <button type="submit" disabled={isSending || !accountId}>{isSending ? "Sendet..." : "Per SMTP senden"}</button>
        </div>
      </form>
    </div>
  );
}

interface ProviderPreset {
  label: string;
  imapHost: string;
  imapPort: number;
  smtpHost: string;
  smtpPort: number;
}

function getProviderPreset(email: string): ProviderPreset | null {
  const domain = email.split("@")[1]?.trim().toLowerCase();
  if (!domain) return null;

  const presets: Record<string, ProviderPreset> = {
    "hotmail.com": outlookPreset,
    "hotmail.de": outlookPreset,
    "outlook.com": outlookPreset,
    "outlook.de": outlookPreset,
    "live.com": outlookPreset,
    "live.de": outlookPreset,
    "msn.com": outlookPreset,
    "gmail.com": gmailPreset,
    "googlemail.com": gmailPreset,
    "icloud.com": icloudPreset,
    "me.com": icloudPreset,
    "mac.com": icloudPreset,
    "yahoo.com": yahooPreset,
    "yahoo.de": yahooPreset,
    "gmx.de": gmxPreset,
    "gmx.net": gmxPreset,
    "web.de": webDePreset
  };

  return presets[domain] ?? null;
}

const outlookPreset: ProviderPreset = {
  label: "Hotmail/Outlook",
  imapHost: "outlook.office365.com",
  imapPort: 993,
  smtpHost: "smtp-mail.outlook.com",
  smtpPort: 587
};

const gmailPreset: ProviderPreset = {
  label: "Gmail",
  imapHost: "imap.gmail.com",
  imapPort: 993,
  smtpHost: "smtp.gmail.com",
  smtpPort: 587
};

const icloudPreset: ProviderPreset = {
  label: "iCloud Mail",
  imapHost: "imap.mail.me.com",
  imapPort: 993,
  smtpHost: "smtp.mail.me.com",
  smtpPort: 587
};

const yahooPreset: ProviderPreset = {
  label: "Yahoo Mail",
  imapHost: "imap.mail.yahoo.com",
  imapPort: 993,
  smtpHost: "smtp.mail.yahoo.com",
  smtpPort: 587
};

const gmxPreset: ProviderPreset = {
  label: "GMX",
  imapHost: "imap.gmx.net",
  imapPort: 993,
  smtpHost: "mail.gmx.net",
  smtpPort: 587
};

const webDePreset: ProviderPreset = {
  label: "WEB.DE",
  imapHost: "imap.web.de",
  imapPort: 993,
  smtpHost: "smtp.web.de",
  smtpPort: 587
};

function applyProviderPreset(form: CreateImapAccountInput): CreateImapAccountInput {
  const preset = getProviderPreset(form.email);
  if (!preset) return form;
  return {
    ...form,
    displayName: form.displayName || form.email.split("@")[0] || "Privat",
    imapHost: preset.imapHost,
    imapPort: preset.imapPort,
    smtpHost: preset.smtpHost,
    smtpPort: preset.smtpPort
  };
}

function loadThemeMode(): ThemeMode {
  try {
    const raw = localStorage.getItem("kizumail.theme");
    return raw === "light" || raw === "dark" || raw === "system" ? raw : "dark";
  } catch {
    return "dark";
  }
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function desktopPreferencesInput(settings: LocalSettings) {
  return {
    autoSync: settings.autoSync,
    notifications: settings.notifications,
    privacyMode: settings.privacyMode,
    launchAtLogin: settings.launchAtLogin,
    minimizeToTray: settings.minimizeToTray
  };
}

function isTauriDesktop() {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function sendNewMailNotifications(results: SyncResult[], settings: LocalSettings) {
  if (!settings.notifications || !isTauriDesktop()) return;
  if (!(await isPermissionGranted())) return;

  const messages = results.flatMap((result) => result.ok ? result.newMessages : []);
  const uniqueMessages = [...new Map(messages.map((message) => [message.messageId, message])).values()];
  for (const message of uniqueMessages.slice(0, 5)) {
    sendNotification({
      title: settings.privacyMode ? "Neue E-Mail" : message.sender || "Neue E-Mail",
      body: settings.privacyMode
        ? `Neue Nachricht in KizuMail von ${message.sender || "einem Absender"}.`
        : message.subject || "Neue Nachricht eingegangen."
    });
  }
  if (uniqueMessages.length > 5) {
    sendNotification({
      title: "KizuMail",
      body: `${uniqueMessages.length - 5} weitere neue E-Mails sind eingegangen.`
    });
  }
}

function syncResultMessage(results: SyncResult[], successMessage: string) {
  const failures = results.filter((result) => !result.ok);
  if (!failures.length) return successMessage;
  const firstFailure = failures[0];
  const prefix = failures.length === results.length
    ? "Synchronisierung fehlgeschlagen"
    : `${results.length - failures.length} von ${results.length} Konten synchronisiert`;
  return `${prefix}: ${firstFailure.details}`;
}

function aiSettingsToInput(settings: AISettings): AISettingsUpdateInput {
  return {
    enabled: settings.enabled,
    accountId: settings.accountId,
    provider: settings.provider,
    generationModel: settings.generationModel,
    embeddingModel: settings.embeddingModel,
    ollamaBaseUrl: settings.ollamaBaseUrl
  };
}

function aiSearchStatusLabel(state: AIRequestState<AISearchResult>) {
  if (state.status === "loading") return "KI-Suche läuft";
  if (state.status === "error") return "KI-Suche fehlgeschlagen";
  if (state.status === "success") return `${state.data?.hits.length ?? 0} belegte Treffer`;
  return "KI-Suche";
}

function formatAIScore(score: number) {
  const normalized = score <= 1 ? score * 100 : score;
  return `${Math.round(Math.max(0, Math.min(100, normalized)))}%`;
}

function normalizeIndexProgress(status: AIIndexStatus | null) {
  if (!status) return 0;
  if (typeof status.progress === "number") {
    const progress = status.progress <= 1 ? status.progress * 100 : status.progress;
    return Math.max(0, Math.min(100, progress));
  }
  if (status.total <= 0) return status.state === "complete" ? 100 : 0;
  return Math.max(0, Math.min(100, (status.processed / status.total) * 100));
}

function indexStatusLabel(status: AIIndexStatus | null) {
  switch (status?.state) {
    case "running":
      return "Indexierung läuft";
    case "complete":
      return "Index ist aktuell";
    case "cancelled":
      return "Indexierung abgebrochen";
    case "error":
      return status.lastError || "Indexierung fehlgeschlagen";
    case "idle":
      return "Bereit zur Indexierung";
    case "not_started":
    default:
      return "Noch nicht gestartet";
  }
}

function resolveTheme(mode: ThemeMode): "dark" | "light" {
  if (mode === "light" || mode === "dark") return mode;
  return "dark";
}

function loadLocalSettings(): LocalSettings {
  try {
    const raw = localStorage.getItem("kizumail.settings");
    if (!raw) return defaultSettings;
    const parsed = JSON.parse(raw) as Partial<LocalSettings>;
    return {
      autoSync: typeof parsed.autoSync === "boolean" ? parsed.autoSync : defaultSettings.autoSync,
      notifications: typeof parsed.notifications === "boolean" ? parsed.notifications : defaultSettings.notifications,
      compactList: typeof parsed.compactList === "boolean" ? parsed.compactList : defaultSettings.compactList,
      privacyMode: typeof parsed.privacyMode === "boolean" ? parsed.privacyMode : defaultSettings.privacyMode,
      launchAtLogin: typeof parsed.launchAtLogin === "boolean" ? parsed.launchAtLogin : defaultSettings.launchAtLogin,
      minimizeToTray: typeof parsed.minimizeToTray === "boolean" ? parsed.minimizeToTray : defaultSettings.minimizeToTray
    };
  } catch {
    return defaultSettings;
  }
}

function loadLocalRules(): LocalRule[] {
  try {
    const raw = localStorage.getItem("kizumail.rules");
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

function loadOAuthClientIds(): Record<string, string> {
  try {
    const raw = localStorage.getItem("kizumail.oauthClientIds");
    return raw ? JSON.parse(raw) : {};
  } catch {
    return {};
  }
}

function messageListInputForView(view: View, listFilter: ListFilter): MessageListInput {
  const input: MessageListInput = {};
  switch (view) {
    case "inbox":
    case "sent":
    case "drafts":
    case "archive":
    case "spam":
    case "trash":
      input.folderRole = view;
      break;
    case "starred":
    case "important":
      input.flagged = true;
      break;
    case "unread":
      input.unread = true;
      break;
    case "attachments":
      input.hasAttachments = true;
      break;
    case "today": {
      const from = startOfDay(new Date());
      const to = new Date(from);
      to.setDate(to.getDate() + 1);
      input.dateFrom = from.toISOString();
      input.dateTo = to.toISOString();
      break;
    }
    default:
      break;
  }

  if (listFilter === "unread") input.unread = true;
  if (listFilter === "flagged") input.flagged = true;
  if (listFilter === "attachments") input.hasAttachments = true;
  return input;
}

function messageCacheKey(query: {
  sort: MessageSortMode;
  accountFilter: string;
  activeView: View;
  listFilter: ListFilter;
}) {
  return [query.activeView, query.listFilter, query.sort, query.accountFilter].join(":");
}

function mergeMessages(current: MessageSummary[], incoming: MessageSummary[]) {
  const merged = new Map(current.map((message) => [message.id, message]));
  for (const message of incoming) merged.set(message.id, message);
  return Array.from(merged.values());
}

function messageMatchesView(message: MessageSummary, view: View) {
  const folder = message.folder.toLowerCase();
  const folderRole = message.folderRole?.toLowerCase();
  const today = new Date().toDateString();
  switch (view) {
    case "inbox":
      return folderRole === "inbox" || folder.includes("inbox") || folder.includes("posteingang");
    case "starred":
    case "important":
      return message.flagged || message.subject.toLowerCase().includes("important") || message.subject.toLowerCase().includes("wichtig");
    case "sent":
      return folderRole === "sent" || folder.includes("sent") || folder.includes("gesendet");
    case "drafts":
      return folderRole === "drafts" || folder.includes("draft") || folder.includes("entwurf");
    case "archive":
      return folderRole === "archive" || folder.includes("archive") || folder.includes("archiv") || folder.includes("alle nachrichten");
    case "spam":
      return folderRole === "spam" || folder.includes("spam") || folder.includes("junk");
    case "trash":
      return folderRole === "trash" || folder.includes("trash") || folder.includes("papierkorb") || folder.includes("deleted");
    case "unread":
      return message.unread;
    case "attachments":
      return message.hasAttachments || message.preview.toLowerCase().includes("attachment") || message.preview.toLowerCase().includes("anhang");
    case "today":
      return new Date(message.date).toDateString() === today;
    case "newsletters":
      return message.hasNewsletterSignals;
    case "rules":
    case "settings":
      return false;
    default:
      return true;
  }
}

function sortMessages(messages: MessageSummary[], sortMode: MessageSortMode) {
  return [...messages].sort((left, right) => {
    switch (sortMode) {
      case "oldest":
        return messageTimestamp(left) - messageTimestamp(right);
      case "unread":
        return Number(right.unread) - Number(left.unread) || messageTimestamp(right) - messageTimestamp(left);
      case "sender":
        return left.sender.localeCompare(right.sender, "de", { sensitivity: "base" }) || messageTimestamp(right) - messageTimestamp(left);
      case "subject":
        return left.subject.localeCompare(right.subject, "de", { sensitivity: "base" }) || messageTimestamp(right) - messageTimestamp(left);
      case "attachments":
        return Number(right.hasAttachments) - Number(left.hasAttachments) || messageTimestamp(right) - messageTimestamp(left);
      case "newest":
      default:
        return messageTimestamp(right) - messageTimestamp(left);
    }
  });
}

function groupMessagesByDate(messages: MessageSummary[]) {
  const groups = new Map<string, MessageSummary[]>();
  for (const message of messages) {
    const label = messageDateGroupLabel(message.date);
    groups.set(label, [...(groups.get(label) ?? []), message]);
  }
  return Array.from(groups.entries()).map(([label, groupedMessages]) => ({
    label,
    messages: groupedMessages
  }));
}

function messageDateGroupLabel(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "Ohne Datum";
  const today = startOfDay(new Date());
  const day = startOfDay(date);
  const diffDays = Math.floor((today.getTime() - day.getTime()) / 86_400_000);
  if (diffDays <= 0) return "Heute";
  if (diffDays === 1) return "Gestern";
  if (diffDays < 7) return "Diese Woche";
  if (diffDays < 14) return "Letzte Woche";
  if (diffDays < 31) return "Dieser Monat";
  if (diffDays < 183) return "Letzte 6 Monate";
  return "Älter";
}

function startOfDay(date: Date) {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

function messageTimestamp(message: MessageSummary) {
  const time = new Date(message.date).getTime();
  return Number.isNaN(time) ? 0 : time;
}

function viewTitle(view: View) {
  const titles: Record<View, string> = {
    inbox: "Posteingang",
    starred: "Markiert",
    sent: "Gesendet",
    drafts: "Entwürfe",
    archive: "Archiv",
    spam: "Spam",
    trash: "Papierkorb",
    unread: "Ungelesen",
    attachments: "Mit Anhängen",
    today: "Heute",
    important: "Wichtig",
    newsletters: "Newsletter",
    rules: "Regeln",
    settings: "Einstellungen"
  };
  return titles[view];
}

function listFilterLabel(filter: ListFilter) {
  const labels: Record<ListFilter, string> = {
    all: "Alle",
    unread: "Ungelesen",
    flagged: "Wichtig",
    attachments: "Anhänge"
  };
  return labels[filter];
}

function listSortLabel(sortMode: MessageSortMode) {
  const labels: Record<MessageSortMode, string> = {
    newest: "Neueste",
    oldest: "Älteste",
    unread: "Ungelesen",
    sender: "Absender",
    subject: "Betreff",
    attachments: "Anhänge"
  };
  return labels[sortMode];
}

function accountFilterLabel(accountId: string, accounts: Account[]) {
  if (accountId === "all") return "Alle Konten";
  const account = accounts.find((item) => item.id === accountId);
  return account?.email ?? "Konto";
}

function accountSyncLabel(account: Account, statuses: SyncStatus[], syncingAccountId: string | null, isBackgroundSyncing: boolean) {
  if (syncingAccountId === account.id) return "Sync";
  if (isBackgroundSyncing) return "Delta";
  if (account.authStatus === "needs_reconnect") return "Neu verbinden";
  if (account.authStatus === "needs_password") return "Passwort";
  if (account.authStatus === "repairable" || account.credentialStatus === "session_only") return "Reparieren";
  const accountStatuses = statuses.filter((status) => status.accountId === account.id);
  if (accountStatuses.some((status) => status.lastError)) return "Hinweis";
  if (accountStatuses.some((status) => !status.backfillComplete)) return "Smart";
  return account.syncEnabled ? "Aktiv" : "Bereit";
}

function formatShortTime(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleTimeString("de-DE", { hour: "2-digit", minute: "2-digit" });
}

function formatMessageListDate(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  const today = new Date();
  if (date.toDateString() === today.toDateString()) {
    return date.toLocaleTimeString("de-DE", { hour: "2-digit", minute: "2-digit" });
  }
  return date.toLocaleDateString("de-DE", { day: "2-digit", month: "2-digit" });
}

function formatFullDateTime(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "Datum unbekannt";
  return date.toLocaleString("de-DE", {
    weekday: "short",
    day: "2-digit",
    month: "2-digit",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit"
  });
}

function formatFileSize(bytes: number) {
  if (!bytes || bytes < 0) return "Größe unbekannt";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function senderDomain(email?: string) {
  const domain = email?.split("@")[1]?.trim().toLowerCase() ?? "";
  return domain.replace(/[>),;]+$/g, "");
}

function senderAccent(domain: string) {
  const normalized = domain.toLowerCase();
  if (normalized.includes("spotify")) return "#1ed760";
  if (normalized.includes("youtube")) return "#ff416c";
  if (normalized.includes("google") || normalized.includes("gmail")) return "#5b8cff";
  if (normalized.includes("apple") || normalized.includes("icloud")) return "#d8dce5";
  if (normalized.includes("github")) return "#f5f7fb";
  if (normalized.includes("amazon")) return "#ffb84d";
  if (normalized.includes("dropbox")) return "#3984ff";
  return "#9b7bff";
}

function shortEmailLabel(email: string) {
  const [name, domain] = email.split("@");
  if (!domain) return email;
  const shortName = name.length > 15 ? `${name.slice(0, 13)}...` : name;
  return `${shortName}@${domain}`;
}

function providerLabel(account: Account) {
  const domain = senderDomain(account.email);
  if (account.provider === "gmail" || domain.includes("gmail")) return "Gmail";
  if (account.provider === "microsoft" || domain.includes("hotmail") || domain.includes("outlook") || domain.includes("live")) return "Outlook";
  if (domain.includes("icloud") || domain.includes("me.com") || domain.includes("mac.com")) return "iCloud";
  return account.provider === "imap" ? "IMAP" : account.provider;
}

function senderIconSources(domain: string) {
  if (!domain) return [];
  const baseDomain = domain
    .replace(/^mail\./, "")
    .replace(/^email\./, "")
    .replace(/^notifications\./, "")
    .replace(/^noreply\./, "");
  return [
    `https://logo.clearbit.com/${encodeURIComponent(baseDomain)}?size=256`,
    `https://www.google.com/s2/favicons?domain_url=https://${encodeURIComponent(baseDomain)}&sz=256`,
    `https://icons.duckduckgo.com/ip3/${encodeURIComponent(baseDomain)}.ico`
  ];
}

function renderLinkedText(text: string) {
  const parts = text.split(/(https?:\/\/[^\s]+)/gi);
  return parts.map((part, index) => {
    if (!/^https?:\/\//i.test(part)) return part;
    const href = part.replace(/[),.;]+$/g, "");
    return (
      <a href={href} target="_blank" rel="noreferrer" key={`${href}-${index}`}>
        {shortenLink(href)}
      </a>
    );
  });
}

function unsubscribeMethodLabel(method: UnsubscribeMethod) {
  switch (method) {
    case "http_get":
      return "Abmelde-Link";
    case "http_post":
      return "HTTP-Bestätigung";
    case "mailto":
      return "Abmelde-E-Mail";
    case "html_link":
      return "Link in E-Mail";
    default:
      return "Unbekannt";
  }
}
