// Copyright 2023 The AccessKit Authors. All rights reserved.
// Licensed under the Apache License, Version 2.0 (found in
// the LICENSE-APACHE file) or the MIT license (found in
// the LICENSE-MIT file), at your option.

use accesskit::{ActionHandler, ActionRequest, Point};
use accesskit_consumer::{NodeId, Tree};
use hashbrown::HashMap;
use std::fmt::{Debug, Formatter};
use std::sync::{
    Arc, Mutex, RwLock, RwLockReadGuard,
    atomic::{AtomicBool, Ordering},
};
use windows::Win32::{
    Foundation::{LPARAM, WPARAM},
    UI::{
        Accessibility::{UIA_AutomationFocusChangedEventId, UIA_EVENT_ID, UIA_PROPERTY_ID},
        WindowsAndMessaging::{PostMessageW, RegisterWindowMessageW},
    },
};
use windows::core::{ComObject, w};

use crate::{node::PlatformNode, util::*, window_handle::WindowHandle};

pub(crate) trait ActionHandlerNoMut {
    fn do_action(&self, request: ActionRequest);
}

pub(crate) struct ActionHandlerWrapper<H: ActionHandler + Send>(Mutex<H>);

impl<H: 'static + ActionHandler + Send> ActionHandlerWrapper<H> {
    pub(crate) fn new(inner: H) -> Self {
        Self(Mutex::new(inner))
    }
}

impl<H: ActionHandler + Send> ActionHandlerNoMut for ActionHandlerWrapper<H> {
    fn do_action(&self, request: ActionRequest) {
        self.0.lock().unwrap().do_action(request)
    }
}

pub(crate) struct Context {
    pub(crate) hwnd: WindowHandle,
    pub(crate) tree: RwLock<Tree>,
    pub(crate) action_handler: Arc<dyn ActionHandlerNoMut + Send + Sync>,
    pub(crate) is_placeholder: AtomicBool,
    /// zgx patch: the activation handler was asked for the tree (a
    /// placeholder for an MSAA probe is served without asking).
    pub(crate) tree_requested: AtomicBool,
    /// zgx patch: a client read past the placeholder, and the window was
    /// sent `activation_message` to ask for the tree.
    activation_posted: AtomicBool,
    platform_nodes: Mutex<HashMap<NodeId, ComObject<PlatformNode>>>,
    /// zgx patch: what UI Automation clients listen for, as UIA tells the
    /// fragment root (`IRawElementProviderAdviseEvents`): each event id, and
    /// for property-changed handlers each property id, with its count of
    /// handlers.
    subscriptions: Mutex<HashMap<i32, usize>>,
}

impl Debug for Context {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Context")
            .field("hwnd", &self.hwnd)
            .field("tree", &self.tree)
            .field("action_handler", &"ActionHandler")
            .field("is_placeholder", &self.is_placeholder)
            .finish()
    }
}

impl Context {
    pub(crate) fn new(
        hwnd: WindowHandle,
        tree: Tree,
        action_handler: Arc<dyn ActionHandlerNoMut + Send + Sync>,
        is_placeholder: bool,
    ) -> Arc<Self> {
        Arc::new(Self {
            hwnd,
            tree: RwLock::new(tree),
            action_handler,
            is_placeholder: AtomicBool::new(is_placeholder),
            tree_requested: AtomicBool::new(!is_placeholder),
            activation_posted: AtomicBool::new(false),
            platform_nodes: Mutex::new(HashMap::new()),
            subscriptions: Mutex::new(HashMap::new()),
        })
    }

    /// zgx patch: a client read past the placeholder root (`what`: its
    /// children, focus, a pattern, more than its basic properties). Unless
    /// the tree was asked for already, the window is sent
    /// `activation_message`, whose handling (`Adapter::activate`, on the
    /// window's thread) asks for it. This read is answered from the
    /// placeholder; the real tree's arrival raises a focus event, and the
    /// client reads again.
    pub(crate) fn read_past_placeholder(&self, what: impl FnOnce() -> String) {
        if !self.is_placeholder.load(Ordering::SeqCst)
            || self.tree_requested.load(Ordering::SeqCst)
            || self.activation_posted.swap(true, Ordering::SeqCst)
        {
            return;
        }
        log(|| format!("a client read past the placeholder ({})", what()));
        // SAFETY: posting to our own window, from any thread.
        let _ = unsafe {
            PostMessageW(
                Some(self.hwnd.0),
                activation_message(),
                WPARAM(0),
                LPARAM(0),
            )
        };
    }

    /// A client added (or removed) a handler for these event and property ids.
    pub(crate) fn advise(&self, ids: &[i32], added: bool) {
        let mut subscriptions = self.subscriptions.lock().unwrap();
        for id in ids {
            let count = subscriptions.entry(*id).or_default();
            *count = if added { *count + 1 } else { count.saturating_sub(1) };
        }
    }

