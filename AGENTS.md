# AGENTS.md

Instructions for AI coding agents working in this repo. Claude Code reads this
through `CLAUDE.md` (which imports it). Codex, Cursor, Copilot, and other agents
read this file directly.

## What this is

`brief-agent-buildos26` is a voice-first VC meeting-prep agent (Rust / Axum,
single binary, no database). A Slack `/brief <fund>` command researches a fund,
its partners, and its portfolio with sourced citations, posts the brief to Slack,
then places an outbound Twilio call bridged to the OpenAI Realtime API to deliver
it by voice. Every claim is sourced, and a 3-tier verifier refuses to state
anything it cannot ground.

## Setup (run this when the user asks you to set up or run the project)

Outcome: a running server with a healthy `/health`, then the three webhooks wired
through an ngrok tunnel. Pause whenever you need a secret from the user.

1. Toolchain: confirm `cargo` 1.94+ (edition 2024) via rustup, plus `ngrok`. If
   either is missing, give the user the install command for their OS and wait.
2. Config: `cp .env.example .env`. Walk the user through every `APP__*` variable,
   saying what it is, whether it is required, and where to get it. The required
   set to boot and run one brief:
   - `APP__OPENAI__API_KEY`: platform.openai.com (reused for Realtime when
     `APP__OPENAI_REALTIME__API_KEY` is unset)
   - `APP__OPENAI_REALTIME__PROJECT_ID`, `APP__OPENAI_REALTIME__WEBHOOK_SECRET`:
     the OpenAI project id and the Realtime webhook signing secret
   - `APP__SLACK__BOT_TOKEN`, `APP__SLACK__SIGNING_SECRET`: a Slack app
     (api.slack.com/apps) with a `/brief` slash command
   - `APP__TWILIO__ACCOUNT_SID`, `APP__TWILIO__AUTH_TOKEN`,
     `APP__TWILIO__FROM_NUMBER`: Twilio (console.twilio.com), voice-capable number
   - `APP__DESTINATION_PHONE`: the E.164 number the agent calls
   - `APP__OPENAI_REALTIME__WEBHOOK_BASE_URL`: set to the ngrok HTTPS URL (step 6)
   Optional: `APP__TAVILY_API_KEY` (web search; DuckDuckGo is the fallback),
   `APP__LINEAR_API_KEY` (the `linear_query` tool), `APP__PORT` (default 8080),
   `APP__BRIEF_MODEL`.
3. Build: `cargo build`. Fix any toolchain errors before continuing.
4. Run: `cargo run`. It binds `APP__PORT` (the `.env.example` value is 8080) and
   logs `brief-agent listening` with the address.
5. Verify: `curl localhost:8080/health` returns `{"ok":true}`. Report the port and
   result to the user.
6. Expose and wire webhooks: start `ngrok http 8080`, take the HTTPS URL, set
   `APP__OPENAI_REALTIME__WEBHOOK_BASE_URL` to it, restart `cargo run`, then have
   the user point all three at the tunnel:
   - Slack `/brief` Request URL: `<ngrok>/slack/commands/brief`
   - OpenAI Realtime webhook: `<ngrok>/internal/voice/webhook/openai-realtime`
   - Twilio status callback: `<ngrok>/internal/voice/webhook/twilio/status`
7. Smoke test: in Slack run `/brief <fund name>`. Expect a brief in the channel,
   then an outbound call to `APP__DESTINATION_PHONE`.

## Commands

- Run: `cargo run`
- Lint: `cargo clippy`
- Release build: `cargo build --release`
- Health: `curl localhost:8080/health`
- Container: `docker build -t brief-agent:dev . && docker run --rm -p 3030:3030 brief-agent:dev`
  (the image sets `APP__PORT=3030`)

## Layout and conventions

- Single Axum binary. Routes are registered in `src/main.rs`.
- Config is environment-only: `APP__*`, where `__` denotes nesting, parsed in
  `src/config.rs`.
- HTTP handlers live in `src/handlers/`; external integrations (OpenAI Realtime,
  Twilio) live in `src/infrastructure/`.
- The six data tools are HTTP endpoints under `/tools/*` on the same binary.
- Never commit secrets. `.env` is gitignored; `.env.example` is the template.
  Keep the two in sync when adding config.
