// ═══════════════════════════════════════════════════════════════════════
// Read-only project-user presence feed (task 378f645a).
//
// Consumes herm-core's cross-tenant presence feed (GET /v1/agent-presence)
// so the Herm parent sidebar can OBSERVE project-user agents (slyce, brodie,
// jono, dublab, salon94, ...) that run in their OWN herdr session/socket.
//
// Read-only contract: the sidebar renders these rows distinctly and treats
// any row with NO local pane id as non-interactive — no focus / send-text /
// kill / pane actions. Project-user isolation is preserved: we never touch
// their socket, never dual-register them under the herm tenant, and never
// expose a pane id / control token back to the parent.
// ═══════════════════════════════════════════════════════════════════════

use serde::Deserialize;
use std::sync::OnceLock;
use std::time::Duration;

static PRESENCE_API_TOKEN: OnceLock<String> = OnceLock::new();

/// Capture the service credential before worker threads or child processes start.
/// Keep it in-process rather than leaving it in the environment inherited by children.
pub(crate) fn initialize_api_token() {
    let token = std::env::var("HERM_CORE_API_TOKEN").unwrap_or_default();
    std::env::remove_var("HERM_CORE_API_TOKEN");
    let _ = PRESENCE_API_TOKEN.set(token);
}

/// One row from herm-core `GET /v1/agent-presence` → `{ agents: [...] }`.
/// We only deserialize the read-only surface; we deliberately IGNORE any
/// `pane_id` / control field present on the wire (the sidebar never needs it
/// and must never expose it).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, serde::Serialize)]
pub struct PresenceRow {
    pub agent_id: String,
    /// Stable native session identity from herm-core; safe read-only metadata.
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub lane: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub effective_status: Option<String>,
    #[serde(default)]
    pub last_status: Option<String>,
    #[serde(default)]
    pub process_alive: Option<bool>,
    #[serde(default)]
    pub stream_alive: Option<bool>,
    #[serde(default)]
    pub last_seen_ts: Option<String>,
    #[serde(default)]
    pub lifecycle: Option<String>,
    #[serde(default)]
    pub idle: Option<bool>,
    #[serde(default)]
    pub session_memo: Option<String>,
    /// Structured heartbeat runtime/context are intentionally opaque. The
    /// sidebar only derives explicitly named display values from them.
    #[serde(default)]
    pub runtime: Option<serde_json::Value>,
    #[serde(default)]
    pub context: Option<serde_json::Value>,
    // Deliberately NOT deserialized: pane_id / control token.
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, serde::Serialize)]
struct PresenceEnvelope {
    #[serde(default)]
    agents: Vec<PresenceRow>,
}

/// A projected, read-only remote agent ready for the sidebar.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RemoteAgent {
    pub agent_id: String,
    #[serde(skip_serializing)]
    pub session_id: Option<String>,
    pub host: Option<String>,
    pub project: String,
    pub lane: String,
    pub status: String,
    pub user: String,
    pub cwd: Option<String>,
    pub process_alive: bool,
    pub stream_alive: bool,
    pub last_seen_ts: Option<String>,
    pub lifecycle: Option<String>,
    pub idle: Option<bool>,
    pub session_memo: Option<String>,
    pub context_usage: Option<String>,
}

