//! The grounder as the driver uses it: off unless the weights exist, loaded
//! on first use, dropped again after a quiet spell.
//!
//! `DOVE_DRIVER_GROUNDER_MODEL` names the ONNX asset. Unset, or pointing at
//! a missing file, the feature is off and [`Grounder::from_env`] returns
//! None. `DOVE_DRIVER_GROUNDER_IDLE_SECS` (default 300) is how long the
//! loaded network may sit unused before it is unloaded.

use crate::infer::{self, Calibration};
use crate::model::Model;
use crate::types::{Decision, Observation};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

pub const MODEL_ENV: &str = "DOVE_DRIVER_GROUNDER_MODEL";
pub const IDLE_ENV: &str = "DOVE_DRIVER_GROUNDER_IDLE_SECS";
const DEFAULT_IDLE: Duration = Duration::from_secs(300);

struct Loaded {
    model: Arc<Model>,
    last_used: Instant,
}

/// Where the network comes from: a file, or bytes built into the binary.
#[derive(Clone, Debug)]
enum Source {
    File(PathBuf),
    Bytes(&'static [u8]),
}

pub struct Grounder {
    source: Source,
    idle: Duration,
    loaded: Mutex<Option<Loaded>>,
    reaper_running: Mutex<bool>,
}

impl Grounder {
    /// The process-wide grounder, or None when the weights are absent.
    pub fn from_env() -> Option<Arc<Grounder>> {
        static SHARED: OnceLock<Option<Arc<Grounder>>> = OnceLock::new();
        SHARED
            .get_or_init(|| {
                let path = PathBuf::from(std::env::var_os(MODEL_ENV)?);
                if !path.is_file() {
                    return None;
                }
                let idle = std::env::var(IDLE_ENV)
                    .ok()
                    .and_then(|s| s.trim().parse::<u64>().ok())
                    .map(Duration::from_secs)
                    .unwrap_or(DEFAULT_IDLE);
                Some(Arc::new(Grounder::new(path, idle)))
            })
            .clone()
    }

    pub fn new(path: PathBuf, idle: Duration) -> Self {
        Self::with_source(Source::File(path), idle)
    }

    /// A grounder over an asset built into the binary.
    pub fn from_bytes(bytes: &'static [u8], idle: Duration) -> Self {
        Self::with_source(Source::Bytes(bytes), idle)
    }

    fn with_source(source: Source, idle: Duration) -> Self {
        Self {
            source,
            idle,
            loaded: Mutex::new(None),
            reaper_running: Mutex::new(false),
        }
    }

    /// The asset's file, when it is one.
    pub fn path(&self) -> Option<&Path> {
        match &self.source {
            Source::File(p) => Some(p),
            Source::Bytes(_) => None,
        }
    }

    /// Whether the network is in memory now.
    pub fn is_loaded(&self) -> bool {
        self.loaded
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }

    /// Drop the network now (it reloads on the next decision).
    pub fn unload(&self) {
        *self.loaded.lock().unwrap_or_else(|e| e.into_inner()) = None;
        crate::model::release_free_memory();
    }

    fn model(self: &Arc<Self>) -> Result<Arc<Model>, String> {
        let mut slot = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(loaded) = slot.as_mut() {
            loaded.last_used = Instant::now();
            return Ok(loaded.model.clone());
        }
        let loaded = match &self.source {
            Source::File(path) => {
                Model::load(path).map_err(|e| format!("loading {}: {e:#}", path.display()))
            }
            Source::Bytes(bytes) => Model::read(&mut std::io::Cursor::new(*bytes))
                .map_err(|e| format!("loading the built-in NEXI-CUA-1: {e:#}")),
        };
        let model = Arc::new(loaded?);
        *slot = Some(Loaded {
            model: model.clone(),
            last_used: Instant::now(),
        });
        drop(slot);
        self.start_reaper();
        Ok(model)
    }

    /// One thread per load: it ends once the network is unloaded.
    fn start_reaper(self: &Arc<Self>) {
        let mut running = self
            .reaper_running
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if *running {
            return;
        }
        *running = true;
        let weak: Weak<Self> = Arc::downgrade(self);
        let tick = (self.idle / 4).clamp(Duration::from_millis(50), Duration::from_secs(30));
        let spawned = std::thread::Builder::new()
            .name("dove-grounder-idle".into())
            .spawn(move || loop {
                std::thread::sleep(tick);
                let Some(this) = weak.upgrade() else { return };
                let mut slot = this.loaded.lock().unwrap_or_else(|e| e.into_inner());
                let expired = slot
                    .as_ref()
                    .is_none_or(|loaded| loaded.last_used.elapsed() >= this.idle);
                if expired {
                    *slot = None;
                    drop(slot);
                    crate::model::release_free_memory();
                    *this
                        .reaper_running
                        .lock()
                        .unwrap_or_else(|e| e.into_inner()) = false;
                    return;
                }
            });
        if spawned.is_err() {
            *running = false;
        }
    }

    /// Decide one goal: resolver first, then NEXI-CUA-1. A load failure
    /// is an abstention whose reason says why.
    pub fn decide(self: &Arc<Self>, obs: &Observation, goal: &str) -> Decision {
        match self.model() {
            Ok(model) => infer::decide(Some(&model), model.calibration, obs, goal, true),
            Err(error) => {
                let mut decision = infer::decide(None, Calibration::NONE, obs, goal, true);
                if decision.abstain {
                    decision.reason = error;
                }
                decision
            }
        }
    }
}

/// Decide `goal` with the deterministic resolver alone (no network): exact
/// names, ordinals, key chords. It abstains whenever the resolver defers.
pub fn decide_resolver_only(obs: &Observation, goal: &str) -> Decision {
    infer::decide(None, Calibration::NONE, obs, goal, true)
}

/// NEXI-CUA-1 as built into this binary (`assets/nexi-cua-1.onnx`).
#[cfg(feature = "embedded-model")]
pub const EMBEDDED_MODEL: &[u8] = include_bytes!("../assets/nexi-cua-1.onnx");

/// The process-wide grounder over the built-in network (loaded on first
/// decision, unloaded after `idle` without one).
#[cfg(feature = "embedded-model")]
pub fn embedded(idle: Duration) -> Arc<Grounder> {
    static SHARED: OnceLock<Arc<Grounder>> = OnceLock::new();
    SHARED.get_or_init(|| Arc::new(Grounder::from_bytes(EMBEDDED_MODEL, idle))).clone()
}
