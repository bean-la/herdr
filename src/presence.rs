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
use std::time::Duration;

/// One row from herm-core `GET /v1/agent-presence` → `{ agents: [...] }`.
/// We only deserialize the read-only surface; we deliberately IGNORE any
/// `pane_id` / control field present on the wire (the sidebar never needs it
/// and must never expose it).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, serde::Serialize)]
pub struct PresenceRow {
    pub agent_id: String,
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
    // Deliberately NOT deserialized: pane_id / session_id / control token.
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
}

impl PresenceRow {
    /// True when the agent is currently live (process alive or lifecycle
    /// active). Used to filter stale rows from the default sidebar view.
    pub fn is_live(&self) -> bool {
        self.process_alive == Some(true) || self.lifecycle.as_deref() == Some("active")
    }
}

/// Derive a lane from an agent id (`host-project-lane` → lane; `host-lane` →
/// lane). Mirrors the herm data-layer rule. Falls back to the raw id.
pub fn derive_lane(agent_id: &str) -> String {
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
        let lane = row
            .lane
            .clone()
            .unwrap_or_else(|| derive_lane(&row.agent_id));
        let status = row
            .effective_status
            .clone()
            .or_else(|| row.last_status.clone())
            .unwrap_or_else(|| "idle".to_string());
        RemoteAgent {
            user: derive_user(row.project.as_deref(), &row.agent_id),
            agent_id: row.agent_id,
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

/// Fetch the cross-tenant presence feed from herm-core.
///
/// Reads env: `HERM_CORE_BASE` (default http://127.0.0.1:8787) and
/// `HERM_CORE_API_TOKEN` (auth). Returns Ok(agents) on success, Err on any
/// network/auth/parse failure — the caller degrades gracefully (keeps last
/// known rows, never panics, never blocks the render loop beyond `timeout`).
pub fn fetch_presence(timeout: Duration) -> Result<Vec<RemoteAgent>, String> {
    let base = std::env::var("HERM_CORE_BASE").unwrap_or_else(|_| "http://127.0.0.1:8787".into());
    let token = std::env::var("HERM_CORE_API_TOKEN").unwrap_or_default();
    let include_recent = std::env::var("HERDR_PRESENCE_INCLUDE_RECENT")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false);

    let agent = ureq::AgentBuilder::new().timeout(timeout).build();

    let mut req = agent.get(&format!("{base}/v1/agent-presence"));
    if !token.is_empty() {
        req = req.set("X-API-Token", token.as_str());
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
        };
        let agent = RemoteAgent::from_row(row);
        assert_eq!(agent.project, "slyce");
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
    fn derive_lane_and_user_match_canonical_rule() {
        assert_eq!(derive_lane("herm-b-slyce-perky-e9fb"), "perky-e9fb");
        assert_eq!(derive_lane("herm-b-herm-hackdaddy"), "hackdaddy");
        assert_eq!(derive_lane("kooky-b9a3"), "b9a3");
        assert_eq!(
            derive_user(Some("slyce"), "herm-b-slyce-perky-e9fb"),
            "slyce"
        );
        assert_eq!(derive_user(Some("herm"), "herm-b-herm-hackdaddy"), "herm");
    }
}
