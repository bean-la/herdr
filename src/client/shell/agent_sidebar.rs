use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
    widgets::{Paragraph, Widget},
};

use super::*;

pub(super) struct AgentRow {
    pub(super) pane_id: String,
    pub(super) status: crate::api::schema::AgentStatus,
    pub(super) focused: bool,
    pub(super) remote: bool,
    pub(super) rows: Vec<Vec<crate::ui::ResolvedToken>>,
}

pub(super) fn ordered_agent_pane_ids(
    snapshot: &ClientShellSnapshot,
    sort: crate::config::AgentPanelSortConfig,
) -> Vec<String> {
    if snapshot.agent_view_label.is_some() {
        return snapshot
            .agent_order
            .iter()
            .filter(|pane_id| {
                snapshot
                    .agents
                    .iter()
                    .any(|agent| agent.pane_id == pane_id.as_str())
            })
            .cloned()
            .collect();
    }
    let mut agents = snapshot.agents.iter().collect::<Vec<_>>();
    if sort == crate::config::AgentPanelSortConfig::Priority {
        agents.sort_by_key(|agent| {
            (
                std::cmp::Reverse(status_priority(agent.agent_status)),
                std::cmp::Reverse(agent.state_change_seq),
            )
        });
    }
    agents
        .into_iter()
        .map(|agent| agent.pane_id.clone())
        .collect()
}

pub(super) fn render_agent_panel(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    agent_scroll: &mut usize,
    hits: &mut ShellHitMap,
) {
    if !render_agent_panel_header(
        buffer,
        area,
        snapshot.agent_view_label.as_deref(),
        config,
        hits,
    ) {
        return;
    }

    let rows = agent_rows(snapshot, config, None);
    render_agent_list(
        buffer,
        area,
        &rows,
        snapshot
            .agent_view_label
            .as_ref()
            .map(|_| " no matching agents"),
        config,
        agent_scroll,
        hits,
        |row| row.rows.len(),
        |buffer, rect, row, hits| {
            if !row.remote {
                hits.agents.push((rect, row.pane_id.clone()));
            }
            render_agent_row(buffer, rect, row, config);
        },
    );
}

pub(super) fn render_agent_panel_header(
    buffer: &mut Buffer,
    area: Rect,
    agent_view_label: Option<&str>,
    config: &ClientShellConfig,
    hits: &mut ShellHitMap,
) -> bool {
    if area.height == 0 {
        return false;
    }
    put_text(
        buffer,
        area.x,
        area.y,
        area.width,
        &"─".repeat(area.width as usize),
        Style::default().fg(config.palette.surface_dim),
    );
    if area.height < 2 {
        return false;
    }
    let header_y = area.y + 1;
    if let Some(label) = agent_view_label {
        let label_width = display_width(label).min(area.width as usize) as u16;
        let label_rect = Rect::new(area.right().saturating_sub(label_width), header_y, label_width, 1);
        put_text(buffer, label_rect.x, label_rect.y, label_rect.width, label, Style::default().fg(config.palette.accent).add_modifier(Modifier::BOLD));
        return true;
    }

    let sort_label = match config.agent_panel_sort {
        crate::config::AgentPanelSortConfig::Spaces => "grpd",
        crate::config::AgentPanelSortConfig::Priority => "prio",
    };
    let scope_label = agent_panel_scope_label(config.agent_panel_scope);
    let remotes_label = agent_panel_remotes_label(config.agent_panel_remotes);
    let labels = [sort_label, scope_label, remotes_label];
    let rects = right_aligned_toggle_rects(area, header_y, &labels);
    hits.agent_sort_toggle = if config.mouse_capture {
        rects[0]
    } else {
        Rect::default()
    };
    hits.agent_scope_toggle = if config.mouse_capture {
        rects[1]
    } else {
        Rect::default()
    };
    hits.agent_remotes_toggle = if config.mouse_capture {
        rects[2]
    } else {
        Rect::default()
    };
    let toggle_style = Style::default()
        .fg(config.palette.overlay0)
        .add_modifier(Modifier::BOLD);
    let separator_style = Style::default().fg(config.palette.surface_dim);
    for (index, (label, rect)) in labels.iter().zip(rects.iter()).enumerate() {
        if index > 0 {
            let previous = rects[index - 1];
            let gap = rect.x.saturating_sub(previous.right());
            if gap > 0 {
                put_text(
                    buffer,
                    previous.right(),
                    header_y,
                    gap,
                    " · ",
                    separator_style,
                );
            }
        }
        put_text(buffer, rect.x, rect.y, rect.width, label, toggle_style);
    }
    true
}

