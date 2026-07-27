# KizuMail Architecture

KizuMail is a local-first desktop client. The UI never talks directly to mail providers, the filesystem, or secret storage. React calls Tauri commands, and Rust owns privileged local operations.

## Runtime Boundaries

- React UI: presentation, local UI state, user confirmation flows.
- Tauri commands: typed IPC boundary for accounts, messages, search, newsletters, rules, and audit.
- Rust core: provider adapters, IMAP/SMTP/OAuth orchestration, database access, secret references, HTML/mail safety checks.
- SQLite: local cache, sync state, FTS search, audit log.
- OS credential store: OAuth refresh tokens and IMAP/SMTP passwords.

## MVP Command Surface

- `accounts_list`: returns account metadata without secrets.
- `messages_list`: returns local message summaries.
- `newsletter_list`: returns detected newsletter subscriptions with confidence and risk.
- `audit_list`: returns local audit events for critical actions.

Future commands should stay preview-first for destructive or networked actions. For example, unsubscribe execution should be split into preview/dry-run and confirmed execution.

## Security Defaults

- No credentials in SQLite.
- No telemetry.
- No automatic external content in mail bodies.
- No blind unsubscribe.
- All destructive batch actions require a preview and explicit confirmation.
- Audit entries must not include full mail bodies, passwords, OAuth tokens, or raw auth headers.

## Provider Strategy

The first real provider target should be generic IMAP/SMTP because it proves the sync engine without OAuth verification delays. Microsoft OAuth is the first OAuth target. Gmail should stay behind a POC/advanced flag until app verification and scope requirements are clear.
