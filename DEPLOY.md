# Deploy

## Local

```sh
docker build -t brief-agent:dev .
docker run --rm -p 3030:3030 brief-agent:dev
# health: curl localhost:3030/health
```

## AKS migration path (~30 min, post-hackathon)

This repo lives on GitHub (CEO setup) while OWI production runs Azure DevOps Pipelines → Docker Hub → AKS via kustomize (see `owilabs/audit-agent`). The migration is mechanical — we mirror the audit-agent orchestrator deploy:

1. **AzDO ↔ GitHub service connection.** In OWI's Azure DevOps project, add a GitHub service connection pointing at `owilabs-cto/brief-agent-buildOS26`. Reuse your existing Kubernetes (`<your-aks-service-connection>`) and Docker registry (`<your-dockerhub-service-connection>`) connections.
2. **Copy + adapt the pipeline.** Start from `audit-agent/azure-pipelines-orchestrator.yml`. Drop the `Run Tests` job (no Postgres dependency here), drop the path filters (`orchestrator/**`, `shared/**`), and adjust the Docker build to the single-package layout — `docker buildx build --file Dockerfile .` instead of `--file orchestrator/Dockerfile`.
3. **Copy + rename the k8s manifests.** Start from `audit-agent/k8s/orchestrator/{base,overlays/production}/`. Global rename `<orchestrator-app-name> → brief-agent`, change `containerPort: 8080 → 3030`, update probe paths from `/api/health → /health`, and trim `overlays/production/patches/env.yaml` to only the env vars this binary actually reads. Target namespace stays `public-ingress` (same as audit-agent).
4. **Registry.** Push to Docker Hub `<your-dockerhub-org>/brief-agent`. Not ACR (this stack does not use ACR despite the Azure-everywhere naming).
5. **Verify.** Push to `main`, watch the pipeline build + push + `kubectl apply -k`. Check pod readiness with `kubectl -n public-ingress get pods -l app=brief-agent`.

## What's intentionally not in this repo

- **No `azure-pipelines.yml`** — committing a pipeline that references the Docker registry and AKS service connections without those connections being wired to this GitHub repo would silently fail every push to main. The migration is short enough that pre-writing the YAML buys nothing.
- **No `k8s/` overlays** — the `env.yaml` patch in audit-agent's production overlay references 25+ secret keys (`elevenlabs-*`, `openai-*`, `azure-*`, `twilio-*`) that don't apply here. A hand-trimmed copy without those refs is fiction until the binary actually reads its own config.
- **No ADR for the GitHub-vs-AzDO split.** The decision is captured in this file; a separate ADR would just restate it.

The Dockerfile is the only production-shape artifact we commit here. It mirrors `audit-agent/orchestrator/Dockerfile` (multi-stage, `rust:1.94-slim-bookworm` builder, `debian:bookworm-slim` runtime, non-root `app` user, HEALTHCHECK) so step 2 of the migration is a no-op.