fn right_aligned_toggle_rects(area: Rect, y: u16, labels: &[&str]) -> Vec<Rect> {
    let separator_width = display_width(" · ") as u16;
    let widths = labels
        .iter()
        .map(|label| display_width(label).min(area.width as usize) as u16)
        .collect::<Vec<_>>();
    let mut total = widths.iter().copied().sum::<u16>();
    if labels.len() > 1 {
        total = total.saturating_add(separator_width.saturating_mul((labels.len() - 1) as u16));
    }
    let mut x = area.right().saturating_sub(total.min(area.width));
    widths
        .into_iter()
        .map(|width| {
            let rect = Rect::new(x.min(area.right()), y, width.min(area.right().saturating_sub(x)), 1);
            x = x.saturating_add(width).saturating_add(separator_width);
            rect
        })
        .collect()
}

fn agent_panel_scope_label(
    scope: crate::config::AgentPanelScopeConfig,
) -> &'static str {
    match scope {
        crate::config::AgentPanelScopeConfig::All => "all",
        crate::config::AgentPanelScopeConfig::ActiveWorkspace => "here",
    }
}

fn agent_panel_remotes_label(
    remotes: crate::config::AgentPanelRemotesConfig,
) -> &'static str {
    match remotes {
        crate::config::AgentPanelRemotesConfig::Show => "remotes",
        crate::config::AgentPanelRemotesConfig::Hide => "local",
    }
}

/// Workspace used by the "here" agent-panel scope.
///
/// Per-client shell location can lag behind the focused pane on multi-client VPS
/// hosts, so fall back to the focused agent/pane workspace before filtering.
fn effective_scope_workspace_id(snapshot: &ClientShellSnapshot) -> Option<&str> {
    snapshot
        .focused_workspace_id
        .as_deref()
        .or_else(|| {
            snapshot
                .agents
                .iter()
                .find(|agent| agent.focused)
                .map(|agent| agent.workspace_id.as_str())
        })
        .or_else(|| {
            snapshot
                .focused_pane_id
                .as_deref()
                .and_then(|pane_id| {
                    snapshot
                        .panes
                        .iter()
                        .find(|pane| pane.pane_id == pane_id)
                        .map(|pane| pane.workspace_id.as_str())
                })
        })
        .or_else(|| {
            snapshot
                .workspaces
                .iter()
                .find(|workspace| workspace.focused)
                .map(|workspace| workspace.workspace_id.as_str())
        })
}

fn effective_scope_workspace_label(snapshot: &ClientShellSnapshot) -> Option<&str> {
    effective_scope_workspace_id(snapshot).and_then(|workspace_id| {
        snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.workspace_id == workspace_id)
            .map(|workspace| workspace.label.as_str())
    })
}

fn agent_matches_scope(
    snapshot: &ClientShellSnapshot,
    agent_workspace_id: &str,
    scope: crate::config::AgentPanelScopeConfig,
) -> bool {
    if scope == crate::config::AgentPanelScopeConfig::All {
        return true;
    }
    match effective_scope_workspace_id(snapshot) {
        Some(workspace_id) => agent_workspace_id == workspace_id,
        None => true,
    }
}

fn remote_presence_matches_scope(
    snapshot: &ClientShellSnapshot,
    project: &str,
    scope: crate::config::AgentPanelScopeConfig,
) -> bool {
    if scope == crate::config::AgentPanelScopeConfig::All {
        return true;
    }
    match effective_scope_workspace_label(snapshot) {
        Some(label) => project == label,
        None => true,
    }
}

fn live_presence_covers_lane(
    snapshot: &ClientShellSnapshot,
    project: &str,
    lane: &str,
) -> bool {
    snapshot.remote_agents.iter().any(|agent| {
        agent.process_alive && agent.project == project && agent.lane == lane
    })
}

/// True when this server already has a workspace tab named for the lane.
/// Fleet lanes on the VPS should render as local tab rows, not presence rows.
fn workspace_has_lane_tab(
    snapshot: &ClientShellSnapshot,
    project: &str,
    lane: &str,
) -> bool {
    snapshot
        .workspaces
        .iter()
        .filter(|workspace| workspace.label == project)
        .flat_map(|workspace| {
            snapshot
                .tabs
                .iter()
                .filter(|tab| tab.workspace_id == workspace.workspace_id)
        })
        .any(|tab| tab.label == lane)
}