    /// Whether raising `event_id` (with `property_id`, for a property
    /// change) reaches anyone: always while a screen reader runs (it may
    /// take events through the MSAA bridge, where no handler is advised),
    /// and for focus, which legacy WinEvent clients follow; otherwise only
    /// what a client listens for. Raising what no one listens for made UIA
    /// call back into the provider about ten times an event: a dashboard's
    /// live labels raised two thousand text and name changes a second.
    /// `screen_reader` is `screen_reader()`'s answer, read once an update:
    /// a system call each event cost as much as raising them.
    pub(crate) fn wanted(
        &self,
        event_id: UIA_EVENT_ID,
        property_id: Option<UIA_PROPERTY_ID>,
        screen_reader: bool,
    ) -> bool {
        event_id == UIA_AutomationFocusChangedEventId
            || screen_reader
            || listened(&self.subscriptions.lock().unwrap(), event_id, property_id)
    }

    pub(crate) fn read_tree(&self) -> RwLockReadGuard<'_, Tree> {
        self.tree.read().unwrap()
    }

    pub(crate) fn client_top_left(&self) -> Point {
        client_top_left(self.hwnd)
    }

    pub(crate) fn do_action(&self, request: ActionRequest) {
        self.action_handler.do_action(request);
    }

    pub(crate) fn get_or_create_platform_node(
        self: &Arc<Self>,
        id: NodeId,
    ) -> ComObject<PlatformNode> {
        let mut platform_nodes = self.platform_nodes.lock().unwrap();
        if let Some(result) = platform_nodes.get(&id) {
            return result.clone();
        }

        let result = PlatformNode::new(self, id);
        platform_nodes.insert(id, result.clone());
        result
    }

    pub(crate) fn remove_platform_node(&self, id: NodeId) {
        let mut platform_nodes = self.platform_nodes.lock().unwrap();
        platform_nodes.remove(&id);
    }
}

/// Whether a screen reader runs (`SPI_GETSCREENREADER`).
pub(crate) fn screen_reader() -> bool {
    let mut on = windows::core::BOOL(0);
    // SAFETY: SPI_GETSCREENREADER writes one BOOL to the pointer given.
    let read = unsafe {
        windows::Win32::UI::WindowsAndMessaging::SystemParametersInfoW(
            windows::Win32::UI::WindowsAndMessaging::SPI_GETSCREENREADER,
            0,
            Some((&mut on as *mut windows::core::BOOL).cast()),
            windows::Win32::UI::WindowsAndMessaging::SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    // Unknown: as if one runs, so nothing is held back.
    read.is_err() || on.as_bool()
}

/// Whether a client in `subscriptions` listens for `event_id`, and for a
/// property change, for `property_id` too.
fn listened(
    subscriptions: &HashMap<i32, usize>,
    event_id: UIA_EVENT_ID,
    property_id: Option<UIA_PROPERTY_ID>,
) -> bool {
    let has = |id: i32| subscriptions.get(&id).is_some_and(|count| *count > 0);
    has(event_id.0) && property_id.is_none_or(|property| has(property.0))
}

/// zgx patch: the message a placeholder's window is sent when a client
/// reads past the placeholder (`Context::read_past_placeholder`). The
/// subclassing adapter handles it; a window using `Adapter` directly passes
/// it to `Adapter::activate`.
pub fn activation_message() -> u32 {
    static MESSAGE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    // SAFETY: a constant, NUL-terminated name.
    *MESSAGE.get_or_init(|| unsafe { RegisterWindowMessageW(w!("AccessKitActivate")) })
}

/// zgx patch: what the adapter does about clients, on standard error,
/// while `ZGX_ACCESSIBILITY_LOG` is set.
pub(crate) fn log(what: impl FnOnce() -> String) {
    if std::env::var_os("ZGX_ACCESSIBILITY_LOG").is_some() {
        eprintln!("accessibility: {}", what());
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::Accessibility::{
        UIA_AutomationPropertyChangedEventId, UIA_NamePropertyId, UIA_Text_TextChangedEventId,
        UIA_ValueValuePropertyId,
    };

    #[test]
    fn only_events_a_client_listens_for_are_raised() {
        let mut subscriptions = HashMap::new();
        let property = UIA_AutomationPropertyChangedEventId;
        assert!(!listened(&subscriptions, UIA_Text_TextChangedEventId, None));
        assert!(!listened(&subscriptions, property, Some(UIA_NamePropertyId)));
        // A handler for value changes: those, not name changes.
        subscriptions.insert(property.0, 1);
        subscriptions.insert(UIA_ValueValuePropertyId.0, 1);
        assert!(listened(&subscriptions, property, Some(UIA_ValueValuePropertyId)));
        assert!(!listened(&subscriptions, property, Some(UIA_NamePropertyId)));
        assert!(!listened(&subscriptions, UIA_Text_TextChangedEventId, None));
        // Removed: none again.
        subscriptions.insert(UIA_ValueValuePropertyId.0, 0);
        assert!(!listened(&subscriptions, property, Some(UIA_ValueValuePropertyId)));
    }
}
