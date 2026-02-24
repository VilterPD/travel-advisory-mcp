use std::{collections::BTreeMap, env, net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    extract::Request,
    http::{header, HeaderValue, Method},
    middleware::{self, Next},
    response::Response,
    routing::get,
    Json as AxumJson, Router,
};
use rmcp::{
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::ServerInfo,
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    Json as McpJson, ServerHandler,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const APP_NAME: &str = "TravelAdvisory MCP";
const DEFAULT_AA_BASE_URL: &str = "https://www.auswaertiges-amt.de/opendata";
const FETCH_TIMEOUT_MS: u64 = 10_000;
const UPSTREAM_EMPTY_ERROR: &str = "Upstream returned empty object; API may be disabled.";
const MCP_POST_ACCEPT: &str = "application/json, text/event-stream";
const MCP_GET_ACCEPT: &str = "text/event-stream";
const JSON_CONTENT_TYPE: &str = "application/json";

#[derive(Clone)]
struct TravelAdvisoryService {
    tool_router: ToolRouter<Self>,
    http: reqwest::Client,
    aa_base_url: String,
}

impl TravelAdvisoryService {
    fn new(http: reqwest::Client, aa_base_url: String) -> Self {
        Self {
            tool_router: Self::tool_router(),
            http,
            aa_base_url,
        }
    }

    async fn fetch_response_object(&self, path: &str) -> Result<Map<String, Value>, String> {
        let url = format!(
            "{}/{}",
            self.aa_base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        );

        let response = self
            .http
            .get(url)
            .header("accept", "application/json")
            .send()
            .await
            .map_err(|err| format!("Upstream request failed: {err}"))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|err| format!("Failed to read upstream body: {err}"))?;

        if !status.is_success() {
            return Err(format!(
                "Upstream request failed ({} {}).",
                status.as_u16(),
                status.canonical_reason().unwrap_or("Unknown")
            ));
        }

        let data: Value = if body.trim().is_empty() {
            Value::Object(Map::new())
        } else {
            serde_json::from_str(&body)
                .map_err(|_| "Upstream returned invalid JSON.".to_string())?
        };

        if data.as_object().is_some_and(|obj| obj.is_empty()) {
            return Err(UPSTREAM_EMPTY_ERROR.to_string());
        }

        let response = data
            .get("response")
            .and_then(Value::as_object)
            .ok_or_else(|| "Upstream response missing `response` object.".to_string())?;

        if response.is_empty() {
            return Err(UPSTREAM_EMPTY_ERROR.to_string());
        }

        Ok(response.clone())
    }
}

#[tool_router]
impl TravelAdvisoryService {
    #[tool(
        name = "traveladvisory.ping",
        description = "Basic health ping for this MCP server."
    )]
    async fn ping(&self) -> McpJson<PingResponse> {
        McpJson(PingResponse {
            ok: true,
            name: APP_NAME.to_string(),
        })
    }

    #[tool(
        name = "traveladvisory.list_warnings",
        description = "List travel advisory metadata from the German Foreign Office OpenData API."
    )]
    async fn list_warnings(
        &self,
        Parameters(input): Parameters<ListWarningsInput>,
    ) -> Result<McpJson<ListWarningsResponse>, String> {
        let only_warnings = input.only_warnings.unwrap_or(false);
        let limit = input.limit.unwrap_or(50).clamp(1, 250);

        let response = self.fetch_response_object("/travelwarning").await?;
        let mut items = Vec::new();

        for (key, raw) in response {
            if !key.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }

            let raw_obj = match raw.as_object() {
                Some(value) => value,
                None => continue,
            };

            let item = ListWarningItem {
                content_id: key.parse::<u64>().unwrap_or_default(),
                country_name: string_field(raw_obj.get("countryName")),
                iso2: string_field(raw_obj.get("countryCode")),
                iso3: string_field(raw_obj.get("iso3CountryCode")),
                last_modified: timestamp_field(raw_obj.get("lastModified")),
                effective: timestamp_field(raw_obj.get("effective")),
                flags: bool_flags(raw_obj),
                raw: Value::Object(raw_obj.clone()),
            };

            if !matches_country(&item, input.country.as_deref()) {
                continue;
            }

            if only_warnings && !is_warning(item.flags.as_ref()) {
                continue;
            }

            items.push(item);
            if items.len() >= limit {
                break;
            }
        }

        if items.is_empty() {
            return Err("Upstream returned no warning entries.".to_string());
        }

        Ok(McpJson(ListWarningsResponse { items }))
    }

    #[tool(
        name = "traveladvisory.get_warning",
        description = "Fetch one advisory (including content) by contentId."
    )]
    async fn get_warning(
        &self,
        Parameters(input): Parameters<GetWarningInput>,
    ) -> Result<McpJson<GetWarningResponse>, String> {
        let content_id = parse_content_id(&input.content_id)
            .ok_or_else(|| "Invalid contentId. Please provide a numeric value.".to_string())?;

        let format = input.format.unwrap_or(ContentFormat::Text);
        let path = format!("/travelwarning/{content_id}");
        let response = self.fetch_response_object(&path).await?;

        let raw_obj = response
            .get(&content_id.to_string())
            .and_then(Value::as_object)
            .cloned()
            .or_else(|| {
                response
                    .iter()
                    .find_map(|(key, value)| {
                        key.chars().all(|c| c.is_ascii_digit()).then_some(value)
                    })
                    .and_then(Value::as_object)
                    .cloned()
            })
            .ok_or_else(|| format!("No advisory content found for contentId {content_id}."))?;

        let html_candidate = string_field(raw_obj.get("contentHtml")).or_else(|| {
            let content = string_field(raw_obj.get("content"))?;
            looks_like_html(&content).then_some(content)
        });

        let text = html_candidate
            .as_deref()
            .map(html_to_text)
            .or_else(|| text_from_fields(&raw_obj))
            .unwrap_or_default();

        let content = DetailContent {
            text,
            html: matches!(format, ContentFormat::Html)
                .then_some(html_candidate)
                .flatten(),
        };

        Ok(McpJson(GetWarningResponse {
            content_id,
            title: string_field(raw_obj.get("title")),
            country_name: string_field(raw_obj.get("countryName")),
            iso2: string_field(raw_obj.get("countryCode")),
            iso3: string_field(raw_obj.get("iso3CountryCode")),
            last_modified: timestamp_field(raw_obj.get("lastModified")),
            effective: timestamp_field(raw_obj.get("effective")),
            content,
            raw: Value::Object(raw_obj),
        }))
    }
}

