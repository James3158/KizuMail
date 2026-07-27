# KizuMail AI Security

KizuMail treats every email, search candidate, user-supplied reply hint, and explicitly included attachment as untrusted data. AI output is advisory text only. AI code must never receive mail, filesystem, network, account, credential, or tool capabilities.

## Security Boundary

Use `src-tauri/src/ai/security.rs` as the only prompt-construction and output-validation boundary for summaries, quick replies, and Finder reranking.

- With the current `ai::models::GenerationRequest`, map `PromptRequest.system_prompt` to `instruction`, `PromptRequest.trusted_prompt` to `prompt`, and `PromptRequest.untrusted_data_json` to one `UntrustedText` entry. Never put `user_prompt` or `untrusted_data_json` into the trusted `prompt` field.
- For a provider API used without `GenerationRequest`, pass `system_prompt` as the system/developer message and `user_prompt` as a separate user message. Do not concatenate both roles.
- Send no tools, functions, mail commands, URL fetchers, or provider actions with the model request. `PromptRequest.allow_tools` is always `false` and must be enforced by the provider adapter.
- Treat the JSON between `<KIZUMAIL_UNTRUSTED_DATA_JSON>` boundaries as inert data. The serializer escapes `<`, `>`, and `&`, so mail text cannot close or create a trusted boundary.
- Validate every model response before display, caching, ranking, or reuse. Reject Markdown fences, unknown fields, unknown candidate IDs, duplicate IDs, non-integer scores, and oversized output.
- Never execute text from a summary or quick reply. A quick reply remains a draft that requires explicit user review and a separate send confirmation.

Prompt instructions reduce model risk but are not an authorization boundary. The actual boundary is the absence of tools and privileged APIs, strict account scoping, output validation, and explicit user confirmation.

## Prompt APIs

- `build_summary_prompt`: returns a request whose only accepted output is `summary`, `key_points`, and `action_items` JSON.
- `build_quick_reply_prompt`: returns a draft-only request whose only accepted output is `reply` JSON.
- `build_finder_rerank_prompt`: replaces database and account IDs with request-local aliases. `FinderPrompt::validate_output` rejects invented aliases and maps accepted aliases back to local candidate IDs.
- `validate_summary_output`, `validate_quick_reply_output`, and `FinderPrompt::validate_output`: enforce exact schemas and hard character/count limits.
- `redact_log_field`: mandatory for structured fields that may contain keys, tokens, prompts, model output, mail bodies, or attachment text.
- `redact_inline_secrets`: defense-in-depth for error strings. It does not replace structured redaction.

## Attachments

Attachments are excluded by default. Set `PromptSecurityOptions.include_attachments = true` only after the user explicitly enables attachment processing for the current operation.

- Include extracted text only, never raw binary data or local storage paths.
- Apply the same untrusted-data boundary, input cap, PII policy, and output validation as mail bodies.
- Do not reuse an attachment permission for another message, account, or background job.
- Virus/malware scanning and file-type validation happen before extraction and are separate from prompt safety.

## Cloud And Local Checklist

### Cloud inference

- [ ] Obtain an explicit user opt-in that identifies the provider and data categories sent.
- [ ] Default `minimize_pii` to enabled in product settings; show a clear override when full content is required.
- [ ] Apply `PromptSecurityOptions { target: Cloud, minimize_pii: true, ... }` before constructing the provider request.
- [ ] Confirm provider retention, training, regional processing, and deletion settings.
- [ ] Do not send account IDs, message IDs, credential references, local paths, OAuth data, raw headers, or unrelated thread/account content.
- [ ] Keep attachments disabled unless separately and explicitly approved.
- [ ] Set provider-side output/token caps in addition to local validators.
- [ ] Disable provider tools, browsing, remote URL retrieval, and function calling.

PII minimization in `security.rs` conservatively redacts email addresses and phone-like digit sequences. It is a minimizer, not a complete DLP system; cloud consent and data minimization remain mandatory.

