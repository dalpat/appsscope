//! Backend-agnostic core for AppsScope.
//!
//! This crate defines the vocabulary the rest of the app speaks: the [`Backend`]
//! trait each package system implements, and the domain types they all map onto.
//! It has no GTK dependency and no I/O of its own, so it stays testable and
//! cheap to compile.

pub mod backend;
pub mod error;
pub mod plan;
pub mod sandbox;
pub mod types;

pub use backend::Backend;
pub use error::{Error, Result};
pub use plan::{InstallPlan, PlanItem, format_size};
pub use sandbox::{Permission, PermissionKind, Risk, Sandbox};
pub use types::{
    App, AppRef, BackendId, IconSource, InstalledApp, Phase, Progress, Release, Screenshot, Update,
};
