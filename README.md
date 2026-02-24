# TravelAdvisory MCP

This repository is a minimal feasibility prototype and proof of concept for exposing German Foreign Office travel advisories over MCP.
Implementation was created with support from an inference model.
See [docs/llm-governance.md](docs/llm-governance.md) for governance details.

## Purpose

This server gives MCP clients direct access to official travel-advisory data.
It is useful when you want to answer questions like:

- "Are there warnings for country X?"
- "Do we have a partial warning or a full warning?"
- "Give me the latest official advisory text for a specific country entry."

## Available Tools

### `traveladvisory.ping`

What it does:
- Health check for the MCP server itself.

When to use:
- Verify server availability before calling data tools.

### `traveladvisory.list_warnings`

What it does:
- Lists advisory metadata across countries.
- Supports filtering by country and warning-only mode.

Key options:
- `country` (name, ISO2, or ISO3)
- `onlyWarnings` (only entries with warning flags)
- `limit` (max number of entries)

Returns:
- Country identifiers, timestamps, warning flags, and `contentId`.

### `traveladvisory.get_warning`

What it does:
- Fetches one concrete advisory by `contentId`.

Key options:
- `contentId`
- `format` (`text` or `html`)

Returns:
- Advisory metadata plus full advisory content.

## Data Source

- Source: German Foreign Office OpenData API (`auswaertiges-amt.de/opendata`)
- Upstream endpoints used:
  - `/travelwarning`
  - `/travelwarning/{contentId}`

## Run and Setup

### Local Run

```bash
npm install
npm run build
npm start
```

Dev mode:

```bash
npm run dev
```

Default URLs:

- MCP: `http://localhost:3000/mcp`
- Health: `http://localhost:3000/healthz`

### Environment Variables

- `PORT` (default: `3000`)
- `AA_BASE_URL` (default: `https://www.auswaertiges-amt.de/opendata`)

### Docker

Build:

```bash
docker build -t travel-advisory-mcp .
```

Run:

```bash
docker run --rm -p 3000:3000 \
  -e PORT=3000 \
  -e AA_BASE_URL=https://www.auswaertiges-amt.de/opendata \
  travel-advisory-mcp
```

## Error Behavior

- Upstream fetch timeout: `10s`
- If upstream returns `{}`: tool returns explicit MCP error
- Invalid tool input (for example bad `contentId`): explicit MCP error
