//! The browser an interactive SSO login opens, when it is not the system's
//! default one (a corporate browser some identity providers require, a
//! dedicated profile, or none at all on a headless box).

use serde::{Deserialize, Serialize};

/// `program` value that opens nothing: the URL is only printed.
pub const NO_BROWSER: &str = "none";
/// `--browser` value that forgets the saved browser (back to the system one).
pub const DEFAULT_BROWSER: &str = "default";
/// Placeholder replaced by the authorization URL in [`BrowserConfig::args`].
pub const URL_PLACEHOLDER: &str = "{url}";

/// A browser to launch, saved per server.
///
/// `program` is run directly, never through a shell, so a path with spaces
/// needs no quoting and the URL cannot be interpreted as a command.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct BrowserConfig {
    /// Executable name (looked up in `PATH`) or full path, or `none`.
    pub program: String,
    /// Arguments; `{url}` is replaced by the authorization URL. Without a
    /// `{url}`, the URL is passed as the last argument.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
}

impl BrowserConfig {
    /// The arguments to launch with for `url`.
    #[must_use]
    pub fn command_args(&self, url: &str) -> Vec<String> {
        let mut has_placeholder = false;
        let mut args: Vec<String> = self
            .args
            .iter()
            .map(|arg| {
                if arg.contains(URL_PLACEHOLDER) {
                    has_placeholder = true;
                    arg.replace(URL_PLACEHOLDER, url)
                } else {
                    arg.clone()
                }
            })
            .collect();
        if !has_placeholder {
            args.push(url.to_string());
        }
        args
    }

    /// Whether this only prints the URL.
    #[must_use]
    pub fn opens_nothing(&self) -> bool {
        self.program.eq_ignore_ascii_case(NO_BROWSER)
    }
}

/// What `login --browser` asked for this run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserFlag {
    /// `--browser default`: forget the saved browser.
    Reset,
    /// `--browser <program> [--browser-arg <arg>]...`
    Use(BrowserConfig),
}

impl BrowserFlag {
    /// Build the flag from the command line; `None` when `--browser` is absent.
    #[must_use]
    pub fn from_args(program: Option<&str>, args: &[String]) -> Option<Self> {
        let program = program?.trim();
        if program.is_empty() || program.eq_ignore_ascii_case(DEFAULT_BROWSER) {
            return Some(BrowserFlag::Reset);
        }
        Some(BrowserFlag::Use(BrowserConfig {
            program: program.to_string(),
            args: args.to_vec(),
        }))
    }

    /// The value to save for the server.
    #[must_use]
    pub fn saved_value(&self) -> Option<BrowserConfig> {
        match self {
            BrowserFlag::Reset => None,
            BrowserFlag::Use(config) => Some(config.clone()),
        }
    }
}

/// The browser to use, in order: `--browser` for this run, then
/// `PROXYAUTH_BROWSER` (a program, no arguments), then the one saved for the
/// server. `None` is the system's default browser.
#[must_use]
pub fn effective_browser(
    flag: Option<&BrowserFlag>,
    env: Option<&str>,
    saved: Option<&BrowserConfig>,
) -> Option<BrowserConfig> {
    if let Some(flag) = flag {
        return flag.saved_value();
    }
    if let Some(program) = env.map(str::trim).filter(|program| !program.is_empty()) {
        return BrowserFlag::from_args(Some(program), &[]).and_then(|flag| flag.saved_value());
    }
    saved.cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn browser(program: &str, args: &[&str]) -> BrowserConfig {
        BrowserConfig {
            program: program.to_string(),
            args: args.iter().map(ToString::to_string).collect(),
        }
    }

    #[test]
    fn the_url_is_appended_unless_placed() {
        let url = "https://idp.example/authorize?a=1&b=2";
        assert_eq!(browser("corp-browser", &[]).command_args(url), [url]);
        assert_eq!(
            browser("chrome", &["--profile-directory=Work", "--new-window"]).command_args(url),
            ["--profile-directory=Work", "--new-window", url]
        );
        assert_eq!(
            browser("corp", &["--open={url}", "--kiosk"]).command_args(url),
            [format!("--open={url}").as_str(), "--kiosk"]
        );
    }

    #[test]
    fn the_flag_resets_uses_or_is_absent() {
        assert_eq!(BrowserFlag::from_args(None, &[]), None);
        assert_eq!(
            BrowserFlag::from_args(Some("default"), &[]),
            Some(BrowserFlag::Reset)
        );
        assert_eq!(
            BrowserFlag::from_args(Some(" DEFAULT "), &[]),
            Some(BrowserFlag::Reset)
        );
        assert_eq!(
            BrowserFlag::from_args(
                Some(r"C:\Program Files\Corp\browser.exe"),
                &["--sso".to_string()]
            ),
            Some(BrowserFlag::Use(browser(
                r"C:\Program Files\Corp\browser.exe",
                &["--sso"]
            )))
        );
        assert!(browser("None", &[]).opens_nothing());
        assert!(!browser("firefox", &[]).opens_nothing());
    }

    #[test]
    fn the_flag_wins_then_the_environment_then_the_saved_browser() {
        let saved = browser("saved-browser", &["--profile=work"]);
        let flag = BrowserFlag::Use(browser("flag-browser", &[]));

        assert_eq!(
            effective_browser(Some(&flag), Some("env-browser"), Some(&saved)),
            Some(browser("flag-browser", &[]))
        );
        assert_eq!(
            effective_browser(Some(&BrowserFlag::Reset), Some("env-browser"), Some(&saved)),
            None
        );
        assert_eq!(
            effective_browser(None, Some("env-browser"), Some(&saved)),
            Some(browser("env-browser", &[]))
        );
        assert_eq!(effective_browser(None, Some("default"), Some(&saved)), None);
        assert_eq!(
            effective_browser(None, Some("  "), Some(&saved)),
            Some(saved.clone())
        );
        assert_eq!(effective_browser(None, None, None), None);
    }

    #[test]
    fn a_browser_round_trips_through_the_config_file() {
        let yaml = serde_yaml_ng::to_string(&browser("corp", &[])).unwrap();
        assert!(!yaml.contains("args"), "empty args are not written: {yaml}");
        let parsed: BrowserConfig = serde_yaml_ng::from_str("program: corp\n").unwrap();
        assert_eq!(parsed, browser("corp", &[]));
    }
}
