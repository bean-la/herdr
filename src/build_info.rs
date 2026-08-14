//! Build identity helpers.

pub const BASE_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn channel() -> &'static str {
    non_empty(option_env!("HERDR_BUILD_CHANNEL")).unwrap_or("stable")
}

pub fn build_id() -> Option<&'static str> {
    non_empty(option_env!("HERDR_BUILD_ID"))
}

pub fn version() -> String {
    match channel() {
        "stable" => match build_id() {
            Some(build_id) => format!("{BASE_VERSION}-{build_id}"),
            None => BASE_VERSION.to_string(),
        },
        channel => match build_id() {
            Some(build_id) => format!("{BASE_VERSION}-{channel}.{build_id}"),
            None => format!("{BASE_VERSION}-{channel}"),
        },
    }
}

pub fn is_preview() -> bool {
    channel() == "preview"
}

fn non_empty(value: Option<&'static str>) -> Option<&'static str> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn stable_version_defaults_to_cargo_version() {
        assert!(!super::version().is_empty());
    }

    #[test]
    fn stable_version_with_build_id_appends_stamp() {
        // Version must surface the build stamp on the stable channel too,
        // otherwise `status restart_needed` can't tell a rebuilt-but-not-
        // restarted server from a current one (2026-08-14).
        let v = super::version();
        assert!(v.starts_with(super::BASE_VERSION));
        // When HERDR_BUILD_ID is set at compile time the stamp must appear in
        // the reported version string (else restart_needed stays blind).
        if let Some(build_id) = super::build_id() {
            assert!(v.contains(build_id), "version {v:?} missing build_id {build_id:?}");
        }
    }
}
