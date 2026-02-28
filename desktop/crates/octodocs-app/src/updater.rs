//! Background update checker.
//! On startup AppState spawns `check_for_update()` on the background executor.
//! If a newer release is found the result is stored in `AppState::update_available`
//! and a banner appears in the UI.

use reqwest::blocking::Client;

/// The version compiled into this binary (from Cargo.toml).
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const REPO: &str = "elranu/octodocs";

/// Hit the GitHub releases API and return the latest tag if it is newer than
/// the current binary.  Returns `None` on any error or when already up-to-date.
pub fn check_for_update() -> Option<String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let client = Client::builder()
        .user_agent(format!("octodocs-app/{CURRENT_VERSION}"))
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .ok()?;

    let text = client.get(&url).send().ok()?.text().ok()?;
    let tag = extract_tag_name(&text)?;
    let latest = tag.trim_start_matches('v');
    let current = CURRENT_VERSION.trim_start_matches('v');

    if version_is_newer(latest, current) { Some(tag) } else { None }
}

/// Spawn the platform-appropriate update process.
///
/// - Linux / macOS: re-runs `install.sh` via the system shell in the background.
/// - Windows: downloads the Inno Setup installer to `%TEMP%` and launches it
///   with `/verysilent`, then the installer replaces the running binary and
///   relaunches the app.  The caller should quit after this returns `Ok`.
pub fn launch_update(tag: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "windows")]
    {
        let url = format!(
            "https://github.com/{REPO}/releases/download/{tag}/OctoDocs-Setup-x86_64.exe"
        );
        let client = Client::builder()
            .user_agent(format!("octodocs-app/{CURRENT_VERSION}"))
            .build()?;

        let bytes = client.get(&url).send()?.bytes()?;
        let tmp = std::env::temp_dir().join("octodocs-update-installer.exe");
        std::fs::write(&tmp, &bytes)?;

        std::process::Command::new(&tmp)
            .arg("/verysilent")
            .arg("/update=true")
            .spawn()?;
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = tag; // unused on Unix
        let install_url =
            "https://raw.githubusercontent.com/elranu/octodocs/main/install.sh";
        std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("curl -fsSL '{install_url}' | sh"))
            .spawn()?;
        Ok(())
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn extract_tag_name(json: &str) -> Option<String> {
    let key = "\"tag_name\"";
    let pos = json.find(key)?;
    let after = &json[pos + key.len()..];
    let start = after.find('"')? + 1;
    let rest = &after[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Returns `true` when `candidate` is strictly greater than `current`
/// using simple three-part numeric comparison (e.g. "0.1.8" > "0.1.7").
fn version_is_newer(candidate: &str, current: &str) -> bool {
    let parse = |v: &str| -> (u64, u64, u64) {
        let mut p = v.splitn(3, '.').map(|s| s.parse::<u64>().unwrap_or(0));
        (p.next().unwrap_or(0), p.next().unwrap_or(0), p.next().unwrap_or(0))
    };
    parse(candidate) > parse(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_patch() {
        assert!(version_is_newer("0.1.8", "0.1.7"));
    }

    #[test]
    fn same_version() {
        assert!(!version_is_newer("0.1.7", "0.1.7"));
    }

    #[test]
    fn older_version() {
        assert!(!version_is_newer("0.1.6", "0.1.7"));
    }

    #[test]
    fn newer_minor() {
        assert!(version_is_newer("0.2.0", "0.1.99"));
    }

    #[test]
    fn extract_tag() {
        let json = r#"{"tag_name": "v0.1.8","name":"v0.1.8"}"#;
        assert_eq!(extract_tag_name(json).as_deref(), Some("v0.1.8"));
    }
}
