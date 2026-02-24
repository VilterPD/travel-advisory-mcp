# AGENTS.md

## Purpose
- This repo is a very small PoC MCP server for German Foreign Office travel advisories.

## Scope
- Keep changes small, explicit, and easy to review.
- Preserve MCP tool contracts unless a change is explicitly requested.

## Simplicity First
- Prefer the simplest implementation that keeps the current feature set intact.
- Prioritize clear request flow over abstraction-heavy structure.
- Remove non-essential code/noise when behavior stays the same.
- Favor straightforward helpers and naming over clever patterns.

## Commands
- Build: `cargo build`
- Run: `cargo run`
- Format: `cargo fmt`
- Lint: `cargo clippy`

## Commit Messages
- Follow the 50/72 guideline:
- Keep the subject line at or below 50 characters.
- Wrap the body at around 72 characters per line.

## Architecture Notes
- Main server entrypoint: `src/main.rs`
- MCP endpoint: `/mcp`
- Health endpoint: `/healthz`
- Upstream source: `https://www.auswaertiges-amt.de/opendata`

## Guardrails
- Do not invent advisory data; only return upstream-backed results.
- Fail with clear errors when upstream data is missing or invalid.
- Keep dependencies minimal unless there is a strong reason.
- Keep `Cargo.lock` in sync with `Cargo.toml`.

## Definition of Done
- Build passes.
- README/docs updated if behavior changes.
- Changes remain consistent with this PoC scope.
