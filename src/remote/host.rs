//! Remote-host side of the SSH stdio bridge.

use std::io;
use std::time::Duration;

#[cfg(unix)]
use std::ffi::CStr;

/// D126/D127: native remote-client-bridge is reserved for the herm service
/// user (and root). A project user must never run it — it would spawn an
/// independent project-user herdr server (the D126 isolation violation seen
/// via `brndr --remote slyce@herm-b`). Route project users through the scoped
/// `brn o --remote` (boot.ts → attach-proxy) path instead.
///
/// Native `--remote herm-b` as the herm service user remains allowed.
#[cfg(unix)]
fn native_remote_allowed_for(effective_user: &str) -> bool {
    effective_user == "herm" || effective_user == "root"
}

/// Resolve the effective username from getpwuid(geteuid()) — NOT the
/// spoofable $USER env var.
#[cfg(unix)]
fn effective_username() -> Option<String> {
    unsafe {
        let uid = libc::geteuid();
        let pwd = libc::getpwuid(uid);
        if pwd.is_null() {
            return None;
        }
        let name = (*pwd).pw_name;
        if name.is_null() {
            return None;
        }
        Some(CStr::from_ptr(name).to_string_lossy().into_owned())
    }
}

pub(crate) fn run_remote_client_bridge() -> io::Result<()> {
    #[cfg(unix)]
    if let Some(user) = effective_username() {
        if !native_remote_allowed_for(&user) {
            eprintln!(
                "brndr: native remote-client-bridge is reserved for the herm service user. \
                 Project users must attach via `brn o --remote <user>@<host>` (boot.ts → attach-proxy). \
                 Refusing to spawn an independent project-user herdr server (D126/D127)."
            );
            return Err(io::Error::other(
                "native remote attach blocked for project user (D126); use `brn o --remote`",
            ));
        }
    }

    ensure_remote_server_running()?;

    let socket_path = crate::server::socket_paths::client_socket_path();
    let stream = crate::ipc::connect_local_stream(&socket_path).map_err(|err| {
        io::Error::new(
            err.kind(),
            format!(
                "failed to connect to remote Herdr client socket {}: {err}",
                socket_path.display()
            ),
        )
    })?;

    crate::platform::forward_remote_bridge_stdio(stream)
}

fn ensure_remote_server_running() -> io::Result<()> {
    let socket_path = crate::server::socket_paths::client_socket_path();
    if crate::server::autodetect::is_server_listening() {
        let status = crate::api::read_runtime_status_at(
            &crate::api::socket_path(),
            Duration::from_millis(500),
        )?
        .ok_or_else(|| io::Error::other("remote server status API is unavailable"))?;
        if status
            .capabilities
            .as_ref()
            .and_then(|capabilities| capabilities.endpoint_protocol_generation)
            == Some(crate::protocol::endpoint::ENDPOINT_PROTOCOL_GENERATION)
        {
            return Ok(());
        }
        return Err(io::Error::other(
            "remote herdr server needs one final update before this bridge can attach; rerun `herdr --remote` from an interactive terminal to approve it",
        ));
    }

    crate::server::autodetect::spawn_server_daemon()?;
    crate::server::autodetect::wait_for_server_socket(&socket_path, Duration::from_secs(5))
}

#[cfg(all(test, unix))]
mod tests {
    use super::native_remote_allowed_for;

    #[test]
    fn native_remote_blocked_for_project_users() {
        assert!(!native_remote_allowed_for("slyce"));
        assert!(!native_remote_allowed_for("seb"));
        assert!(!native_remote_allowed_for("alice"));
    }

    #[test]
    fn native_remote_allowed_for_herm_and_root() {
        assert!(native_remote_allowed_for("herm"));
        assert!(native_remote_allowed_for("root"));
    }
}