fn primary_pane_for_tab<'a>(
    snapshot: &'a ClientShellSnapshot,
    workspace_id: &str,
    tab_id: &str,
) -> Option<&'a crate::protocol::ClientShellPane> {
    let panes = snapshot
        .panes
        .iter()
        .filter(|pane| pane.workspace_id == workspace_id && pane.tab_id == tab_id)
        .collect::<Vec<_>>();
    panes
        .iter()
        .find(|pane| pane.focused)
        .or_else(|| panes.first())
        .copied()
}

fn lane_tab_agent_rows(
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
) -> Vec<AgentRow> {
    if snapshot.agent_view_label.is_some() {
        return Vec::new();
    }
    let covered_panes = snapshot
        .agents
        .iter()
        .map(|agent| agent.pane_id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut rows = Vec::new();
    for workspace in &snapshot.workspaces {
        if !agent_matches_scope(
            snapshot,
            workspace.workspace_id.as_str(),
            config.agent_panel_scope,
        ) {
            continue;
        }
        let mut tabs = snapshot
            .tabs
            .iter()
            .filter(|tab| tab.workspace_id == workspace.workspace_id)
            .collect::<Vec<_>>();
        tabs.sort_by_key(|tab| tab.number);
        for tab in tabs {
            let Some(pane) = primary_pane_for_tab(snapshot, &workspace.workspace_id, &tab.tab_id)
            else {
                continue;
            };
            if covered_panes.contains(pane.pane_id.as_str()) {
                continue;
            }
            if !live_presence_covers_lane(snapshot, workspace.label.as_str(), tab.label.as_str()) {
                continue;
            }
            let tab_label = Some(tab.label.as_str());
            let ui_rows = crate::ui::sidebar_agent_rows(
                &config.agents,
                crate::ui::AgentTokenContext {
                    machine: None,
                    workspace: &workspace.label,
                    tab: tab_label,
                    pane: pane.label.as_deref(),
                    agent_label: Some(tab.label.as_str()),
                    terminal_title: None,
                    terminal_title_stripped: None,
                    canonical_agent: None,
                    tokens: &HashMap::new(),
                },
                sidebar_status_text(crate::api::schema::AgentStatus::Idle),
            );
            rows.push(AgentRow {
                pane_id: pane.pane_id.clone(),
                status: crate::api::schema::AgentStatus::Idle,
                focused: pane.focused,
                remote: false,
                rows: ui_rows,
            });
        }
    }
    rows
}

pub(super) fn render_agent_list<T>(
    buffer: &mut Buffer,
    area: Rect,
    rows: &[T],
    empty_message: Option<&str>,
    config: &ClientShellConfig,
    agent_scroll: &mut usize,
    hits: &mut ShellHitMap,
    row_lines: impl Fn(&T) -> usize,
    mut render_row: impl FnMut(&mut Buffer, Rect, &T, &mut ShellHitMap),
) {
    let body = Rect::new(
        area.x,
        area.y.saturating_add(3),
        area.width,
        area.height.saturating_sub(3),
    );
    hits.agent_body = body;
    if body.is_empty() || rows.is_empty() {
        *agent_scroll = 0;
        if let Some(message) = empty_message.filter(|_| !body.is_empty()) {
            put_text(
                buffer,
                body.x,
                body.y,
                body.width,
                message,
                Style::default()
                    .fg(config.palette.overlay0)
                    .add_modifier(Modifier::DIM),
            );
        }
        return;
    }

    let row_heights = rows
        .iter()
        .map(|row| row_lines(row).max(1).min(u16::MAX as usize) as u16)
        .collect::<Vec<_>>();
    let gaps = rows
        .iter()
        .enumerate()
        .map(|(index, _)| {
            if index + 1 < rows.len() {
                config.agents.row_gap
            } else {
                0
            }
        })
        .collect::<Vec<_>>();
    let metrics =
        super::scroll::list_scroll_metrics(&row_heights, &gaps, body.height, *agent_scroll);
    hits.agent_max_scroll = metrics.max_offset_from_bottom;
    hits.agent_scroll_metrics = Some(metrics);
    *agent_scroll = metrics
        .max_offset_from_bottom
        .saturating_sub(metrics.offset_from_bottom);
    let show_scrollbar = metrics.max_offset_from_bottom > 0 && body.width > 1;
    let content_width = body.width.saturating_sub(u16::from(show_scrollbar));
    let mut y = body.y;
    for (index, row) in rows.iter().enumerate().skip(*agent_scroll) {
        let height = row_heights[index].min(body.height);
        if y.saturating_add(height) > body.bottom() {
            break;
        }
        let rect = Rect::new(body.x, y, content_width, height);
        render_row(buffer, rect, row, hits);
        y = y
            .saturating_add(height)
            .saturating_add(if index + 1 < rows.len() {
                config.agents.row_gap
            } else {
                0
            });
    }

    if show_scrollbar {
        let track = Rect::new(body.right().saturating_sub(1), body.y, 1, body.height);
        hits.agent_scrollbar = track;
        super::scroll::render_list_scrollbar(buffer, track, metrics, &config.palette);
    }
}

pub(super) fn agent_rows(
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    machine: Option<&str>,
) -> Vec<AgentRow> {
    ordered_agent_pane_ids(snapshot, config.agent_panel_sort)
        .into_iter()
        .filter_map(|pane_id| {
            let agent = snapshot
                .agents
                .iter()
                .find(|agent| agent.pane_id == pane_id)?;
            if !agent_matches_scope(snapshot, agent.workspace_id.as_str(), config.agent_panel_scope)
            {
                return None;
            }
            let workspace = snapshot
                .workspaces
                .iter()
                .find(|workspace| workspace.workspace_id == agent.workspace_id)?;
            let tab = snapshot.tabs.iter().find(|tab| tab.tab_id == agent.tab_id);
            let pane = snapshot
                .panes
                .iter()
                .find(|pane| pane.pane_id == agent.pane_id);
            let tab_label = tab.map(|tab| tab.label.as_str());
            let agent_label = agent
                .display_agent
                .as_deref()
                .or(agent.name.as_deref())
                .or(agent.agent.as_deref())
                .or(agent.title.as_deref());
            let labels = agent
                .state_labels
                .iter()
                .cloned()
                .collect::<HashMap<_, _>>();
            let tokens = agent.tokens.iter().cloned().collect::<HashMap<_, _>>();
            let state_text = labels
                .get(status_text(agent.agent_status))
                .map(String::as_str)
                .unwrap_or_else(|| sidebar_status_text(agent.agent_status));
            let canonical_agent = agent
                .agent
                .as_deref()
                .and_then(crate::detect::parse_agent_label);
            let rows = crate::ui::sidebar_agent_rows(
                &config.agents,
                crate::ui::AgentTokenContext {
                    machine,
                    workspace: &workspace.label,
                    tab: tab_label,
                    pane: agent
                        .title
                        .as_deref()
                        .or_else(|| pane.and_then(|pane| pane.label.as_deref())),
                    agent_label,
                    terminal_title: agent.terminal_title.as_deref(),
                    terminal_title_stripped: agent.terminal_title_stripped.as_deref(),
                    canonical_agent,
                    tokens: &tokens,
                },
                state_text,
            );
            Some(AgentRow {
                pane_id: agent.pane_id.clone(),
                status: agent.agent_status,
                focused: agent.focused,
                remote: false,
                rows,
            })
        })
        .chain(lane_tab_agent_rows(snapshot, config))
        .chain(remote_agent_rows(snapshot, config))
        .collect()
}

fn remote_agent_rows<'a>(
    snapshot: &'a ClientShellSnapshot,
    config: &'a ClientShellConfig,
) -> impl Iterator<Item = AgentRow> + 'a {
    snapshot
        .remote_agents
        .iter()
        .filter(|_| snapshot.agent_view_label.is_none())
        .filter(|_| config.agent_panel_remotes == crate::config::AgentPanelRemotesConfig::Show)
        .filter(|agent| agent.process_alive)
        .filter(|agent| {
            remote_presence_matches_scope(snapshot, agent.project.as_str(), config.agent_panel_scope)
        })
        .filter(|agent| {
            !workspace_has_lane_tab(snapshot, agent.project.as_str(), agent.lane.as_str())
        })
        .map(|agent| {
            let status = remote_agent_status(&agent.status);
            let label = agent.lane.as_str();
            let kind = remote_agent_kind(agent);
            let canonical_agent = (kind == "Pi").then_some(crate::detect::Agent::Pi);
            let mut tokens = HashMap::new();
            if let Some(memo) = agent
                .session_memo
                .as_deref()
                .filter(|memo| !memo.is_empty())
            {
                tokens.insert("session_memo".to_string(), memo.to_string());
            }
            tokens.insert("kind".to_string(), kind.to_string());
            if let Some(last_seen) = agent
                .last_seen_ts
                .as_deref()
                .and_then(remote_relative_time)
            {
                tokens.insert("last_seen".to_string(), last_seen);
            }
            if let Some(context) = agent.context_usage.clone() {
                tokens.insert("context".to_string(), context);
            }
            let rows = crate::ui::sidebar_agent_rows(
                &config.agents,
                crate::ui::AgentTokenContext {
                    machine: agent.host.as_deref(),
                    workspace: &agent.project,
                    tab: Some(agent.lane.as_str()),
                    pane: None,
                    agent_label: Some(label),
                    terminal_title: None,
                    terminal_title_stripped: None,
                    canonical_agent,
                    tokens: &tokens,
                },
                sidebar_status_text(status),
            );
            AgentRow {
                pane_id: format!("remote:{}", agent.agent_id),
                status,
                focused: false,
                remote: true,
                rows,
            }
        })
}

