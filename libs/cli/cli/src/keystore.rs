//! OS credential store used to keep server and cluster tokens.
//!
//! Wraps `keyring-core` and picks the platform store once, on first use:
//! - macOS: Keychain
//! - Windows: Credential Manager
//! - Linux: Secret Service (GNOME Keyring, KWallet, ...), falling back to the
//!   kernel keyutils keyring when no Secret Service is reachable (headless
//!   hosts, SSH sessions without a D-Bus session bus).
//! - other *nix: Secret Service
//!
//! Secrets are never written to a plaintext file: if no store can be
//! initialised, [`entry`] returns an error.

use std::sync::LazyLock;

use keyring_core::{Entry, Error, Result};
use tracing::{debug, warn};

static STORE_INIT: LazyLock<Result<()>> = LazyLock::new(init_default_store);

/// Build an entry for `(service, user)` in the platform credential store.
pub fn entry(service: &str, user: &str) -> Result<Entry> {
    if let Err(err) = &*STORE_INIT {
        debug!("Credential store unavailable: {}", err);
        return Err(Error::NoDefaultStore);
    }
    Entry::new(service, user)
}

#[cfg(target_os = "macos")]
fn init_default_store() -> Result<()> {
    keyring_core::set_default_store(apple_native_keyring_store::keychain::Store::new()?);
    Ok(())
}

#[cfg(target_os = "windows")]
fn init_default_store() -> Result<()> {
    keyring_core::set_default_store(windows_native_keyring_store::Store::new()?);
    Ok(())
}

#[cfg(target_os = "linux")]
fn init_default_store() -> Result<()> {
    match zbus_secret_service_keyring_store::Store::new() {
        Ok(store) => keyring_core::set_default_store(store),
        Err(err) => {
            warn!(
                "Secret Service unavailable ({}); falling back to the kernel keyutils keyring",
                err
            );
            keyring_core::set_default_store(linux_keyutils_keyring_store::Store::new()?);
        }
    }
    Ok(())
}

#[cfg(all(
    unix,
    not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "android",
        target_os = "linux"
    ))
))]
fn init_default_store() -> Result<()> {
    keyring_core::set_default_store(zbus_secret_service_keyring_store::Store::new()?);
    Ok(())
}

#[cfg(any(target_os = "ios", target_os = "android", not(any(unix, windows))))]
fn init_default_store() -> Result<()> {
    Err(Error::NotSupportedByStore(
        "no credential store available on this platform".to_string(),
    ))
}