fn context_usage(row: &PresenceRow) -> Option<String> {
    let objects = [row.context.as_ref(), row.runtime.as_ref()];
    for object in objects.into_iter().flatten() {
        let Some(object) = object.as_object() else {
            continue;
        };
        if let (Some(used), Some(limit)) = (
            object
                .get("context_used")
                .and_then(serde_json::Value::as_u64),
            object
                .get("context_limit")
                .and_then(serde_json::Value::as_u64),
        ) {
            if let Some(percent) = used.saturating_mul(100).checked_div(limit) {
                return Some(format!("{percent}%"));
            }
        }
        for key in ["context_percent", "percent"] {
            if let Some(value) = object.get(key) {
                if let Some(value) = value.as_u64() {
                    return Some(format!("{value}%"));
                }
                if let Some(value) = value.as_str().filter(|value| !value.is_empty()) {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

impl PresenceRow {
    /// True when the agent is currently live (process alive or lifecycle
    /// active). Used to filter stale rows from the default sidebar view.
    pub fn is_live(&self) -> bool {
        self.process_alive == Some(true) || self.lifecycle.as_deref() == Some("active")
    }
}

/// Derive a lane from an agent id.
///
/// Prefer splitting on `-{project}-` so single-token hosts keep the nickname
/// (`sebluair-herm-groovy-16be` → `groovy-16be`) and hyphenated hosts still
/// work (`herm-b-slyce-perky-e9fb` → `perky-e9fb`). Falls back to the
/// host-token heuristic, then the raw id.
pub fn derive_lane(agent_id: &str, project: Option<&str>) -> String {
    if let Some(project) = project.filter(|project| !project.is_empty()) {
        let marker = format!("-{project}-");
        if let Some(index) = agent_id.find(&marker) {
            let rest = &agent_id[index + marker.len()..];
            if !rest.is_empty() {
                return rest.to_string();
            }
        }
    }
    let parts: Vec<&str> = agent_id.split('-').collect();
    if parts.len() >= 4 {
        return parts[3..].join("-");
    }
    if parts.len() >= 2 {
        return parts[1..].join("-");
    }
    agent_id.to_string()
}

/// The OS user running a project-user agent: the project slug for non-herm
/// projects, `herm` for herm agents. Falls back to `herm` when unknown.
pub fn derive_user(project: Option<&str>, agent_id: &str) -> String {
    match project {
        Some(p) if p != "herm" => p.to_string(),
        Some("herm") | Some(_) => "herm".to_string(),
        None => {
            let parts: Vec<&str> = agent_id.split('-').collect();
            if parts.len() >= 3 {
                parts[parts.len() - 2].to_string()
            } else {
                "herm".to_string()
            }
        }
    }
}

impl RemoteAgent {
    pub fn from_row(row: PresenceRow) -> Self {
        let project = row.project.clone().unwrap_or_else(|| "herm".to_string());
        let derived = derive_lane(&row.agent_id, row.project.as_deref());
        let lane = match row.lane.as_deref().filter(|lane| !lane.is_empty()) {
            Some(api_lane) if derived == api_lane || derived.ends_with(&format!("-{api_lane}")) => {
                derived
            }
            Some(api_lane) => api_lane.to_string(),
            None => derived,
        };
        let status = row
            .effective_status
            .clone()
            .or_else(|| row.last_status.clone())
            .unwrap_or_else(|| "idle".to_string());
        let context_usage = context_usage(&row);
        RemoteAgent {
            user: row
                .user
                .filter(|user| !user.is_empty())
                .unwrap_or_else(|| derive_user(row.project.as_deref(), &row.agent_id)),
            host: row.host,
            agent_id: row.agent_id,
            session_id: row.session_id,
            project,
            lane,
            status,
            cwd: row.cwd,
            process_alive: row.process_alive.unwrap_or(false),
            stream_alive: row.stream_alive.unwrap_or(false),
            last_seen_ts: row.last_seen_ts,
            lifecycle: row.lifecycle,
            idle: row.idle,
            session_memo: row.session_memo,
            context_usage,
        }
    }
}

/// Parse the JSON body of `GET /v1/agent-presence` into projected remote
/// agents. Non-live (stale/stopped) rows are dropped by default unless
/// `include_recent` is set. Pure + deterministic — unit-testable.
pub fn parse_presence_body(body: &str, include_recent: bool) -> Result<Vec<RemoteAgent>, String> {
    let envelope: PresenceEnvelope =
        serde_json::from_str(body).map_err(|e| format!("bad presence JSON: {e}"))?;
    let mut out: Vec<RemoteAgent> = Vec::with_capacity(envelope.agents.len());
    for row in envelope.agents {
        if !include_recent && !row.is_live() {
            continue;
        }
        out.push(RemoteAgent::from_row(row));
    }
    out.sort_by(|a, b| {
        a.project
            .cmp(&b.project)
            .then_with(|| a.lane.cmp(&b.lane))
            .then_with(|| a.agent_id.cmp(&b.agent_id))
    });
    Ok(out)
}

/// herm-core on the VPS is loopback. Laptop brndr has no :8787 — use the
/// tailnet HTTPS API so presence remotes show up off herm-b.
fn presence_api_base() -> String {
    if let Ok(base) = std::env::var("HERM_CORE_BASE") {
        if !base.is_empty() {
            return base;
        }
    }
    if std::path::Path::new("/opt/herm/env/herm-core.env").exists() {
        return "http://127.0.0.1:8787".into();
    }
    let host =
        std::env::var("HERM_TAILNET_HOST").unwrap_or_else(|_| "herm-b.tail94725b.ts.net".into());
    format!("https://{host}:8787")
}

/// Fetch the cross-tenant presence feed from herm-core.
///
/// Reads `HERM_CORE_BASE` when set. Off herm-b that defaults to the tailnet
/// HTTPS API; on the VPS it stays loopback. `HERM_CORE_API_TOKEN` authenticates.
/// Failures degrade to the last known rows.
pub fn fetch_presence(timeout: Duration) -> Result<Vec<RemoteAgent>, String> {
    let include_recent = std::env::var("HERDR_PRESENCE_INCLUDE_RECENT")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false);
    fetch_presence_from_base(&presence_api_base(), include_recent, timeout)
}

fn fetch_presence_from_base(
    base: &str,
    include_recent: bool,
    timeout: Duration,
) -> Result<Vec<RemoteAgent>, String> {
    let token = PRESENCE_API_TOKEN
        .get()
        .map(String::as_str)
        .unwrap_or_default();
    let agent = ureq::AgentBuilder::new().timeout(timeout).build();

    let mut req = agent.get(&format!("{base}/v1/agent-presence"));
    if !token.is_empty() {
        req = req.set("X-API-Token", token);
    }

    let body = req
        .call()
        .map_err(|e| format!("presence fetch failed: {e}"))?;
    let body = body
        .into_string()
        .map_err(|e| format!("presence read failed: {e}"))?;
    parse_presence_body(&body, include_recent)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(agent_id: &str, project: &str, alive: bool) -> PresenceRow {
        PresenceRow {
            agent_id: agent_id.into(),
            session_id: None,
            project: Some(project.into()),
            host: Some("herm-b".into()),
            lane: None,
            cwd: Some("/home/project".into()),
            user: None,
            effective_status: Some("idle".into()),
            last_status: Some("idle".into()),
            process_alive: Some(alive),
            stream_alive: Some(alive),
            last_seen_ts: Some("2026-08-18T20:00:00Z".into()),
            lifecycle: if alive {
                Some("active".into())
            } else {
                Some("detached_recent".into())
            },
            idle: Some(!alive),
            session_memo: None,
            runtime: None,
            context: None,
        }
    }

    #[test]
    fn fetch_presence_uses_the_captured_token() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::time::Duration;

        assert!(PRESENCE_API_TOKEN.set("fixture-token".into()).is_ok());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, request_rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut byte = [0; 1];
            while !request.ends_with(b"\r\n\r\n") {
                let count = stream.read(&mut byte).unwrap();
                assert_ne!(count, 0, "client closed before sending HTTP headers");
                request.push(byte[0]);
                assert!(request.len() < 8192, "unexpectedly large HTTP header");
            }
            request_tx
                .send(String::from_utf8(request).unwrap())
                .unwrap();
            let body = br#"{"agents":[{"agent_id":"herm-b-slyce-perky-e9fb","project":"slyce","host":"herm-b","effective_status":"working","process_alive":true,"stream_alive":true,"lifecycle":"active","idle":false}]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
        });

        let agents =
            fetch_presence_from_base(&format!("http://{address}"), false, Duration::from_secs(2))
                .unwrap();
        let request = request_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        server.join().unwrap();
        assert!(request
            .to_ascii_lowercase()
            .contains("x-api-token: fixture-token"));
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].agent_id, "herm-b-slyce-perky-e9fb");
    }

    #[test]
    fn initialize_api_token_keeps_credential_out_of_child_environment() {
        const MODE: &str = "HERDR_TEST_PRESENCE_TOKEN_MODE";
        match std::env::var(MODE).as_deref() {
            Ok("verify") => {
                assert!(std::env::var_os("HERM_CORE_API_TOKEN").is_none());
            }
            Ok("capture") => {
                assert_eq!(std::env::var("HERM_CORE_API_TOKEN").unwrap(), "test-token");
                initialize_api_token();
                assert_eq!(
                    PRESENCE_API_TOKEN.get().map(String::as_str),
                    Some("test-token")
                );
                assert!(std::env::var_os("HERM_CORE_API_TOKEN").is_none());
                let status = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "presence::tests::initialize_api_token_keeps_credential_out_of_child_environment",
                        "--test-threads=1",
                    ])
                    .env(MODE, "verify")
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .unwrap();
                assert!(status.success());
            }
            _ => {
                let status = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "presence::tests::initialize_api_token_keeps_credential_out_of_child_environment",
                        "--test-threads=1",
                    ])
                    .env(MODE, "capture")
                    .env("HERM_CORE_API_TOKEN", "test-token")
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .unwrap();
                assert!(status.success());
            }
        }
    }

    #[test]
    fn parse_presence_body_filters_stale_unless_recent() {
        let live = row("herm-b-slyce-perky-e9fb", "slyce", true);
        let stale = row("herm-b-slyce-jazzy-3701", "slyce", false);
        let body = serde_json::to_string(&PresenceEnvelope {
            agents: vec![stale.clone(), live.clone()],
        })
        .unwrap();

        let live_only = parse_presence_body(&body, false).unwrap();
        assert_eq!(live_only.len(), 1);
        assert_eq!(live_only[0].agent_id, "herm-b-slyce-perky-e9fb");

        let with_recent = parse_presence_body(&body, true).unwrap();
        assert_eq!(with_recent.len(), 2);
    }

    #[test]
    fn remote_agent_never_exposes_pane_id() {
        let row = PresenceRow {
            agent_id: "herm-b-slyce-perky-e9fb".into(),
            session_id: None,
            project: Some("slyce".into()),
            host: Some("herm-b".into()),
            lane: None,
            cwd: None,
            user: None,
            effective_status: Some("working".into()),
            last_status: None,
            process_alive: Some(true),
            stream_alive: Some(true),
            last_seen_ts: None,
            lifecycle: Some("active".into()),
            idle: Some(false),
            session_memo: None,
            runtime: None,
            context: None,
        };
        let agent = RemoteAgent::from_row(row);
        assert_eq!(agent.project, "slyce");
        assert_eq!(agent.host.as_deref(), Some("herm-b"));
        assert_eq!(agent.user, "slyce");
        assert_eq!(agent.lane, "perky-e9fb");
        // serde deserialization of RemoteAgent has no pane field; PresenceRow
        // ignores pane_id/session_id on the wire.
        assert!(serde_json::to_value(&agent)
            .unwrap()
            .get("pane_id")
            .is_none());
        assert!(serde_json::to_value(&agent)
            .unwrap()
            .get("session_id")
            .is_none());
    }

    #[test]
    fn presence_context_usage_is_projected_without_forwarding_raw_payload() {
        let mut row = row("herm-b-slyce-perky-e9fb", "slyce", true);
        row.context = Some(serde_json::json!({
            "context_used": 50,
            "context_limit": 200,
            "depends_on": "D1"
        }));
        let agent = RemoteAgent::from_row(row);
        assert_eq!(agent.context_usage.as_deref(), Some("25%"));
        let value = serde_json::to_value(agent).unwrap();
        assert!(value.get("context").is_none());
        assert!(value.get("runtime").is_none());
    }

    #[test]
    fn derive_lane_and_user_match_canonical_rule() {
        assert_eq!(
            derive_lane("herm-b-slyce-perky-e9fb", Some("slyce")),
            "perky-e9fb"
        );
        assert_eq!(
            derive_lane("herm-b-herm-hackdaddy", Some("herm")),
            "hackdaddy"
        );
        assert_eq!(
            derive_lane("sebluair-herm-groovy-16be", Some("herm")),
            "groovy-16be"
        );
        assert_eq!(derive_lane("kooky-b9a3", None), "b9a3");
        let from_suffix_only_api = RemoteAgent::from_row(PresenceRow {
            agent_id: "sebluair-herm-groovy-16be".into(),
            session_id: None,
            project: Some("herm".into()),
            host: Some("sebluair".into()),
            lane: Some("16be".into()),
            cwd: None,
            user: None,
            effective_status: Some("idle".into()),
            last_status: None,
            process_alive: Some(true),
            stream_alive: Some(true),
            last_seen_ts: None,
            lifecycle: Some("active".into()),
            idle: Some(false),
            session_memo: None,
            runtime: None,
            context: None,
        });
        assert_eq!(from_suffix_only_api.lane, "groovy-16be");
        assert_eq!(
            derive_user(Some("slyce"), "herm-b-slyce-perky-e9fb"),
            "slyce"
        );
        assert_eq!(derive_user(Some("herm"), "herm-b-herm-hackdaddy"), "herm");
    }
}
