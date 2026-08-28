use std::io;
use std::path::Path;

use crate::config::{ServerConfig, SocketAccessMode};

/// Resolved socket permission policy for API/client/handoff binds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SocketAccessPolicy {
    User,
    Group { name: String, gid: u32 },
}

impl SocketAccessPolicy {
    pub(crate) fn mode_bits(&self) -> u32 {
        match self {
            Self::User => SocketAccessMode::User.mode_bits(),
            Self::Group { .. } => SocketAccessMode::Group.mode_bits(),
        }
    }

    pub(crate) fn describe(&self) -> String {
        match self {
            Self::User => format!("user ({:04o})", self.mode_bits()),
            Self::Group { name, .. } => {
                format!("group ({:04o}, group={name})", self.mode_bits())
            }
        }
    }
}

pub(crate) fn policy_from_server_config(
    config: &ServerConfig,
) -> Result<SocketAccessPolicy, String> {
    match config.socket_access_mode()? {
        SocketAccessMode::User => Ok(SocketAccessPolicy::User),
        SocketAccessMode::Group => {
            let name = config.socket_group_name().ok_or_else(|| {
                "server.socket_access = \"group\" requires server.socket_group".to_string()
            })?;
            #[cfg(not(unix))]
            {
                let _ = name;
                Err("server.socket_access = \"group\" is only supported on Unix".to_string())
            }
            #[cfg(unix)]
            {
                let gid = lookup_group_gid(name).map_err(|err| {
                    format!("server.socket_group {name:?} could not be resolved: {err}")
                })?;
                Ok(SocketAccessPolicy::Group {
                    name: name.to_string(),
                    gid,
                })
            }
        }
    }
}

pub(crate) fn policy_from_loaded_config() -> io::Result<SocketAccessPolicy> {
    policy_from_server_config(&crate::config::Config::load().config.server)
        .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))
}

pub(crate) fn apply_configured_socket_access(path: &Path) -> io::Result<()> {
    apply_socket_access(path, &policy_from_loaded_config()?)
}

pub(crate) fn apply_socket_access(path: &Path, policy: &SocketAccessPolicy) -> io::Result<()> {
    crate::ipc::restrict_socket_permissions(path, policy.mode_bits())?;
    #[cfg(unix)]
    if let SocketAccessPolicy::Group { gid, name } = policy {
        if let Err(err) = std::os::unix::fs::chown(path, None, Some(*gid)) {
            return Err(io::Error::other(format!(
                "failed to set socket group {name:?} on {}: {err}",
                path.display()
            )));
        }
        crate::ipc::restrict_socket_permissions(path, policy.mode_bits())?;
    }
    Ok(())
}

pub(crate) fn describe_configured_socket_access() -> String {
    let server = crate::config::Config::load().config.server;
    match policy_from_server_config(&server) {
        Ok(policy) => policy.describe(),
        Err(err) => format!("invalid ({err})"),
    }
}

