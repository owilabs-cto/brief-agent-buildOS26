# VC Prep Intelligence Agent — 2026-05-14 hackathon

Voice-first VC meeting prep that compresses the 50–100h of fund/partner/portfolio research per raise into minutes, with every sentence sourced (LAW #0) and a 3-tier confidence verifier preventing fabrication.

Built for the Telus Most Trustworthy Agentic System cash prize, 2026-05-14.

## Setup with Claude Code

Clone the repo, open it in [Claude Code](https://claude.com/claude-code) (or any agent that reads `AGENTS.md`: Codex, Cursor, Copilot, and others), and paste:

> Set this project up and run it end to end, following AGENTS.md. Walk me through each API key I need and where to get it, then start it and confirm /health is green.

The agent reads [`AGENTS.md`](AGENTS.md) and drives the rest: toolchain check, `.env` scaffolding, build, run, health check, and wiring the Slack, OpenAI Realtime, and Twilio webhooks through ngrok. You only supply the keys when asked.

## Architecture (Slack-first pivot)

- **`/brief`** (Rust/Axum) — Responses-API orchestrator. `gpt-5.5`, `reasoning.effort=medium`, 7-LAWS system prompt, parallel function-calling across the 6 data tools, structured-output JSON `{brief, phone_brief, drill_down_facts, audit_trail, do_not_claim, warnings}`.
- **`/slack/commands/brief`** — Slack slash command receiver. HMAC-verified. Calls `/brief`, posts to Slack (main message + threaded audit trail), then places an outbound Twilio call bridged to OpenAI Realtime SIP with the prepared brief baked into the session instructions.
- **`/internal/voice/webhook/openai-realtime`** — receives `realtime.call.incoming` from OpenAI, joins back to the prepared brief via the SIP `x-audit-call-id` header, calls `/accept` with vc_brief instructions + `end_call` tool.
- **`/internal/voice/webhook/twilio/status`** — receives Twilio status callbacks (`initiated` / `ringing` / `answered` / `completed`), updates the Slack message footer through Ringing → Connected → Call ended.
- **6 data tools** (same binary):
  - `search_gmail` — recency-flagged threads (>90d = historical)
  - `web_search` — fund / partner / portfolio queries (Tavily + DDG fallback)
  - `web_fetch` — URL drill-down
  - `linear_query` — scoped to a configured allowlist of Linear projects
  - `local_docs_search` — primary-source local corpus
  - `verify_claim` — 3-tier confidence verifier (Tier 1 / 2 / 3)
- **`/session`** — legacy WebRTC ephemeral-key endpoint. Retained but unused after the Slack pivot.

## Demo runtime (laptop + ngrok)

```sh
ngrok http --url <reserved-domain>.ngrok-free.dev 8080
APP__PORT=8080 cargo run
```

Wire the Slack manifest's `/brief` Request URL + OpenAI Realtime webhook + Twilio outbound status callback to the ngrok HTTPS URL. See `.env.example` for the full env-var list.

## Charter — 7 Guarantees

LAW #0 (sourcing absolute) + 6 mitigations against the failure modes the agent must refuse to commit:
1. No statement without `verify_claim` returning `grounded` and `sources[]` non-empty.
2-6. The remaining five mitigations against those failure modes.

## Run

```sh
cargo run
# server on http://localhost:8080  (APP__PORT, default 8080)
curl localhost:8080/health   # → {"ok":true}
```
