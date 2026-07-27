# Security Policy

KizuMail handles email content and credentials locally, so security reports are taken seriously.

## Reporting a vulnerability

Please do not open a public issue for a suspected vulnerability, exposed credential, authentication bypass, cross-account data leak, unsafe mail rendering problem, or destructive mail action.

Use GitHub's private vulnerability reporting or Security Advisory feature for this repository. Include:

- the affected KizuMail version or commit;
- the operating system;
- clear reproduction steps;
- the expected and actual behavior;
- the potential impact;
- logs or screenshots only after removing personal mail content, addresses, tokens, and credentials.

Do not include working account passwords, OAuth refresh tokens, API keys, private client configuration, or real mailbox exports.

## Current security boundaries

- Credentials and AI keys belong in the operating-system credential store.
- Mail bodies, attachments, indexes, and sync state belong in local app data.
- The source repository must contain placeholders only.
- Cloud AI remains optional and action-driven.
- AI output is advisory and must not directly execute mail actions.
- Destructive mail and unsubscribe actions require an explicit user action.

KizuMail is currently an alpha project and has not received an independent professional security audit.
