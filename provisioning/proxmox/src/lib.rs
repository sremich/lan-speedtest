//! Idempotent provisioning for the speed test guest.
//!
//! Exposed as a library so the flow can be tested against a scripted fake
//! hypervisor rather than a real one — see `tests/idempotency.rs`.

pub mod config;
pub mod proxmox;
pub mod runner;
pub mod setup;
