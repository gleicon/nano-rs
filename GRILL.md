# GRILL: Google Apps Script polyfill for nano-rs

## Resolved decisions

### 1. Delivery mechanism
**Q:** New runtime mode (Rust-side bindings) or pure JS shim?
**A:** Pure JS shim — no new Rust.
**Rationale:** nano-rs already exposes `fetch()`, `TextEncoder/Decoder`, `crypto`, `console`, `nano:kv`. A single `.js` file loaded before the user's script provides full GAS globals without touching the runtime.


### 2. Target use case
**Q:** Who are the users and what's the pattern?
**A:** Non-tech users who glue AI to spreadsheets / data pipelines / LLM document processing.
**Rationale:** Core loop: read rows from Sheets → call LLM via UrlFetchApp → write back results. Real data, real Google APIs required.

### 3. SpreadsheetApp data source
**Q:** In-memory mock, VFS-backed, or live Google API?
**A:** Live bridge (Option C) — shim calls Google Sheets/Docs REST APIs via fetch().
**Rationale:** Exploration project; mock data is useless for the actual use case. User confirmed willing to handle Google client library/auth complexity.


### 4. Google credentials scope
**Q:** Service account key per-tenant or global?
**A:** Always per-tenant — each app's env_vars holds its own service account JSON.
**Rationale:** Global credentials would let all tenants access each other's authorized sheets. Non-negotiable for multi-tenant.

### 5. SpreadsheetApp API surface
**Q:** getActiveSpreadsheet() only, or also openById()?
**A:** Both. getActiveSpreadsheet() reads Nano.env.SPREADSHEET_ID. openById(id) works for any sheet the service account can access.
**Rationale:** Non-tech users set env var once; power users call openById() per request.


### 6. Entry point dispatch
**Q:** How does nano-rs call into a GAS script with no fetch() handler?
**A:** X-then-Y-then-main: shim provides fetch(request) that tries doGet/doPost first (GAS web app convention), then {"function":"name"} JSON body dispatch, then main() fallback.
**Rationale:** Covers web app scripts unchanged, time-trigger scripts via explicit invocation, and simple scripts with a main entry point. No user code changes required for web apps.


### 7. Write-back strategy
**Q:** Immediate Google Sheets API writes or batch-at-end?
**A:** Batch-at-end. Collect all setValues/setValue/appendRow calls, flush with single Sheets batchUpdate on handler return. SpreadsheetApp.flush() forces early flush.
**Rationale:** GAS itself batches internally. "Iterate 100 rows → write result" pattern hits Sheets rate limits (100 req/100s per user) with immediate writes. Batch stays within limits.

### 8. API scope for v1
**Q:** Which GAS services ship in v1?
**A:** SpreadsheetApp (read/write values, getDataRange, appendRow, getSheetByName), DocumentApp (read-only — getBody().getText()), DriveApp (getFileById, getFolderById, list files — read/write basic ops), UrlFetchApp, Logger, PropertiesService (nano:kv-backed), Utilities (base64/JSON/sleep), CacheService (nano:kv + manual TTL).
**Stubs that throw clear error:** GmailApp, CalendarApp, ScriptApp, HtmlService, UI dialogs.
**Rationale:** Covers the "read Docs/Sheets/Drive → call LLM → write back" pipeline end-to-end. All three Google APIs use same OAuth token, same fetch() pattern, marginal extra cost. Hard cutoff at email/calendar/UI.


### 9. Loading mechanism and binary consistency
**Q:** How is the shim loaded — VFS file, auto-prepend, or nano: built-in module?
**A:** nano:gas built-in module — same pattern as nano:kv. JS shim embedded as pub const GAS_SHIM_CODE: &str = r#"..."# in src/runtime/gas.rs. User writes: import 'nano:gas'; at top of script.
**Rationale:** Fully consistent with existing nano:kv pattern. Ships in single binary (no VFS file, no external dep). Rust surface: one new file, one mod declaration, one match arm in get_nano_module_code(). All runtime behavior (OAuth2 JWT, REST API calls, dispatch) is pure JS using existing fetch(), crypto.subtle, nano:kv.

### 10. OAuth2 implementation
**Q:** Bundle google-auth-library or implement JWT flow in JS?
**A:** Pure JS JWT implementation using crypto.subtle (RSASSA-PKCS1-v1_5). Token cached in nano:kv per-tenant with 1-hour TTL checked manually. No external library.
**Rationale:** google-auth-library uses Node.js APIs not available in nano-rs. crypto.subtle with RSA already works (shipped in v2.2.2). Keeps single-binary constraint.

## Summary

The GAS compatibility shim is:
- A nano:gas ESM module (JS embedded in Rust const, same as nano:kv)
- Pure JS: uses fetch(), crypto.subtle, nano:kv, console
- Live bridge to Google APIs via service account JWT (per-tenant, from Nano.env)
- SpreadsheetApp + DocumentApp + DriveApp + UrlFetchApp + Logger + PropertiesService + Utilities + CacheService
- Entry point dispatch: doGet/doPost → function-name JSON body → main() fallback
- Writes batched, flushed on handler return (or via SpreadsheetApp.flush())
- Stubs for GmailApp, CalendarApp, ScriptApp, HtmlService that throw descriptive errors

Rust delta: ~10 lines across 3 files. JS shim: ~500 lines.
