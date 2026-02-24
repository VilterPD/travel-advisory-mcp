import { randomUUID } from "node:crypto";
import express from "express";
import { McpServer } from "@modelcontextprotocol/server/server/mcp.js";
import { StreamableHTTPServerTransport } from "@modelcontextprotocol/server/server/streamableHttp.js";
import { isInitializeRequest } from "@modelcontextprotocol/server/types.js";
import * as z from "zod/v4";

const APP_NAME = "TravelAdvisory MCP";
const PORT = toPort(process.env.PORT, 3000);
const DEFAULT_AA_BASE_URL = "https://www.auswaertiges-amt.de/opendata";
const AA_BASE_URL = (process.env.AA_BASE_URL || DEFAULT_AA_BASE_URL).replace(/\/+$/, "");
const FETCH_TIMEOUT_MS = 10_000;
const UPSTREAM_EMPTY_ERROR = "Upstream returned empty object; API may be disabled.";

type JsonObject = Record<string, unknown>;
type SessionEntry = { server: McpServer; transport: StreamableHTTPServerTransport };

const sessions = new Map<string, SessionEntry>();
const app = express();

app.use(express.json({ limit: "1mb" }));

app.get("/healthz", (_req, res) => {
  res.status(200).json({ ok: true });
});

app.post("/mcp", async (req, res) => {
  forceMcpAccept(req);
  const sessionId = getSessionId(req);

  try {
    if (sessionId) {
      const session = sessions.get(sessionId);
      if (!session) {
        return sendJsonRpcError(res, 404, -32001, `Session not found: ${sessionId}`);
      }
      await session.transport.handleRequest(req, res, req.body);
      return;
    }

    if (!isInitializeRequest(req.body)) {
      return sendJsonRpcError(res, 400, -32000, "Bad Request: No valid session ID provided");
    }

    const server = buildMcpServer();
    let activeSessionId: string | undefined;
    let transport!: StreamableHTTPServerTransport;

    transport = new StreamableHTTPServerTransport({
      sessionIdGenerator: () => randomUUID(),
      enableJsonResponse: true,
      onsessioninitialized: (id) => {
        activeSessionId = id;
        sessions.set(id, { server, transport });
      }
    });

    transport.onclose = () => {
      if (activeSessionId) {
        sessions.delete(activeSessionId);
      }
    };

    await server.connect(transport);
    await transport.handleRequest(req, res, req.body);
  } catch (error) {
    console.error("MCP POST error:", error);
    if (!res.headersSent) {
      sendJsonRpcError(res, 500, -32603, "Internal server error");
    }
  }
});

app.get("/mcp", async (req, res) => {
  await handleSessionRequest(req, res, "GET");
});

app.delete("/mcp", async (req, res) => {
  await handleSessionRequest(req, res, "DELETE");
});

app.listen(PORT, "0.0.0.0", () => {
  console.log(`${APP_NAME} listening on port ${PORT}`);
  console.log(`MCP endpoint: http://localhost:${PORT}/mcp`);
  console.log(`Health endpoint: http://localhost:${PORT}/healthz`);
  console.log(`AA_BASE_URL: ${AA_BASE_URL}`);
});

process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);

async function handleSessionRequest(req: express.Request, res: express.Response, method: "GET" | "DELETE"): Promise<void> {
  forceMcpAccept(req);

  const sessionId = getSessionId(req);
  if (!sessionId) {
    res.status(400).send("Invalid or missing session ID");
    return;
  }

  const session = sessions.get(sessionId);
  if (!session) {
    res.status(404).send("Session not found");
    return;
  }

  try {
    await session.transport.handleRequest(req, res);
  } catch (error) {
    console.error(`MCP ${method} error:`, error);
    if (!res.headersSent) {
      res.status(500).send("Internal server error");
    }
  }
}

