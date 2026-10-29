//! Paired devices: other computers on the user's Woozlit account that run
//! Keplorayx with "Use this computer as a machine" on (or `dove node serve`
//! headless). Agents reach them through the account's device relay on the
//! Cloudflare edge, so neither side opens a port.
//!
//! The relay client lives in the `dove-mesh-host` sidecar (mesh workspace);
//! this module only builds invocations of it and reads its device list.

use crate::host::Invocation;

/// A paired device as the chip and the Machines page show it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub online: bool,
}

/// The local command that runs `argv` on `device` with live stdio.
pub fn exec_invocation(device: &str, argv: &[String]) -> Invocation {
    let _ = (device, argv);
    Invocation { program: "false".into(), args: Vec::new(), local_cwd: None }
}

/// Devices paired with this computer. Empty until pairing is built.
pub fn devices() -> Vec<Device> {
    Vec::new()
}

/// A device's display name, falling back to its id.
pub fn name(device: &str) -> String {
    devices().into_iter().find(|d| d.id == device).map(|d| d.name).unwrap_or_else(|| device.to_string())
}

/// Whether this build can pair devices (the relay client is present).
pub fn pairing_available() -> bool {
    false
}
