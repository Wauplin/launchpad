//! Harness adapters. Each one owns its native protocol, discovery, monitors, markers and terminal commands; the
//! common contract and the registry live in `launchpad_core::harness`.

pub mod acp;
pub mod claude;
pub mod codex;
pub mod opencode;

use launchpad_core::harness::Registration;

/// Add an adapter here and declare its module above; no host dispatch needs to change.
pub const REGISTRATIONS: &[Registration] = &[
    claude::REGISTRATION,
    codex::REGISTRATION,
    opencode::REGISTRATION,
];

/// Makes the adapters known to the core. Call once at startup, before anything reads a harness.
pub fn install() {
    launchpad_core::harness::install(REGISTRATIONS);
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::PathBuf;

    /// A scratch folder for trace tests, removed on drop.
    pub struct Temp(pub PathBuf);
    impl Temp {
        pub fn new() -> Self {
            let p = std::env::temp_dir().join(format!("launchpad-trace-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        pub fn write(&self, name: &str, data: &[u8]) {
            let path = self.0.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, data).unwrap();
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
}

#[cfg(test)]
mod attribution_tests {
    /// Each harness's display name gives the one-time agent's attribution line.
    #[test]
    fn one_time_agents_are_named_by_harness() {
        let names: Vec<String> = super::REGISTRATIONS.iter().map(|r| launchpad_core::attribution::text(None, r.descriptor.name)).collect();
        let link = "via [Launchpad](https://github.com/Wauplin/launchpad)_";
        assert_eq!(names, [format!("_Written by Claude Code {link}"), format!("_Written by Codex {link}"), format!("_Written by OpenCode {link}")]);
    }
}
