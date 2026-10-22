// SPDX-License-Identifier: GPL-3.0-only

//! Non-secret user preferences. Never store secrets, labels, attributes or
//! search text here.

use cosmic::cosmic_config::{self, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry};

#[derive(Debug, Clone, CosmicConfigEntry, Eq, PartialEq)]
#[version = 1]
pub struct Config {
    /// How long a revealed secret stays visible, in seconds.
    pub reveal_seconds: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self { reveal_seconds: 30 }
    }
}
