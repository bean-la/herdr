use super::render::put_text;
use super::*;

pub(super) fn render_collapsed(
    buffer: &mut Buffer,
    area: Rect,
    endpoints: &[ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    config: &ClientShellConfig,
    hits: &mut ShellHitMap,
) {
    let rows = agent_rows(endpoints, active_endpoint_id, config);
    for (index, row) in rows.into_iter().take(area.height as usize).enumerate() {
        let rect = Rect::new(area.x, area.y + index as u16, area.width, 1);
        if row.agent.focused {
            buffer.set_style(rect, Style::default().bg(config.palette.active_row_bg));
        }
        let initial = row.machine_label.chars().next().unwrap_or('?');
        put_text(
            buffer,
            rect.x,
            rect.y,
            rect.width,
            &format!(
                "{initial}{}",
                status_icon(row.agent.status, config.status_indicators)
            ),
            Style::default()
                .fg(if row.stale {
                    config.palette.overlay0
                } else {
                    status_color(row.agent.status, &config.palette)
                })
                .add_modifier(if row.stale {
                    Modifier::DIM
                } else {
                    Modifier::empty()
                }),
        );
        hits.endpoint_agents
            .push((rect, row.endpoint_id, row.agent.pane_id));
    }
}

pub(super) fn render_expanded(
    buffer: &mut Buffer,
    area: Rect,
    agent_view_label: Option<&str>,
    endpoints: &[ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    config: &ClientShellConfig,
    agent_scroll: &mut usize,
    hits: &mut ShellHitMap,
) {
    if !super::agent_sidebar::render_agent_panel_header(
        buffer,
        area,
        agent_view_label,
        config,
        hits,
    ) {
        return;
    }
    let rows = agent_rows(endpoints, active_endpoint_id, config);
    super::agent_sidebar::render_agent_list(
        buffer,
        area,
        &rows,
        agent_view_label.map(|_| " no matching agents"),
        config,
        agent_scroll,
        hits,
        |row| row.agent.rows.len(),
        |buffer, rect, row, hits| {
            super::agent_sidebar::render_agent_row(buffer, rect, &row.agent, config);
            if row.stale {
                buffer.set_style(
                    rect,
                    Style::default()
                        .fg(config.palette.overlay0)
                        .add_modifier(Modifier::DIM),
                );
            }
            if !row.agent.remote {
                hits.endpoint_agents.push((
                    rect,
                    row.endpoint_id.clone(),
                    row.agent.pane_id.clone(),
                ));
            }
        },
    );
}

impl ClientShellState {
    pub(super) fn reveal_endpoint_agent(
        &mut self,
        endpoint_id: &ClientEndpointId,
        pane_id: &str,
        body_height: u16,
    ) {
        if body_height == 0 {
            return;
        }
        let rows = agent_rows(&self.endpoints, &self.active_endpoint_id, &self.config);
        let Some(target) = rows
            .iter()
            .position(|row| &row.endpoint_id == endpoint_id && row.agent.pane_id == pane_id)
        else {
            return;
        };
        let heights = rows
            .iter()
            .map(|row| row.agent.rows.len().max(1).min(u16::MAX as usize) as u16)
            .collect::<Vec<_>>();
        let mut gaps = vec![self.config.agents.row_gap; rows.len()];
        if let Some(last) = gaps.last_mut() {
            *last = 0;
        }
        self.agent_scroll = super::scroll::list_scroll_start_to_reveal(
            &heights,
            &gaps,
            body_height,
            self.agent_scroll,
            target,
        );
    }
}

struct EndpointAgentRow {
    endpoint_id: ClientEndpointId,
    machine_label: String,
    stale: bool,
    agent: super::agent_sidebar::AgentRow,
}

fn agent_rows(
    endpoints: &[ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    config: &ClientShellConfig,
) -> Vec<EndpointAgentRow> {
    let mut rendered_rows = HashMap::new();
    for endpoint in endpoints {
        let Some(snapshot) = endpoint.snapshot.as_deref() else {
            continue;
        };
        for pane_agent in &snapshot.agents {
            if !super::agent_sidebar::agent_matches_scope(
                snapshot,
                pane_agent.workspace_id.as_str(),
                config.agent_panel_scope,
            ) {
                continue;
            }
            if let Some(row) = super::agent_sidebar::agent_row(
                snapshot,
                &pane_agent.pane_id,
                config,
                Some(endpoint.label.as_str()),
            ) {
                rendered_rows.insert((endpoint.endpoint_id.clone(), row.pane_id.clone()), row);
            }
        }
        for row in super::agent_sidebar::agent_rows(snapshot, config, Some(&endpoint.label)) {
            if snapshot
                .agents
                .iter()
                .any(|pane_agent| pane_agent.pane_id == row.pane_id)
            {
                continue;
            }
            rendered_rows.insert((endpoint.endpoint_id.clone(), row.pane_id.clone()), row);
        }
    }
    let mut ordered_keys = Vec::new();

    // Aggregate navigation orders pane-backed agents. Presence and lane-tab rows
    // are not pane targets, so append them in endpoint snapshot order afterward.
    for row in super::aggregate_navigation::aggregate_agent_rows(
        endpoints,
        active_endpoint_id,
        config.agent_panel_sort,
    ) {
        ordered_keys.push((row.endpoint.endpoint_id.clone(), row.agent.pane_id.clone()));
    }
    for endpoint in endpoints {
        if let Some(snapshot) = endpoint.snapshot.as_deref() {
            for agent in super::agent_sidebar::agent_rows(snapshot, config, Some(&endpoint.label)) {
                let key = (endpoint.endpoint_id.clone(), agent.pane_id.clone());
                let is_pane_backed = snapshot
                    .agents
                    .iter()
                    .any(|pane_agent| pane_agent.pane_id == agent.pane_id);
                if !is_pane_backed && !ordered_keys.contains(&key) {
                    ordered_keys.push(key);
                }
            }
        }
    }

    ordered_keys
        .into_iter()
        .filter_map(|key| {
            let endpoint = endpoints
                .iter()
                .find(|endpoint| endpoint.endpoint_id == key.0)?;
            let mut agent = rendered_rows.remove(&key)?;
            agent.focused &= &endpoint.endpoint_id == active_endpoint_id;
            Some(EndpointAgentRow {
                endpoint_id: endpoint.endpoint_id.clone(),
                machine_label: endpoint.label.clone(),
                stale: endpoint.status != ClientEndpointStatus::Online,
                agent,
            })
        })
        .collect()
}
