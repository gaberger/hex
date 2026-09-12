//! Who serves inference on this machine, named in exactly one place.
//!
//! Founding goal G1's test is that **zero non-test files outside this crate
//! name a provider or a model**. `hex bootstrap` and `hex doctor composition`
//! both broke it, and not trivially: between them they hardcoded the
//! provider's display name, its CLI binary, its process name, its port, two
//! install commands, and a list of three model ids.
//!
//! The model list was the expensive part. It had drifted: `hex bootstrap
//! validate` checked for `gemma4:latest` and `qwen2.5-coder:32b` while
//! `.hex/project.json` configured `gemma4-12b` and `devstral-small-2:24b`. So
//! bootstrap reported a healthy install after verifying three models the
//! project does not use, and said nothing about the three it does. A check
//! that validates the wrong subject passes for the wrong reason, which looks
//! exactly like passing.
//!
//! [`configured_tiers`] fixes that by reading the same config the dispatcher
//! reads. There is no hardcoded model list here either — a default model in
//! this file would be a model the operator never chose.

/// The local inference server hex bootstraps against.
///
/// One value, because this is a single-binary tool for one machine. When hex
/// needs to support a second local server, this becomes an enum and every
/// caller below keeps compiling — which is the point of naming it here rather
/// than spelling it out in six files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalProvider {
    /// For display: "Ollama".
    pub display_name: &'static str,
    /// The executable on `PATH`, and the process name to match.
    pub binary: &'static str,
    /// The port it listens on by default.
    pub default_port: u16,
    /// The subcommand that starts it in the foreground.
    pub serve_arg: &'static str,
    /// How to install it, per platform.
    pub install_macos: &'static str,
    pub install_linux: &'static str,
    /// The environment variable that overrides where it listens.
    pub host_env: &'static str,
}

/// The local provider hex targets.
pub const fn local_provider() -> LocalProvider {
    LocalProvider {
        display_name: "Ollama",
        binary: "ollama",
        default_port: 11434,
        serve_arg: "serve",
        install_macos: "brew install ollama",
        install_linux: "curl https://ollama.ai/install.sh | sh",
        host_env: "OLLAMA_HOST",
    }
}

impl LocalProvider {
    /// `http://127.0.0.1:<port>` — the base URL for a default install.
    pub fn default_base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.default_port)
    }

    /// Where the server actually is: the host environment variable if set,
    /// otherwise the default.
    ///
    /// **This exists because the same variable was read three ways.**
    /// `resource_governor.rs` treated it as `host:port` and prepended a
    /// scheme; `doctor/composition.rs` treated it as a full URL and defaulted
    /// to `http://localhost:11434`; `complete.rs` defaulted to
    /// `http://127.0.0.1:11434`. Set `OLLAMA_HOST=127.0.0.1:11434`, as the
    /// server's own documentation tells you to, and one of those three built
    /// `http://http://…`. Normalising once is the only way that stays fixed.
    pub fn base_url(&self) -> String {
        match std::env::var(self.host_env) {
            Ok(h) if !h.trim().is_empty() => {
                let h = h.trim();
                if h.starts_with("http://") || h.starts_with("https://") {
                    h.to_string()
                } else {
                    format!("http://{h}")
                }
            }
            _ => self.default_base_url(),
        }
    }

    /// `host:port` for a TCP reachability probe, from the same resolved URL.
    ///
    /// Callers used to build `format!("127.0.0.1:{port}")` themselves, which
    /// meant a probe against localhost even when the operator had pointed the
    /// server at another machine — reporting "not running" for a server that
    /// was running fine.
    pub fn socket_addr(&self) -> String {
        let url = self.base_url();
        let rest = url.strip_prefix("http://").or_else(|| url.strip_prefix("https://")).unwrap_or(&url);
        let host_port = rest.split('/').next().unwrap_or(rest);
        if host_port.contains(':') {
            host_port.to_string()
        } else {
            format!("{host_port}:{}", self.default_port)
        }
    }

    /// The install hint for the platform this binary was built for.
    pub fn install_hint(&self) -> &'static str {
        if cfg!(target_os = "macos") {
            self.install_macos
        } else {
            self.install_linux
        }
    }
}

/// The tiers this project configures, as `(label, tier key, model id)`.
///
/// Empty when `.hex/project.json` declares none — which is a real answer, and
/// a caller should say so rather than substitute a guess.
pub fn configured_tiers() -> Vec<(&'static str, &'static str, String)> {
    [("T1", "t1"), ("T2", "t2"), ("T2.5", "t2.5")]
        .into_iter()
        .filter_map(|(label, key)| crate::tiers::tier_model(key).map(|m| (label, key, m)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_base_url_is_built_from_the_declared_port() {
        let p = local_provider();
        assert_eq!(p.default_base_url(), format!("http://127.0.0.1:{}", p.default_port));
    }

    /// Both spellings of the override must produce one well-formed URL.
    #[test]
    fn the_host_override_is_normalised_either_way() {
        let p = local_provider();
        let prev = std::env::var(p.host_env).ok();
        for (set, want) in [
            ("127.0.0.1:9999", "http://127.0.0.1:9999"),
            ("http://box:9999", "http://box:9999"),
            ("  ", ""), // blank falls back to the default
        ] {
            std::env::set_var(p.host_env, set);
            let got = p.base_url();
            let expected = if want.is_empty() { p.default_base_url() } else { want.to_string() };
            assert_eq!(got, expected, "OLLAMA_HOST={set:?}");
            assert!(!got.contains("http://http"), "double scheme from {set:?}");
        }
        match prev {
            Some(v) => std::env::set_var(p.host_env, v),
            None => std::env::remove_var(p.host_env),
        }
    }

    #[test]
    fn the_probe_target_follows_the_host_override() {
        let p = local_provider();
        let prev = std::env::var(p.host_env).ok();
        std::env::set_var(p.host_env, "http://gpu-box:9999/");
        assert_eq!(p.socket_addr(), "gpu-box:9999");
        std::env::set_var(p.host_env, "gpu-box");
        assert_eq!(p.socket_addr(), format!("gpu-box:{}", p.default_port));
        match prev {
            Some(v) => std::env::set_var(p.host_env, v),
            None => std::env::remove_var(p.host_env),
        }
    }

    #[test]
    fn the_install_hint_is_never_empty() {
        assert!(!local_provider().install_hint().is_empty());
    }

    /// `configured_tiers` must return what the project declares and nothing
    /// else. A hardcoded fallback here would put a model id back into the
    /// code path this module exists to clear.
    #[test]
    fn no_config_means_no_tiers_rather_than_a_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        // tier_model resolves through HEX_PROJECT_ROOT; point it at an empty
        // directory and the answer must be "none", not a guess.
        let prev = std::env::var("HEX_PROJECT_ROOT").ok();
        std::env::set_var("HEX_PROJECT_ROOT", dir.path());
        let got = configured_tiers();
        match prev {
            Some(p) => std::env::set_var("HEX_PROJECT_ROOT", p),
            None => std::env::remove_var("HEX_PROJECT_ROOT"),
        }
        assert!(got.is_empty(), "an unconfigured project produced tiers: {got:?}");
    }
}