/// Presence project identity is the only stable kind signal currently on the
/// read-only feed: `herm` is the HRM fleet; project-user sessions are Pi.
fn remote_agent_kind(agent: &crate::protocol::ClientShellRemoteAgent) -> &'static str {
    if agent.project == "herm" {
        "HRM"
    } else {
        "Pi"
    }
}

/// Format presence freshness without claiming that it is a mailbox message.
/// Invalid timestamps are omitted; this keeps old/partial feeds honest.
fn remote_relative_time(timestamp: &str) -> Option<String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    remote_relative_time_at(timestamp, now)
}

fn remote_relative_time_at(timestamp: &str, now: i64) -> Option<String> {
    let seen = parse_rfc3339_seconds(timestamp)?;
    let age = now.saturating_sub(seen).max(0);
    let value = if age < 60 {
        format!("{age}s")
    } else if age < 3_600 {
        format!("{}m", age / 60)
    } else if age < 86_400 {
        format!("{}h", age / 3_600)
    } else {
        format!("{}d", age / 86_400)
    };
    Some(format!("{value} ago"))
}

/// Minimal RFC3339 parser for presence timestamps. Keeping this local avoids
/// making the endpoint protocol depend on a parser feature just for a token.
fn parse_rfc3339_seconds(timestamp: &str) -> Option<i64> {
    fn digits(value: &[u8]) -> Option<i64> {
        value.iter().try_fold(0_i64, |acc, digit| {
            digit
                .is_ascii_digit()
                .then_some(acc * 10 + i64::from(digit - b'0'))
        })
    }
    let bytes = timestamp.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let year = digits(&bytes[0..4])?;
    let month = digits(&bytes[5..7])?;
    let day = digits(&bytes[8..10])?;
    let hour = digits(&bytes[11..13])?;
    let minute = digits(&bytes[14..16])?;
    let second = digits(&bytes[17..19])?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let mut index = 19;
    while bytes.get(index).is_some_and(|byte| byte.is_ascii_digit()) {
        index += 1;
    }
    let offset = match bytes.get(index) {
        Some(b'Z') if index + 1 == bytes.len() => 0_i64,
        Some(sign @ (b'+' | b'-')) if index + 6 == bytes.len() && bytes[index + 3] == b':' => {
            let hours = digits(&bytes[index + 1..index + 3])?;
            let minutes = digits(&bytes[index + 4..index + 6])?;
            if hours > 23 || minutes > 59 {
                return None;
            }
            let value = hours * 3_600 + minutes * 60;
            if *sign == b'+' {
                value
            } else {
                -value
            }
        }
        _ => return None,
    };
    // Days from civil (Gregorian), relative to 1970-01-01.
    let adjusted_year = year - i64::from(month <= 2);
    let era = (if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    }) / 400;
    let year_of_era = adjusted_year - era * 400;
    let month_from_march = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_from_march + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second - offset)
}

