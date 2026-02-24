# LLM Governance

This document defines the current governance baseline for this repository.

## Current Architecture Note

The deployed `travel-advisory-mcp` server does not call an LLM at runtime.
It is an MCP tool server that proxies and normalizes data from the German Foreign Office OpenData API.

## 1. Scope

- Project: `travel-advisory-mcp`
- Owner: `Alexander Sattelmaier`
- Effective from: `2026-02-24`
- Environments: `staging`

## 2. Release Baseline

| Field | Value | Source |
|---|---|---|
| Model provider | `OpenAI` (development assistant context) | Session/runtime context |
| Model ID | `gpt-5` (Codex runtime) | Session/runtime context |
| Model snapshot/date | `2026-02-24` | Session/runtime context |
| App version | `N/A (set per release)` | `git rev-parse --short HEAD` |
| Prompt ID | `N/A (no runtime prompt in this server)` | Architecture |
| Prompt version | `N/A` | Architecture |
| Tool version | `0.1.0` (`src/main.rs`) | Source code |

## 3. Runtime Settings

| Parameter | Value |
|---|---|
| `temperature` | `N/A (no runtime LLM inference)` |
| `top_p` | `N/A (no runtime LLM inference)` |
| `max_output_tokens` | `N/A (no runtime LLM inference)` |
| `seed` | `N/A (no runtime LLM inference)` |
| `reasoning_effort` | `N/A (no runtime LLM inference)` |
| `response_format` | `N/A (no runtime LLM inference)` |
| `tool_choice` | `N/A (no runtime LLM inference)` |
| `timeout_ms` | `10000` (upstream fetch timeout) |

## 4. Prompt Artifacts

| Prompt | Version | File | Hash | Owner | Change |
|---|---|---|---|---|---|
| `N/A` | `N/A` | `N/A` | `N/A` | `Alexander Sattelmaier` | `No runtime prompt artifacts in current architecture` |

## 5. Run Log Schema (current baseline)

```json
{
  "timestamp_utc": "2026-02-24T20:15:00Z",
  "environment": "staging",
  "app_git_sha": "<set per release>",
  "model": {
    "provider": null,
    "id": null,
    "note": "no runtime LLM inference in current architecture"
  },
  "settings": null,
  "prompt": null,
  "tools": [
    {
      "name": "traveladvisory.ping",
      "duration_ms": 25,
      "status": "ok"
    },
    {
      "name": "traveladvisory.list_warnings",
      "duration_ms": 210,
      "status": "ok"
    }
  ],
  "result": {
    "status": "ok",
    "latency_ms": 245,
    "tokens_input": null,
    "tokens_output": null
  }
}
```

Privacy: Do not log unnecessary plaintext personal data; use hashes/IDs where possible.

## 6. Eval Protocol

| Eval suite | Dataset | Metric | Target | Actual | Result |
|---|---|---|---|---|---|
| `mcp-tool-smoke` | `manual MCP calls` | `success rate` | `100%` | `100%` | `pass` |
| `error-path-smoke` | `manual negative tests` | `expected MCP error response` | `100%` | `not executed yet` | `pending` |
| `latency-baseline` | `prod-like sample` | `p95 ms` | `<= 1000` | `not measured yet` | `pending` |

Note: Model-quality evals are not applicable until a runtime LLM is introduced.

## 7. Risk and Guardrail Documentation

- Allowed use: Fetching and summarizing official travel-advisory content via MCP tools.
- Disallowed use: Treating results as legal advice or emergency-response instructions.
- Known weaknesses: Upstream API availability, changing upstream schema, stale cache risk if clients persist data.
- Escalation/review required when: repeated upstream failures, schema drift, or major tool contract changes.
- Fallback behavior: Return explicit MCP tool errors and avoid silent data fabrication.

## 8. Change Management

Any change to model, prompt, or critical settings requires:

1. Update this document (`Release Baseline`, `Runtime Settings`, `Prompt Artifacts`).
2. Run and record relevant evals.
3. Approval by owner/reviewer.
4. Changelog entry with date and ticket/PR reference.

### Changelog

| Date | Change | Type | PR/Ticket | Approval |
|---|---|---|---|---|
| `2026-02-24` | Initial governance baseline created and filled | `docs/governance` | `N/A` | `Alexander Sattelmaier` |

## 9. Minimal Pre-Prod Checklist

- [x] Model and prompt status are documented (`N/A` for runtime in current architecture).
- [x] Runtime settings are documented (including `N/A` where applicable).
- [ ] Full eval suite has been executed and documented.
- [ ] Logging/tracing with stable run metadata is fully implemented.
- [ ] Log privacy controls are verified in operations.
- [x] Fallback behavior and incident trigger conditions are defined.
