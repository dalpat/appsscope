//! D-Bus proxies for PackageKit.
//!
//! PackageKit's model is: ask the daemon for a transaction object, subscribe
//! to its signals, call one method on it, then read results off the signal
//! stream until `Finished`. A transaction is single-use.
//!
//! Critically, the daemon enforces that the connection driving a transaction
//! is the same one that created it — a different connection gets
//! `RefusedByPolicy: sender does not match`. That is why [`super::connection`]
//! caches one connection for the whole process rather than opening one per
//! call.

use std::collections::HashMap;

use zbus::zvariant::OwnedValue;

#[zbus::proxy(
    interface = "org.freedesktop.PackageKit",
    default_service = "org.freedesktop.PackageKit",
    default_path = "/org/freedesktop/PackageKit"
)]
pub trait PackageKit {
    /// Allocate a transaction object path.
    fn create_transaction(&self) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;

    #[zbus(property)]
    fn version_major(&self) -> zbus::Result<u32>;
}

#[zbus::proxy(
    interface = "org.freedesktop.PackageKit.Transaction",
    default_service = "org.freedesktop.PackageKit"
)]
pub trait Transaction {
    /// Hints such as `interactive=false`; also how the frontend declares that
    /// it can present its own progress.
    fn set_hints(&self, hints: &[&str]) -> zbus::Result<()>;

    fn get_packages(&self, filter: u64) -> zbus::Result<()>;
    fn get_updates(&self, filter: u64) -> zbus::Result<()>;
    fn resolve(&self, filter: u64, packages: &[&str]) -> zbus::Result<()>;
    fn get_details(&self, package_ids: &[&str]) -> zbus::Result<()>;

    fn install_packages(&self, flags: u64, package_ids: &[&str]) -> zbus::Result<()>;
    fn remove_packages(
        &self,
        flags: u64,
        package_ids: &[&str],
        allow_deps: bool,
        autoremove: bool,
    ) -> zbus::Result<()>;
    fn update_packages(&self, flags: u64, package_ids: &[&str]) -> zbus::Result<()>;

    /// One result row. `info` is a `PkInfoEnum`.
    #[zbus(signal)]
    fn package(&self, info: u32, package_id: String, summary: String) -> zbus::Result<()>;

    /// Detail row from `get_details`, as a loosely-typed dictionary.
    #[zbus(signal)]
    fn details(&self, data: HashMap<String, OwnedValue>) -> zbus::Result<()>;

    /// Terminal signal. `exit` is a `PkExitEnum`; 1 means success.
    #[zbus(signal)]
    fn finished(&self, exit: u32, runtime: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    fn error_code(&self, code: u32, details: String) -> zbus::Result<()>;

    #[zbus(signal)]
    fn item_progress(&self, package_id: String, status: u32, percentage: u32) -> zbus::Result<()>;

    #[zbus(property)]
    fn percentage(&self) -> zbus::Result<u32>;
}

/// Filter bitfield. PackageKit packs `PkFilterEnum` values as `1 << value`.
///
/// The full set is listed even where unused, because deriving these by hand
/// from `pk-enum.h` is exactly the kind of thing worth writing down once.
#[allow(dead_code)]
pub mod filter {
    pub const INSTALLED: u64 = 1 << 2;
    pub const NOT_INSTALLED: u64 = 1 << 3;
    pub const GUI: u64 = 1 << 6;
    pub const NEWEST: u64 = 1 << 16;
    pub const APPLICATION: u64 = 1 << 24;
}

/// Transaction flags, same `1 << value` packing.
pub mod flags {
    pub const NONE: u64 = 0;
    pub const ONLY_TRUSTED: u64 = 1 << 1;
    /// Resolve and report what *would* happen, changing nothing.
    pub const SIMULATE: u64 = 1 << 2;
}

/// `PkInfoEnum` values we care about.
#[allow(dead_code)]
pub mod info {
    pub const INSTALLED: u32 = 1;
    pub const AVAILABLE: u32 = 2;
    pub const UPDATING: u32 = 9;
    pub const INSTALLING: u32 = 8;
    pub const REMOVING: u32 = 7;
    pub const DOWNLOADING: u32 = 10;
}

/// `PkExitEnum::Success`.
pub const EXIT_SUCCESS: u32 = 1;

/// Split a PackageKit package id: `name;version;arch;data`.
pub fn split_package_id(id: &str) -> (&str, &str, &str, &str) {
    let mut parts = id.split(';');
    (
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
    )
}