fn remote_agent_status(status: &str) -> crate::api::schema::AgentStatus {
    match status {
        "working" => crate::api::schema::AgentStatus::Working,
        "blocked" => crate::api::schema::AgentStatus::Blocked,
        "done" => crate::api::schema::AgentStatus::Done,
        _ => crate::api::schema::AgentStatus::Idle,
    }
}

pub(super) fn render_agent_row(
    buffer: &mut Buffer,
    rect: Rect,
    row: &AgentRow,
    config: &ClientShellConfig,
) {
    let palette = &config.palette;
    let row_style = if row.focused {
        Style::default().bg(palette.active_row_bg)
    } else {
        Style::default()
    };
    let remote_modifier = if row.remote {
        Modifier::DIM | Modifier::ITALIC
    } else {
        Modifier::empty()
    };
    let name_style = if row.focused {
        Style::default()
            .fg(palette.text)
            .add_modifier(Modifier::BOLD | remote_modifier)
    } else {
        Style::default()
            .fg(palette.subtext0)
            .add_modifier(Modifier::BOLD | remote_modifier)
    };
    let status_style = Style::default()
        .fg(status_color(row.status, palette))
        .add_modifier(if row.focused {
            Modifier::empty()
        } else {
            Modifier::DIM
        });
    let secondary = Style::default()
        .fg(palette.overlay0)
        .add_modifier(Modifier::DIM);
    let icon = (
        status_icon(row.status, config.status_indicators),
        Style::default().fg(status_color(row.status, palette)),
    );
    let rows = if row.rows.is_empty() {
        vec![vec![crate::ui::ResolvedToken {
            kind: crate::ui::ResolvedTokenKind::StateIcon,
            style: Default::default(),
        }]]
    } else {
        row.rows.clone()
    };
    for (index, tokens) in rows.iter().take(rect.height as usize).enumerate() {
        let indent = if index == 0 { 1 } else { 3 };
        let mut spans = vec![ratatui::text::Span::raw(" ".repeat(indent))];
        spans.extend(crate::ui::resolved_token_spans(
            tokens,
            icon,
            status_style,
            name_style,
            secondary,
            secondary,
            palette,
            rect.width.saturating_sub(indent as u16) as usize,
        ));
        Paragraph::new(Line::from(spans)).style(row_style).render(
            Rect::new(rect.x, rect.y + index as u16, rect.width, 1),
            buffer,
        );
    }
}