#[tool_handler]
impl ServerHandler for TravelAdvisoryService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("Travel advisory MCP service".to_string()),
            ..Default::default()
        }
    }
}

#[derive(Debug, Serialize, Clone, JsonSchema)]
struct HealthResponse {
    ok: bool,
}

#[derive(Debug, Serialize, Clone, JsonSchema)]
struct PingResponse {
    ok: bool,
    name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ListWarningsInput {
    country: Option<String>,
    only_warnings: Option<bool>,
    limit: Option<usize>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct ListWarningsResponse {
    items: Vec<ListWarningItem>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ListWarningItem {
    content_id: u64,
    country_name: Option<String>,
    iso2: Option<String>,
    iso3: Option<String>,
    last_modified: Option<TimestampInfo>,
    effective: Option<TimestampInfo>,
    flags: Option<BTreeMap<String, bool>>,
    raw: Value,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct GetWarningInput {
    content_id: Value,
    format: Option<ContentFormat>,
}

#[derive(Debug, Deserialize, JsonSchema, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum ContentFormat {
    Html,
    Text,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct GetWarningResponse {
    content_id: u64,
    title: Option<String>,
    country_name: Option<String>,
    iso2: Option<String>,
    iso3: Option<String>,
    last_modified: Option<TimestampInfo>,
    effective: Option<TimestampInfo>,
    content: DetailContent,
    raw: Value,
}

#[derive(Debug, Serialize, JsonSchema)]
struct DetailContent {
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    html: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct TimestampInfo {
    ms: i64,
    iso: String,
}

fn string_field(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

fn timestamp_field(value: Option<&Value>) -> Option<TimestampInfo> {
    let raw = value?;
    let number = match raw {
        Value::Number(num) => num.as_f64(),
        Value::String(s) if !s.trim().is_empty() => s.trim().parse::<f64>().ok(),
        _ => None,
    }?;

    if !number.is_finite() || number <= 0.0 {
        return None;
    }

    let ms = if number >= 1_000_000_000_000.0 {
        number.trunc() as i64
    } else {
        (number * 1000.0).trunc() as i64
    };

    let iso = chrono_like_iso(ms)?;
    Some(TimestampInfo { ms, iso })
}

fn chrono_like_iso(ms: i64) -> Option<String> {
    let seconds = ms.div_euclid(1000);
    let nanos = (ms.rem_euclid(1000) as u32) * 1_000_000;
    let dt = std::time::UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds as u64))?
        .checked_add(Duration::from_nanos(nanos as u64))?;
    let datetime: chrono::DateTime<chrono::Utc> = dt.into();
    Some(datetime.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

fn bool_flags(raw: &Map<String, Value>) -> Option<BTreeMap<String, bool>> {
    let mut flags = BTreeMap::new();
    for (key, value) in raw {
        if let Value::Bool(v) = value {
            flags.insert(key.clone(), *v);
        }
    }
    (!flags.is_empty()).then_some(flags)
}

fn matches_country(item: &ListWarningItem, country: Option<&str>) -> bool {
    let Some(needle) = country.map(str::trim).filter(|v| !v.is_empty()) else {
        return true;
    };

    if needle.len() == 2 {
        return item
            .iso2
            .as_deref()
            .is_some_and(|iso| iso.eq_ignore_ascii_case(needle));
    }

    if needle.len() == 3 {
        return item
            .iso3
            .as_deref()
            .is_some_and(|iso| iso.eq_ignore_ascii_case(needle));
    }

    item.country_name
        .as_deref()
        .unwrap_or_default()
        .to_lowercase()
        .contains(&needle.to_lowercase())
}

fn is_warning(flags: Option<&BTreeMap<String, bool>>) -> bool {
    let Some(flags) = flags else {
        return false;
    };

    if flags.get("warning") == Some(&true)
        || flags.get("partialWarning") == Some(&true)
        || flags.get("situationWarning") == Some(&true)
    {
        return true;
    }

    flags
        .iter()
        .any(|(key, value)| *value && key.to_lowercase().contains("warning"))
}

fn parse_content_id(value: &Value) -> Option<u64> {
    match value {
        Value::Number(num) => num.as_u64(),
        Value::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()))
                .then(|| trimmed.parse::<u64>().ok())
                .flatten()
        }
        _ => None,
    }
}

fn looks_like_html(value: &str) -> bool {
    value.contains('<') && value.contains('>')
}

fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;

    for ch in html.chars() {
        match ch {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }

    normalize_text(&decode_entities(&out))
}

fn text_from_fields(raw: &Map<String, Value>) -> Option<String> {
    let content = string_field(raw.get("content"));
    if let Some(content) = content {
        if !looks_like_html(&content) {
            return Some(normalize_text(&decode_entities(&content)));
        }
    }

    for key in [
        "text",
        "contentText",
        "contentPlain",
        "plainText",
        "lastChanges",
    ] {
        if let Some(text) = string_field(raw.get(key)) {
            return Some(normalize_text(&decode_entities(&text)));
        }
    }

    None
}

fn decode_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

fn normalize_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_port(raw: Option<String>) -> u16 {
    raw.and_then(|v| v.parse::<u16>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(3000)
}

fn is_mcp_path(path: &str) -> bool {
    path == "/mcp" || path.starts_with("/mcp/")
}

async fn normalize_mcp_headers(mut req: Request, next: Next) -> Response {
    if is_mcp_path(req.uri().path()) {
        if req.method() == Method::POST {
            req.headers_mut()
                .insert(header::ACCEPT, HeaderValue::from_static(MCP_POST_ACCEPT));
            if !req.headers().contains_key(header::CONTENT_TYPE) {
                req.headers_mut().insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static(JSON_CONTENT_TYPE),
                );
            }
        } else if req.method() == Method::GET {
            req.headers_mut()
                .insert(header::ACCEPT, HeaderValue::from_static(MCP_GET_ACCEPT));
        }
    }

    next.run(req).await
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port = parse_port(env::var("PORT").ok());
    let aa_base_url = env::var("AA_BASE_URL").unwrap_or_else(|_| DEFAULT_AA_BASE_URL.to_string());
    let aa_base_url = aa_base_url.trim_end_matches('/').to_string();

    let http = reqwest::Client::builder()
        .timeout(Duration::from_millis(FETCH_TIMEOUT_MS))
        .build()?;

    let service_prototype = TravelAdvisoryService::new(http, aa_base_url.clone());
    let streamable_config = StreamableHttpServerConfig {
        stateful_mode: false,
        ..Default::default()
    };

    let mcp_service = StreamableHttpService::new(
        move || Ok(service_prototype.clone()),
        Arc::new(LocalSessionManager::default()),
        streamable_config,
    );

    let app = Router::new()
        .route(
            "/healthz",
            get(|| async { AxumJson(HealthResponse { ok: true }) }),
        )
        .nest_service("/mcp", mcp_service)
        .layer(middleware::from_fn(normalize_mcp_headers));

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    println!("{APP_NAME} listening on port {port}");
    println!("MCP endpoint: http://localhost:{port}/mcp");
    println!("Health endpoint: http://localhost:{port}/healthz");
    println!("AA_BASE_URL: {aa_base_url}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