### Local inference

- [ ] Bind only to loopback or use an in-process runtime; never expose the local model endpoint to the LAN.
- [ ] Send no telemetry or prompt traces by default.
- [ ] Apply the same prompt boundaries and output validators as cloud inference.
- [ ] Keep model files and caches in app-owned directories with normal user ACLs.
- [ ] Treat a local model as untrusted code/output: no tools, no shell, no filesystem, and no mail-provider credentials.

## Multi-Account Isolation

- [ ] Resolve and authorize `account_id` and `message_id` before constructing a prompt.
- [ ] Load source messages with an account-qualified query; never trust a frontend-supplied message ID by itself.
- [ ] Build one summary/quick-reply request from one account and one message only.
- [ ] For Finder, prefilter candidates by accounts the current UI context may access. The security builder aliases IDs but does not grant access.
- [ ] Scope caches by account ID, message revision/content hash, AI task, model, prompt version, PII mode, and attachment flag.
- [ ] Never expose account IDs to a model merely to preserve ranking identity; use the Finder aliases.
- [ ] Clear per-account AI jobs and cache entries when an account is removed or reauthenticated.

## Cache And Deletion

- [ ] AI results are local cache data, not canonical mail data.
- [ ] Provide per-message, per-account, and global AI-cache deletion.
- [ ] Delete cached prompts, outputs, embeddings, rerank results, and extracted attachment text together.
- [ ] Cancel queued/in-flight jobs before deleting an account and discard late responses using a job generation/revision check.
- [ ] Apply a retention limit; do not keep raw prompts for diagnostics.
- [ ] Never place credentials, raw authorization headers, or token responses in an AI cache key or value.

## Credentials And Logging

- [ ] Keep provider API keys and OAuth tokens in the OS Credential Manager, referenced by opaque IDs only.
- [ ] Read credentials only inside the provider adapter immediately before the request; never pass them through prompt structs.
- [ ] Never log `PromptRequest`, mail/attachment text, model raw output, authorization headers, API keys, or tokens.
- [ ] Use `redact_log_field` for sensitive structured fields and log only operation ID, task kind, model, duration, byte counts, status, and account-safe opaque correlation IDs.
- [ ] Run `redact_inline_secrets` on third-party/provider error text before logging or returning diagnostics.
- [ ] Rotate/revoke cloud credentials after suspected exposure and clear affected caches.

## Integration Points

The owning agents should wire the new module without moving security decisions into UI code:

1. AI module root: declare `pub mod security;` from the eventual `src-tauri/src/ai/mod.rs`.
2. Summary command/service: authorize account/message, create `MailPromptInput`, build with `build_summary_prompt`, call the selected provider with no tools, then run `validate_summary_output`.
3. Quick Reply command/service: use `build_quick_reply_prompt`, validate with `validate_quick_reply_output`, return a draft only, and keep SMTP send behind the existing explicit send path.
4. Finder/index service: prefilter authorized candidates, call `build_finder_rerank_prompt`, then use that same `FinderPrompt` instance to validate and resolve aliases.
5. Provider adapters: construct `GenerationRequest { instruction: request.system_prompt, prompt: request.trusted_prompt, untrusted_data: vec![UntrustedText::new("kizumail_security_json", request.untrusted_data_json)], ... }`; enforce timeout and provider token limits, no tool/function declarations, Credential Manager lookup, and redacted errors.
6. Cache owner: include account/revision/privacy/attachment dimensions and implement account/global deletion before enabling persistence.

## Required Security Tests

`security.rs` includes tests for delimiter escape attempts, system-prompt override text, cross-account requests, token exfiltration instructions, forbidden mail actions, attachment gating, cloud PII minimization, strict JSON schemas, output caps, Finder alias enforcement, and log redaction. Provider integration tests must additionally assert that the outgoing request contains no tool definitions and that rejected model output is never cached or shown as trusted data.