function buildMcpServer(): McpServer {
  const server = new McpServer({
    name: "travel-advisory-mcp",
    version: "0.1.0"
  });

  server.registerTool(
    "traveladvisory.ping",
    {
      description: "Basic health ping for this MCP server.",
      inputSchema: {}
    },
    async () => toolSuccess({ ok: true, name: APP_NAME })
  );

  server.registerTool(
    "traveladvisory.list_warnings",
    {
      description: "List travel advisory metadata from the German Foreign Office OpenData API.",
      inputSchema: {
        country: z.string().trim().min(1).optional(),
        onlyWarnings: z.boolean().default(false),
        limit: z.number().int().min(1).max(250).default(50)
      }
    },
    async ({ country, onlyWarnings, limit }) => {
      try {
        const response = await fetchResponse("/travelwarning");
        const items = listItems(response)
          .filter((item) => countryMatches(item, country))
          .filter((item) => !onlyWarnings || isWarning(item.flags))
          .slice(0, limit);

        return toolSuccess({ items });
      } catch (error) {
        return toolError(errorMessage(error));
      }
    }
  );

  server.registerTool(
    "traveladvisory.get_warning",
    {
      description: "Fetch one advisory (including content) by contentId.",
      inputSchema: {
        contentId: z.union([z.number(), z.string()]),
        format: z.enum(["html", "text"]).default("text")
      }
    },
    async ({ contentId, format }) => {
      try {
        const id = parseContentId(contentId);
        if (id === undefined) {
          return toolError("Invalid contentId. Please provide a numeric value.");
        }

        const response = await fetchResponse(`/travelwarning/${id}`);
        const raw = detailItem(response, id);
        if (!raw) {
          throw new Error(`No advisory content found for contentId ${id}.`);
        }

        return toolSuccess(detailPayload(id, raw, format));
      } catch (error) {
        return toolError(errorMessage(error));
      }
    }
  );

  return server;
}

async function fetchResponse(path: string): Promise<JsonObject> {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), FETCH_TIMEOUT_MS);
  const url = `${AA_BASE_URL}${path.startsWith("/") ? path : `/${path}`}`;

  try {
    const response = await fetch(url, {
      method: "GET",
      headers: { accept: "application/json" },
      signal: controller.signal
    });

    const rawText = await response.text();
    if (!response.ok) {
      throw new Error(`Upstream request failed (${response.status} ${response.statusText}).`);
    }

    const data = rawText ? JSON.parse(rawText) : {};
    if (!isObject(data) || isEmptyObject(data)) {
      throw new Error(UPSTREAM_EMPTY_ERROR);
    }

    if (!isObject(data.response) || isEmptyObject(data.response)) {
      throw new Error("Upstream response missing `response` object.");
    }

    return data.response;
  } catch (error) {
    if (error instanceof Error && error.name === "AbortError") {
      throw new Error(`Upstream request timed out after ${FETCH_TIMEOUT_MS}ms.`);
    }
    if (error instanceof Error) {
      throw error;
    }
    throw new Error("Unknown upstream error.");
  } finally {
    clearTimeout(timeout);
  }
}

function listItems(response: JsonObject): Array<{
  contentId: number;
  countryName?: string;
  iso2?: string;
  iso3?: string;
  lastModified?: { ms: number; iso: string };
  effective?: { ms: number; iso: string };
  flags?: Record<string, boolean>;
  raw: unknown;
}> {
  const items = Object.entries(response)
    .filter(([key, value]) => /^\d+$/.test(key) && isObject(value))
    .map(([key, value]) => {
      const record = value as JsonObject;
      return {
        contentId: Number(key),
        countryName: toStringValue(record.countryName),
        iso2: toStringValue(record.countryCode),
        iso3: toStringValue(record.iso3CountryCode),
        lastModified: toTimestamp(record.lastModified),
        effective: toTimestamp(record.effective),
        flags: booleanFlags(record),
        raw: record
      };
    });

  if (items.length === 0) {
    throw new Error("Upstream returned no warning entries.");
  }

  return items;
}

function detailItem(response: JsonObject, contentId: number): JsonObject | undefined {
  const direct = response[String(contentId)];
  if (isObject(direct)) {
    return direct;
  }

  for (const [key, value] of Object.entries(response)) {
    if (/^\d+$/.test(key) && isObject(value)) {
      return value;
    }
  }

  return undefined;
}

function detailPayload(contentId: number, raw: JsonObject, format: "html" | "text") {
  const html =
    toStringValue(raw.contentHtml) ??
    (looksLikeHtml(raw.content) ? toStringValue(raw.content) : undefined);

  const plain =
    htmlToPlainText(html) ||
    textFromRaw(raw) ||
    "";

  const content: { text: string; html?: string } = { text: plain };
  if (format === "html" && html) {
    content.html = html;
  }

  return {
    contentId,
    title: toStringValue(raw.title),
    countryName: toStringValue(raw.countryName),
    iso2: toStringValue(raw.countryCode),
    iso3: toStringValue(raw.iso3CountryCode),
    lastModified: toTimestamp(raw.lastModified),
    effective: toTimestamp(raw.effective),
    content,
    raw
  };
}

