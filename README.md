# KizuMail

KizuMail is an open-source, local-first desktop email client with a modern interface, multi-account synchronization, newsletter tools, and optional AI features.

The application is in early alpha development and currently supports Windows only. It should not yet be treated as a fully audited replacement for mature production clients.

## Platform status

- **Windows:** alpha. Current builds are not code-signed.
- **macOS:** not supported yet; planned. Credential-store integration and packaging are currently Windows-specific.

## AI-assisted development

KizuMail was developed with AI assistance. The project creator defined the concept, made architecture and product decisions, tested the application, and reviewed the implementation. Most code was developed and iterated with OpenAI Codex using GPT-5.5 and GPT-5.6.

AI assistance does not replace testing, review, or maintenance. KizuMail has not received an independent external security audit. Contributions that improve correctness, portability, documentation, and security are welcome.

## Current features

- Multiple Gmail, Microsoft/Outlook, iCloud, and generic IMAP accounts
- OAuth login for Gmail and Microsoft
- IMAP folder discovery, delta synchronization, history backfill, and background synchronization
- Local SQLite mail cache with account isolation
- Inbox, sent, drafts, archive, spam, trash, flagged, unread, attachment, and date-based views
- SMTP sending, reply, forward, archive, delete, spam, read, and flag actions
- Sanitized KizuMail readable view for HTML and text email
- Embedded images, local attachment caching, and attachment access
- Newsletter detection and preview-first unsubscribe actions
- Desktop notifications, autostart, and background tray operation
- Optional AI summaries, quick replies, and semantic mail search
- Gemini cloud AI or a local Ollama-compatible model
- Dark and light themes

## Privacy and security model

KizuMail has no hosted KizuMail backend and no built-in telemetry pipeline. Mail synchronization happens directly between the desktop application and the configured mail provider.

- Mail data and search indexes remain in the local application data directory.
- Passwords, OAuth tokens, and AI API keys are stored through the operating-system credential store.
- Credentials are not stored in the SQLite mail database.
- Cloud AI is optional and disabled unless the user configures and enables it.
- AI actions require an explicit user action; generated replies remain drafts until the user sends them.
- Local OAuth configuration, API keys, databases, logs, attachments, and build credentials are excluded from Git.

See [SECURITY.md](SECURITY.md) and [docs/AI_SECURITY.md](docs/AI_SECURITY.md) for the current security boundaries and reporting process.

## Downloads

Windows builds are published on the [GitHub Releases page](https://github.com/James3158/KizuMail/releases).

- `KizuMail_*_x64-setup.exe`: recommended installer for normal Windows use
- `KizuMail_*_x64_en-US.msi`: MSI package for managed or manual installation
- `kizumail-portable.exe`: portable executable for testing

The current builds are not code-signed. Windows SmartScreen may therefore show an unknown-publisher warning. Verify that downloads come from this repository's Releases page.

No personal OAuth client secret, account password, token, mail database, or AI key is included in public releases. Users must provide their own provider configuration where required.

## Provider configuration

KizuMail supports provider client IDs entered through the app settings. For local development, copy the public template:

```powershell
Copy-Item .\oauth-clients.example.json .\oauth-clients.local.json
```

Then replace only the placeholders needed for your own provider registrations. Never commit or publicly upload `oauth-clients.local.json`.

iCloud and some generic IMAP providers may require an app-specific password. Keep that password private and enter it only inside the application.

## Windows development

Requirements:

- Node.js and npm
- Rust and Cargo
- Microsoft Visual Studio Build Tools with the MSVC C++ toolchain
- Windows SDK
- WebView2 Runtime

Install JavaScript dependencies:

```powershell
npm install
```

Run the frontend:

```powershell
npm run dev
```

Run the desktop application:

```powershell
npm run tauri dev
```

Run the frontend build and UI tests:

```powershell
npm run build
npm run test:ui
```

Run the Rust/Tauri check with the configured Windows toolchain:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\check-tauri.ps1
```

Create local Windows installers and clean temporary build output:

```powershell
powershell -ExecutionPolicy Bypass -File .\build-package-clean.ps1
```

This local packaging script intentionally uses `oauth-clients.local.json` when it exists. Do not publish locally generated artifacts unless you have verified that they contain no private provider configuration. Official public releases are built with placeholder configuration only.

## macOS status

The repository can be cloned on macOS for continued development:

```bash
git clone https://github.com/James3158/KizuMail.git
cd KizuMail
npm install
```

macOS support is planned, but there is currently no supported macOS release. The Rust credential-store integration and packaging configuration are still Windows-specific, so macOS support requires a dedicated portability pass before the Tauri app can be built and distributed as a signed `.app` or `.dmg`.

An Intel Mac targets `x86_64-apple-darwin`; Apple Silicon targets `aarch64-apple-darwin`.

## Project structure

- `src/`: React and TypeScript interface
- `src-tauri/`: Rust backend, local database, mail synchronization, OAuth, SMTP, and desktop integration
- `src-tauri/src/ai/`: optional cloud and local AI providers, indexing, search, and safety boundaries
- `tests/`: Playwright UI coverage
- `docs/`: architecture, distribution, and AI security documentation
- `scripts/`: local checks, builds, cleanup, and uninstallation helpers

## Contributing

Bug reports and focused pull requests are welcome. Please avoid including real email data, screenshots containing personal information, OAuth credentials, API keys, tokens, or local databases.

For security issues, follow [SECURITY.md](SECURITY.md) instead of opening a public issue.

## License

KizuMail is released under the [MIT License](LICENSE).
