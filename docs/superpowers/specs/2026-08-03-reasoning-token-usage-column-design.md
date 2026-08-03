# Reasoning Token Usage Column Design

## Goal

Show reasoning-token usage for each request in the usage dashboard without changing existing token totals or cost calculations.

## Scope

- Add a reasoning-token column to the request log table.
- Show the same value in the request detail panel.
- Capture reasoning tokens from supported proxy responses and Codex session logs.
- Persist and return the value through the existing usage query path.
- Add the user-facing label to every translation catalog.

Provider statistics, model statistics, trend charts, hero totals, and pricing behavior are out of scope.

## Data Model

Add `reasoning_tokens` as a non-negative integer to `proxy_request_logs`. The schema version advances from 16 to 17, and the v16-to-v17 migration adds the column with `NOT NULL DEFAULT 0`. Existing rows therefore remain valid and display zero when no historical reasoning value was stored.

The Rust `TokenUsage`, request-log persistence model, usage query result, and TypeScript `RequestLog` type expose the field as `reasoning_tokens` or `reasoningTokens`, following the naming convention at each boundary.

Reasoning tokens are a subset of output tokens. They must not be added to output tokens, total tokens, or costs a second time.

## Collection

The proxy usage parser reads the provider-specific reasoning field when present:

- OpenAI Responses and Codex: `usage.output_tokens_details.reasoning_tokens`.
- OpenAI-compatible Chat Completions: `usage.completion_tokens_details.reasoning_tokens`.
- Gemini: `usageMetadata.thoughtsTokenCount`.
- Anthropic-compatible payloads: a supported reasoning or thinking-token detail field when supplied by the upstream response.

Missing, malformed, negative, or unsupported values resolve to zero, matching the existing token parser behavior.

Codex session import extends its cumulative counters and delta calculation with `reasoning_output_tokens`. Rebuilding Codex usage through the existing maintenance action reparses historical session files and can recover their reasoning-token values. Historical proxy rows cannot be reconstructed and remain zero.

## Persistence And Queries

All request-log insert paths write `reasoning_tokens`, including proxy logging and Codex session import. Request-log list and detail queries select and map the column. Existing rollup aggregation remains unchanged because aggregate reasoning displays are outside this feature's scope.

The field does not change request deduplication, pricing, rollups, or retention behavior.

## User Interface

Insert a centered, numeric `Reasoning Tokens` column immediately after `Output Tokens` in the request log table. Values use the same locale-aware integer formatting as the other token columns. Increase the empty-state table `colSpan` to match the new column count.

Add a `Reasoning Tokens` row to the token-usage section of the request detail panel. A value of zero is displayed consistently with input and output token values.

Add `usage.reasoningTokens` to every locale under `src/i18n/locales/`; no user-facing fallback string is relied upon.

## Error Handling And Compatibility

The migration is additive and preserves all existing data. Providers that do not report reasoning tokens continue to produce valid records with zero. Parsers use saturating or checked numeric conversion consistent with existing token handling, so an optional detail field cannot prevent the rest of a request's usage from being recorded.

## Verification

- Parser tests cover Responses, Chat Completions, Gemini, missing fields, and malformed values.
- Database migration tests verify v16-to-v17 column creation and the zero default.
- Logger and usage-query tests verify persistence and list/detail mapping.
- Codex session tests verify cumulative reasoning counters become per-event deltas and survive rebuild import.
- Frontend tests verify the request-log header/value, empty-state column span, and request-detail value.
- Run frontend type checking and focused Vitest tests, plus Rust formatting and focused Rust tests.