fn put_text(buffer: &mut Buffer, x: u16, y: u16, width: u16, text: &str, style: Style) {
    for (offset, character) in text.chars().take(width as usize).enumerate() {
        if let Some(cell) = buffer.cell_mut((x + offset as u16, y)) {
            cell.set_char(character).set_style(style);
        }
    }
}

fn display_width(text: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(text)
}

fn sidebar_status_text(status: crate::api::schema::AgentStatus) -> &'static str {
    use crate::api::schema::AgentStatus;
    match status {
        AgentStatus::Blocked => "blocked",
        AgentStatus::Done => "done",
        AgentStatus::Working => "working",
        AgentStatus::Idle | AgentStatus::Unknown => "idle",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(project: &str) -> crate::protocol::ClientShellRemoteAgent {
        crate::protocol::ClientShellRemoteAgent {
            agent_id: "host-project-lane".into(),
            host: Some("host".into()),
            project: project.into(),
            lane: "lane".into(),
            status: "working".into(),
            user: project.into(),
            cwd: None,
            process_alive: true,
            stream_alive: true,
            last_seen_ts: None,
            session_memo: None,
            context_usage: None,
        }
    }

    #[test]
    fn remote_kind_maps_project_user_and_hrm_presence_without_guessing_host() {
        assert_eq!(remote_agent_kind(&remote("slyce")), "Pi");
        assert_eq!(remote_agent_kind(&remote("herm")), "HRM");
        let mut missing_host = remote("slyce");
        missing_host.host = None;
        assert_eq!(missing_host.host, None);
    }

    #[test]
    fn remote_relative_time_handles_fresh_future_and_bad_timestamps() {
        assert_eq!(
            remote_relative_time_at("2026-09-14T00:00:00Z", 1_789_344_000),
            Some("0s ago".into())
        );
        assert_eq!(
            remote_relative_time_at("2026-09-14T01:00:00+01:00", 1_789_344_000),
            Some("0s ago".into())
        );
        assert_eq!(remote_relative_time_at("not-a-time", 0), None);
    }

    #[test]
    fn remote_context_usage_projection_preserves_absence() {
        let mut agent = remote("slyce");
        agent.context_usage = Some("25%".into());
        assert_eq!(agent.context_usage.as_deref(), Some("25%"));
        agent.context_usage = None;
        assert_eq!(agent.context_usage, None);
    }
}