#[cfg(unix)]
fn lookup_group_gid(name: &str) -> io::Result<u32> {
    use std::ffi::CString;

    let c_name = CString::new(name).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "server.socket_group contains interior NUL",
        )
    })?;
    let mut grp = unsafe { std::mem::zeroed::<libc::group>() };
    let mut result: *mut libc::group = std::ptr::null_mut();
    let mut buf = vec![0u8; 1024];
    loop {
        let rc = unsafe {
            libc::getgrnam_r(
                c_name.as_ptr(),
                &mut grp,
                buf.as_mut_ptr().cast::<libc::c_char>(),
                buf.len(),
                &mut result,
            )
        };
        if rc == libc::ERANGE {
            buf.resize(buf.len().saturating_mul(2).max(2048), 0);
            continue;
        }
        if rc != 0 {
            return Err(io::Error::from_raw_os_error(rc));
        }
        if result.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("group {name:?} does not exist"),
            ));
        }
        return Ok(grp.gr_gid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ServerConfig;

    #[test]
    fn user_mode_is_the_default_policy() {
        let policy = policy_from_server_config(&ServerConfig::default()).unwrap();
        assert_eq!(policy, SocketAccessPolicy::User);
        assert_eq!(policy.mode_bits(), 0o600);
        assert_eq!(policy.describe(), "user (0600)");
    }

    #[test]
    fn group_mode_without_socket_group_fails() {
        let config = ServerConfig {
            socket_access: "group".into(),
            socket_group: String::new(),
            ..ServerConfig::default()
        };
        let err = policy_from_server_config(&config).unwrap_err();
        assert!(err.contains("socket_group"), "{err}");
    }

    #[test]
    fn unknown_socket_access_fails() {
        let config = ServerConfig {
            socket_access: "world".into(),
            ..ServerConfig::default()
        };
        let err = policy_from_server_config(&config).unwrap_err();
        assert!(err.contains("user"), "{err}");
        assert!(err.contains("group"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn missing_group_name_fails_to_resolve() {
        let config = ServerConfig {
            socket_access: "group".into(),
            socket_group: "herdr-missing-group-name-xyz".into(),
            ..ServerConfig::default()
        };
        let err = policy_from_server_config(&config).unwrap_err();
        assert!(err.contains("herdr-missing-group-name-xyz"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn user_policy_applies_owner_only_mode() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::net::UnixListener;

        let path = unique_test_socket("sau");
        let _ = fs::remove_file(&path);
        let _listener = UnixListener::bind(&path).unwrap();
        apply_socket_access(&path, &SocketAccessPolicy::User).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        drop(_listener);
        let _ = fs::remove_file(&path);
    }

    #[cfg(unix)]
    #[test]
    fn group_policy_applies_group_mode_and_gid() {
        use std::fs;
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        use std::os::unix::net::UnixListener;

        let gid = unsafe { libc::getgid() };
        let name = current_group_name(gid).unwrap_or_else(|| "current".into());
        let path = unique_test_socket("sag");
        let _ = fs::remove_file(&path);
        let _listener = UnixListener::bind(&path).unwrap();
        apply_socket_access(
            &path,
            &SocketAccessPolicy::Group {
                name: name.clone(),
                gid,
            },
        )
        .unwrap();
        let meta = fs::metadata(&path).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o660);
        assert_eq!(meta.gid(), gid);
        drop(_listener);
        let _ = fs::remove_file(&path);
    }

    #[cfg(unix)]
    #[test]
    fn group_mode_resolves_current_group() {
        let gid = unsafe { libc::getgid() };
        let Some(name) = current_group_name(gid) else {
            return;
        };
        let config = ServerConfig {
            socket_access: "group".into(),
            socket_group: name.clone(),
            ..ServerConfig::default()
        };
        assert_eq!(
            policy_from_server_config(&config).unwrap(),
            SocketAccessPolicy::Group { name, gid }
        );
    }

    #[cfg(unix)]
    fn current_group_name(gid: u32) -> Option<String> {
        let mut grp = unsafe { std::mem::zeroed::<libc::group>() };
        let mut result: *mut libc::group = std::ptr::null_mut();
        let mut buf = vec![0u8; 1024];
        loop {
            let rc = unsafe {
                libc::getgrgid_r(
                    gid,
                    &mut grp,
                    buf.as_mut_ptr().cast::<libc::c_char>(),
                    buf.len(),
                    &mut result,
                )
            };
            if rc == libc::ERANGE {
                buf.resize(buf.len().saturating_mul(2).max(2048), 0);
                continue;
            }
            if rc != 0 || result.is_null() {
                return None;
            }
            let name = unsafe { std::ffi::CStr::from_ptr(grp.gr_name) };
            return Some(name.to_string_lossy().into_owned());
        }
    }

    #[cfg(unix)]
    fn unique_test_socket(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(format!(
            "/tmp/hsa-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ))
    }
}
