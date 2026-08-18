// ═══════════════════════════════════════════════════════════════════════
// Lazy herm-core presence poll (task 378f645a).
//
// Mirrors git_refresh's lazy-poll discipline: only poll herm-core presence
// when the sidebar is set up to show remote agents and the sidebar is not
// collapsed, on an env-tunable cadence (HERM_PRESENCE_POLL_MS, default 15s).
// Fetch runs on a background thread so the render loop is never blocked;
// failures degrade gracefully (keep last known rows, never panic).
// ═══════════════════════════════════════════════════════════════════════

use std::time::{Duration, Instant};

use super::{App, PRESENCE_REFRESH_INTERVAL};
use crate::events::AppEvent;
use crate::presence::fetch_presence;

/// True when the sidebar is configured to show a remote presence section.
/// Remote rows render only when the user has an `agent` sidebar token that
/// asks for them; for now we enable the feed whenever the sidebar is expanded
/// (the remote section is always shown there). Kept as a function so the
/// demand can be tightened to a token later without touching the loop.
fn remote_presence_wanted(app: &App) -> bool {
    // The remote presence section is always eligible when the sidebar is open;
    // gating on collapse avoids pointless polling when the sidebar is hidden.
    !app.state.sidebar_collapsed
}

impl App {
    pub(crate) fn presence_refresh_deadline(&self) -> Option<Instant> {
        (!self.presence_in_flight && remote_presence_wanted(self))
            .then_some(self.last_presence_refresh + *PRESENCE_REFRESH_INTERVAL)
    }

    pub(crate) fn start_presence_refresh_if_due(&mut self, now: Instant) {
        let Some(deadline) = self.presence_refresh_deadline() else {
            return;
        };
        if now < deadline {
            return;
        }
        self.presence_in_flight = true;
        let event_tx = self.event_tx.clone();
        std::thread::spawn(move || {
            let agents = match fetch_presence(Duration::from_secs(5)) {
                Ok(agents) => agents,
                Err(_err) => {
                    // Degrade gracefully: keep last-known rows. Send empty so
                    // the in-flight flag clears; the existing rows are retained
                    // by the consumer when no error is signaled. Simplest robust
                    // contract: on failure, emit PresenceRefreshed with the last
                    // known set unchanged is handled by the consumer — here we
                    // send an empty vec and the handler only replaces when a
                    // fetch actually succeeded (see handle_presence_refreshed).
                    Vec::new()
                }
            };
            let _ = event_tx.blocking_send(AppEvent::PresenceRefreshed { agents });
        });
    }

    /// Consume a presence refresh result. Only replaces state when the fetch
    /// actually produced rows; an empty result (fetch failure) leaves the last
    /// known remote rows in place so the sidebar doesn't flicker on a herm-core
    /// blip. Never panics, never blocks.
    pub(crate) fn handle_presence_refreshed(
        &mut self,
        agents: Vec<crate::presence::RemoteAgent>,
    ) -> bool {
        self.presence_in_flight = false;
        self.last_presence_refresh = Instant::now();
        // Only replace when we got real data (or an explicit empty when the
        // feed genuinely has nothing live — which is indistinguishable here;
        // so we replace on any non-empty, and on empty we treat as "no change"
        // only if we already have rows, else clear). This keeps a herm-core
        // outage from wiping the sidebar.
        if agents.is_empty() && !self.state.remote_agents.is_empty() {
            // Fetch failed or nothing live — retain last known.
            return false;
        }
        if agents == self.state.remote_agents {
            return false;
        }
        self.state.remote_agents = agents;
        self.render_dirty.request_generic();
        self.render_notify.notify_one();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presence::RemoteAgent;

    fn test_app(config: &crate::config::Config) -> super::super::App {
        super::super::App::new(
            config,
            true,
            None,
            tokio::sync::mpsc::unbounded_channel().1,
            crate::api::EventHub::default(),
        )
    }

    fn remote(id: &str) -> RemoteAgent {
        RemoteAgent {
            agent_id: id.into(),
            project: "slyce".into(),
            lane: "perky-e9fb".into(),
            status: "idle".into(),
            user: "slyce".into(),
            cwd: None,
            process_alive: true,
            stream_alive: true,
            last_seen_ts: None,
            lifecycle: Some("active".into()),
            idle: Some(false),
            session_memo: None,
        }
    }

    #[test]
    fn presence_refresh_deadline_gated_on_sidebar_collapse() {
        let config = crate::config::Config::default();
        let mut app = test_app(&config);
        app.state.sidebar_collapsed = false;
        app.last_presence_refresh = Instant::now();
        // Not due yet.
        assert!(app.presence_refresh_deadline().is_some());

        app.state.sidebar_collapsed = true;
        // Collapsed → no demand → no deadline.
        assert!(app.presence_refresh_deadline().is_none());
    }

    #[test]
    fn handle_presence_refreshed_replaces_and_renders() {
        let config = crate::config::Config::default();
        let mut app = test_app(&config);
        app.state.remote_agents = vec![remote("herm-b-slyce-a")];
        let changed = app.handle_presence_refreshed(vec![remote("herm-b-slyce-b")]);
        assert!(changed);
        assert_eq!(app.state.remote_agents.len(), 1);
        assert_eq!(app.state.remote_agents[0].agent_id, "herm-b-slyce-b");
        assert!(!app.presence_in_flight);
    }

    #[test]
    fn handle_presence_refreshed_empty_keeps_last_known_on_outage() {
        let config = crate::config::Config::default();
        let mut app = test_app(&config);
        app.state.remote_agents = vec![remote("herm-b-slyce-a")];
        // Empty = fetch failure / herm-core down → keep last known.
        let changed = app.handle_presence_refreshed(Vec::new());
        assert!(!changed);
        assert_eq!(app.state.remote_agents.len(), 1);
    }
}