function textFromRaw(raw: JsonObject): string | undefined {
  const content = toStringValue(raw.content);
  if (content && !looksLikeHtml(content)) {
    return normalizeText(content);
  }

  const candidates = [raw.text, raw.contentText, raw.contentPlain, raw.plainText, raw.lastChanges];
  for (const candidate of candidates) {
    const text = toStringValue(candidate);
    if (text) {
      return normalizeText(text);
    }
  }
  return undefined;
}

function countryMatches(
  item: { countryName?: string; iso2?: string; iso3?: string },
  country?: string
): boolean {
  const needle = country?.trim();
  if (!needle) {
    return true;
  }
  if (needle.length === 2) {
    return item.iso2?.toLowerCase() === needle.toLowerCase();
  }
  if (needle.length === 3) {
    return item.iso3?.toLowerCase() === needle.toLowerCase();
  }
  return (item.countryName || "").toLowerCase().includes(needle.toLowerCase());
}

function isWarning(flags?: Record<string, boolean>): boolean {
  if (!flags) {
    return false;
  }
  if (flags.warning || flags.partialWarning || flags.situationWarning) {
    return true;
  }
  return Object.entries(flags).some(([key, value]) => key.toLowerCase().includes("warning") && value);
}

function parseContentId(value: string | number): number | undefined {
  if (typeof value === "number") {
    return Number.isInteger(value) && value >= 0 ? value : undefined;
  }

  const trimmed = value.trim();
  if (!/^\d+$/.test(trimmed)) {
    return undefined;
  }
  return Number.parseInt(trimmed, 10);
}

function toTimestamp(value: unknown): { ms: number; iso: string } | undefined {
  const n =
    typeof value === "number"
      ? value
      : typeof value === "string" && value.trim() !== ""
        ? Number(value)
        : undefined;

  if (n === undefined || !Number.isFinite(n) || n <= 0) {
    return undefined;
  }

  const ms = n >= 1_000_000_000_000 ? Math.trunc(n) : Math.trunc(n * 1000);
  return { ms, iso: new Date(ms).toISOString() };
}

function booleanFlags(raw: JsonObject): Record<string, boolean> | undefined {
  const result: Record<string, boolean> = {};
  for (const [key, value] of Object.entries(raw)) {
    if (typeof value === "boolean") {
      result[key] = value;
    }
  }
  return Object.keys(result).length ? result : undefined;
}

function looksLikeHtml(value: unknown): boolean {
  return typeof value === "string" && /<[^>]+>/.test(value);
}

function htmlToPlainText(html?: string): string | undefined {
  if (!html) {
    return undefined;
  }

  const text = html
    .replace(/<script[\s\S]*?<\/script>/gi, " ")
    .replace(/<style[\s\S]*?<\/style>/gi, " ")
    .replace(/<[^>]+>/g, " ")
    .replace(/&amp;/g, "&")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, "\"")
    .replace(/&#39;/g, "'");

  return normalizeText(text);
}

function normalizeText(value: string): string {
  return value.replace(/\s+/g, " ").trim();
}

function toStringValue(value: unknown): string | undefined {
  if (typeof value !== "string") {
    return undefined;
  }
  const trimmed = value.trim();
  return trimmed || undefined;
}

function isObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isEmptyObject(value: unknown): boolean {
  return isObject(value) && Object.keys(value).length === 0;
}

function errorMessage(error: unknown): string {
  return error instanceof Error && error.message ? error.message : "Unknown tool error.";
}

function toolSuccess(payload: unknown) {
  return {
    content: [{ type: "text" as const, text: JSON.stringify(payload, null, 2) }],
    structuredContent: isObject(payload) ? payload : { data: payload }
  };
}

function toolError(message: string) {
  return {
    isError: true,
    content: [{ type: "text" as const, text: message }]
  };
}

function sendJsonRpcError(res: express.Response, status: number, code: number, message: string): void {
  res.status(status).json({
    jsonrpc: "2.0",
    error: { code, message },
    id: null
  });
}

function getSessionId(req: express.Request): string | undefined {
  const raw = req.headers["mcp-session-id"];
  if (typeof raw !== "string") {
    return undefined;
  }
  const trimmed = raw.trim();
  return trimmed || undefined;
}

function forceMcpAccept(req: express.Request): void {
  req.headers.accept = "application/json, text/event-stream";
}

function toPort(raw: string | undefined, fallback: number): number {
  const parsed = Number.parseInt(raw || "", 10);
  if (!Number.isInteger(parsed) || parsed <= 0 || parsed > 65535) {
    return fallback;
  }
  return parsed;
}

function shutdown(): void {
  for (const session of sessions.values()) {
    void session.server.close();
  }
  sessions.clear();
}
