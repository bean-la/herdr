use std::time::{Duration, Instant};

use super::App;
use crate::events::AppEvent;
use crate::presence::fetch_presence;

fn presence_refresh_interval() -> Duration {
    std::env::var("HERM_PRESENCE_POLL_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_secs(15))
}

impl App {
    pub(crate) fn presence_refresh_deadline(&self) -> Option<Instant> {
        (!self.presence_in_flight)
            .then_some(self.last_presence_refresh + presence_refresh_interval())
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
            let result = fetch_presence(Duration::from_secs(5))
                .map_err(|err| format!("presence fetch failed: {err}"));
            let _ = event_tx.blocking_send(AppEvent::PresenceRefreshed { result });
        });
    }

    pub(crate) fn handle_presence_refreshed(
        &mut self,
        result: Result<Vec<crate::presence::RemoteAgent>, String>,
    ) -> bool {
        self.presence_in_flight = false;
        self.last_presence_refresh = Instant::now();
        let agents = match result {
            Ok(agents) => agents,
            Err(_) => return false,
        };
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
    use crate::app::AppPolicy;
    use crate::presence::RemoteAgent;

    fn test_app(config: &crate::config::Config) -> App {
        App::new(
            config,
            AppPolicy::TEST,
            None,
            tokio::sync::mpsc::unbounded_channel().1,
            crate::api::EventHub::default(),
        )
    }

    fn remote(id: &str) -> RemoteAgent {
        RemoteAgent {
            agent_id: id.into(),
            host: Some("herm-b".into()),
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
            context_usage: None,
        }
    }

    #[test]
    fn handle_presence_refreshed_replaces_and_renders() {
        let config = crate::config::Config::default();
        let mut app = test_app(&config);
        app.state.remote_agents = vec![remote("herm-b-slyce-a")];
        let changed = app.handle_presence_refreshed(Ok(vec![remote("herm-b-slyce-b")]));
        assert!(changed);
        assert_eq!(app.state.remote_agents.len(), 1);
        assert_eq!(app.state.remote_agents[0].agent_id, "herm-b-slyce-b");
        assert!(!app.presence_in_flight);
    }

    #[test]
    fn handle_presence_refreshed_ok_empty_clears_stale_rows() {
        let config = crate::config::Config::default();
        let mut app = test_app(&config);
        app.state.remote_agents = vec![remote("herm-b-slyce-a")];
        let changed = app.handle_presence_refreshed(Ok(Vec::new()));
        assert!(changed);
        assert!(app.state.remote_agents.is_empty());
    }

    #[test]
    fn handle_presence_refreshed_err_keeps_last_known_on_outage() {
        let config = crate::config::Config::default();
        let mut app = test_app(&config);
        app.state.remote_agents = vec![remote("herm-b-slyce-a")];
        let changed = app.handle_presence_refreshed(Err("outage".to_string()));
        assert!(!changed);
        assert_eq!(app.state.remote_agents.len(), 1);
    }
}
