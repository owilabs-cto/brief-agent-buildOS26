# VC Prep Intelligence Agent — 2026-05-14 hackathon

Voice-first VC meeting prep that compresses the 50–100h of fund/partner/portfolio research per raise into minutes, with every sentence sourced (LAW #0) and a 3-tier confidence verifier preventing fabrication.

Built for the Telus Most Trustworthy Agentic System cash prize, 2026-05-14.

## Architecture

- **OpenAI Realtime** (WebRTC) — conversational layer, browser mic → agent
- **5 data tools** (Rust/Axum, port 3030):
  - `search_gmail` — recency-flagged threads (>90d = historical)
  - `web_search` — fund / partner / portfolio queries
  - `web_fetch` — URL drill-down
  - `linear_query` — scoped to Multi-provider voice agent + PLAN-002 projects only
  - `local_docs_search` — primary-source local corpus
- **3-tier verifier** (`verify_claim`) — gates every sentence; LAW #0 means *no source → no statement*
  - Tier 1: press / SEC primary source
  - Tier 2: inferred from portfolio or local_docs primary source
  - Tier 3: estimated with explicit uncertainty
- **`/session`** — mints OpenAI Realtime ephemeral keys

## Charter — 7 Guarantees

LAW #0 (sourcing absolute) + 6 mitigations against the failure modes the agent must refuse to commit:
1. No statement without `verify_claim` returning `grounded` and `sources[]` non-empty.
2. — 6. (mitigations enumerated in OWI-106 / OWI-107)

## Linear

- Project: [HACKATHON 2026-05-14 · VC Prep Intelligence Agent](https://linear.app/owilabs/project/hackathon-2026-05-14-vc-prep-intelligence-agent)
- Issues: OWI-101 … OWI-110

## Run

```sh
cargo run
# server on http://localhost:3030
curl localhost:3030/health   # → {"ok":true}
```
