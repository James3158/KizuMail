import type { Account, AuditEvent, MessageSummary, NewsletterSubscription } from "../types";

export const accounts: Account[] = [
  {
    id: "acct-1",
    provider: "imap",
    email: "alex@example.com",
    displayName: "Personal IMAP",
    authType: "password",
    syncEnabled: true,
    lastSyncAt: "2026-07-05T19:35:00.000Z"
  },
  {
    id: "acct-2",
    provider: "microsoft",
    email: "alex@outlook.com",
    displayName: "Outlook",
    authType: "oauth",
    syncEnabled: false
  }
];

export const messages: MessageSummary[] = [
  {
    id: "msg-1",
    accountId: "acct-1",
    sender: "Linear",
    senderEmail: "updates@linear.app",
    subject: "Weekly workspace digest",
    preview: "Your issue activity, project movement, and team updates from the last week.",
    bodyText: "Your issue activity, project movement, and team updates from the last week.",
    date: "2026-07-05T08:12:00.000Z",
    folder: "Inbox",
    unread: true,
    flagged: false,
    hasAttachments: false,
    attachments: [],
    hasNewsletterSignals: true
  },
  {
    id: "msg-2",
    accountId: "acct-1",
    sender: "GitHub",
    senderEmail: "notifications@github.com",
    subject: "Security alert: dependency review",
    preview: "A dependency in one repository requires review. This is not classified as a newsletter.",
    bodyText: "A dependency in one repository requires review. This is not classified as a newsletter.",
    date: "2026-07-04T20:05:00.000Z",
    folder: "Inbox",
    unread: false,
    flagged: false,
    hasAttachments: false,
    attachments: [],
    hasNewsletterSignals: false
  },
  {
    id: "msg-3",
    accountId: "acct-1",
    sender: "Shop Daily",
    senderEmail: "deals@shopdaily.example",
    subject: "Final hours: summer offers",
    preview: "Use your personal link before midnight. Includes unsubscribe headers and marketing patterns.",
    bodyText: "Use your personal link before midnight. Includes unsubscribe headers and marketing patterns.",
    date: "2026-07-04T16:42:00.000Z",
    folder: "Promotions",
    unread: true,
    flagged: false,
    hasAttachments: false,
    attachments: [],
    hasNewsletterSignals: true
  }
];

export const newsletters: NewsletterSubscription[] = [
  {
    id: "sub-1",
    sender: "Linear",
    senderEmail: "updates@linear.app",
    domain: "linear.app",
    messageCount: 18,
    lastMessageAt: "2026-07-05T08:12:00.000Z",
    unsubscribeMethod: "http_post",
    confidenceScore: 94,
    riskScore: 8,
    riskLevel: "low",
    evidence: ["List-Unsubscribe", "List-Unsubscribe-Post", "stable sender domain"]
  },
  {
    id: "sub-2",
    sender: "Shop Daily",
    senderEmail: "deals@shopdaily.example",
    domain: "shopdaily.example",
    messageCount: 47,
    lastMessageAt: "2026-07-04T16:42:00.000Z",
    unsubscribeMethod: "html_link",
    confidenceScore: 72,
    riskScore: 61,
    riskLevel: "medium",
    evidence: ["repeated marketing sender", "unsubscribe text link", "tracking parameters"]
  }
];

export const auditEvents: AuditEvent[] = [
  {
    id: "audit-1",
    action: "sync.preview",
    entity: "acct-1",
    summary: "Loaded local account and mock folder state.",
    createdAt: "2026-07-05T19:35:00.000Z"
  },
  {
    id: "audit-2",
    action: "newsletter.scan",
    entity: "acct-1",
    summary: "Detected 2 candidate newsletter subscriptions.",
    createdAt: "2026-07-05T19:36:00.000Z"
  }
];
