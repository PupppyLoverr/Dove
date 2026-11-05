// Copyright 2022 The AccessKit Authors. All rights reserved.
// Licensed under the Apache License, Version 2.0 (found in
// the LICENSE-APACHE file) or the MIT license (found in
// the LICENSE-MIT file), at your option.

use accesskit::{
    ActionHandler, ActivationHandler, Live, Node as NodeProvider, NodeId as LocalNodeId, Role,
    Tree as TreeData, TreeId, TreeUpdate,
};
use accesskit_consumer::{FilterResult, Node, NodeId, Tree, TreeChangeHandler, TreeState};
use hashbrown::{HashMap, HashSet};
use std::fmt::{Debug, Formatter};
use std::sync::{Arc, atomic::Ordering};
use windows::Win32::{
    Foundation::*,
    UI::{Accessibility::*, WindowsAndMessaging::*},
};

use crate::{
    context::{ActionHandlerNoMut, ActionHandlerWrapper, Context},
    filters::filter,
    node::{NodeWrapper, PlatformNode},
    util::{QueuedEvent, StringBuffer},
    window_handle::WindowHandle,
};

fn focus_event(context: &Arc<Context>, node_id: NodeId) -> QueuedEvent {
    let platform_node = context.get_or_create_platform_node(node_id);
    let element: IRawElementProviderSimple = platform_node.into_interface();
    QueuedEvent::Simple {
        element,
        event_id: UIA_AutomationFocusChangedEventId,
    }
}

struct AdapterChangeHandler<'a> {
    context: &'a Arc<Context>,
    queue: Vec<QueuedEvent>,
    text_changed: HashSet<NodeId>,
    selection_changed: HashMap<NodeId, SelectionChanges>,
    /// zgx patch: whether a screen reader runs, read once for the update.
    screen_reader: bool,
}

impl<'a> AdapterChangeHandler<'a> {
    fn new(context: &'a Arc<Context>) -> Self {
        Self {
            context,
            queue: Vec::new(),
            text_changed: HashSet::new(),
            selection_changed: HashMap::new(),
            screen_reader: crate::context::screen_reader(),
        }
    }

    /// zgx patch: whether `event_id` (a property change of `property_id`)
    /// reaches a client (`Context::wanted`).
    fn wanted(&self, event_id: UIA_EVENT_ID, property_id: Option<UIA_PROPERTY_ID>) -> bool {
        self.context.wanted(event_id, property_id, self.screen_reader)
    }
}

impl AdapterChangeHandler<'_> {
    fn insert_text_change_if_needed_parent(&mut self, node: Node) {
        if !node.supports_text_ranges() || !self.wanted(UIA_Text_TextChangedEventId, None) {
            return;
        }
        let id = node.id();
        if self.text_changed.contains(&id) {
            return;
        }
        let platform_node = self.context.get_or_create_platform_node(node.id());
        let element: IRawElementProviderSimple = platform_node.into_interface();
        // Text change events must come before selection change
        // events. It doesn't matter if text change events come
        // before other events.
        self.queue.insert(
            0,
            QueuedEvent::Simple {
                element,
                event_id: UIA_Text_TextChangedEventId,
            },
        );
        self.text_changed.insert(id);
    }

    fn insert_text_change_if_needed(&mut self, node: &Node) {
        if node.role() != Role::TextRun {
            return;
        }
        if let Some(node) = node.filtered_parent(&filter) {
            self.insert_text_change_if_needed_parent(node);
        }
    }

    fn element_for_possibly_removed_node(
        &self,
        tree_state: &TreeState,
        id: NodeId,
    ) -> IRawElementProviderSimple {
        if tree_state.node_by_id(id).is_some() {
            self.context
                .get_or_create_platform_node(id)
                .into_interface()
        } else {
            PlatformNode::new(self.context, id).into_interface()
        }
    }

    fn handle_selection_state_change(&mut self, node: &Node, is_selected: bool) {
        // If `node` belongs to a selection container, then map the events with the
        // selection container as the key because |FinalizeSelectionEvents| needs to
        // determine whether or not there is only one element selected in order to
        // optimize what platform events are sent.
        let key = if let Some(container) = node.selection_container(&filter) {
            container.id()
        } else {
            node.id()
        };

        let changes = self
            .selection_changed
            .entry(key)
            .or_insert_with(|| SelectionChanges {
                added_items: HashSet::new(),
                removed_items: HashSet::new(),
            });
        if is_selected {
            changes.added_items.insert(node.id());
        } else {
            changes.removed_items.insert(node.id());
        }
    }

    fn enqueue_selection_changes(&mut self, tree: &Tree) {
        let tree_state = tree.state();
        for (id, changes) in self.selection_changed.iter() {
            let Some(node) = tree_state.node_by_id(*id) else {
                continue;
            };
            // Determine if `node` is a selection container with one selected child in
            // order to optimize what platform events are sent.
            let mut container = None;
            let mut only_selected_child = None;
            if node.is_container_with_selectable_children() {
                container = Some(node);
                for child in node.filtered_children(filter) {
                    if let Some(true) = child.is_selected() {
                        if only_selected_child.is_none() {
                            only_selected_child = Some(child);
                        } else {
                            only_selected_child = None;
                            break;
                        }
                    }
                }
            }

            if let Some(only_selected_child) = only_selected_child {
                self.queue.push(QueuedEvent::Simple {
                    element: self
                        .context
                        .get_or_create_platform_node(only_selected_child.id())
                        .into_interface(),
                    event_id: UIA_SelectionItem_ElementSelectedEventId,
                });
                self.queue.push(QueuedEvent::PropertyChanged {
                    element: self
                        .context
                        .get_or_create_platform_node(only_selected_child.id())
                        .into_interface(),
                    property_id: UIA_SelectionItemIsSelectedPropertyId,
                    old_value: false.into(),
                    new_value: true.into(),
                });
                for child_id in changes.removed_items.iter() {
                    let element = self.element_for_possibly_removed_node(tree_state, *child_id);
                    self.queue.push(QueuedEvent::PropertyChanged {
                        element,
                        property_id: UIA_SelectionItemIsSelectedPropertyId,
                        old_value: true.into(),
                        new_value: false.into(),
                    });
                }
            } else {
                // Per UIA documentation, beyond the "invalidate limit" we're supposed to
                // fire a 'SelectionInvalidated' event.  The exact value isn't specified,
                // but System.Windows.Automation.Provider uses a value of 20.
                const INVALIDATE_LIMIT: usize = 20;
                if let Some(container) = container.filter(|_| {
                    changes.added_items.len() + changes.removed_items.len() > INVALIDATE_LIMIT
                }) {
                    let platform_node = self.context.get_or_create_platform_node(container.id());
                    self.queue.push(QueuedEvent::Simple {
                        element: platform_node.into_interface(),
                        event_id: UIA_Selection_InvalidatedEventId,
                    });
                } else {
                    let container_is_multiselectable =
                        container.is_some_and(|c| c.is_multiselectable());
                    for added_id in changes.added_items.iter() {
                        self.queue.push(QueuedEvent::Simple {
                            element: self
                                .context
                                .get_or_create_platform_node(*added_id)
                                .into_interface(),
                            event_id: match container_is_multiselectable {
                                true => UIA_SelectionItem_ElementAddedToSelectionEventId,
                                false => UIA_SelectionItem_ElementSelectedEventId,
                            },
                        });
                        self.queue.push(QueuedEvent::PropertyChanged {
                            element: self
                                .context
                                .get_or_create_platform_node(*added_id)
                                .into_interface(),
                            property_id: UIA_SelectionItemIsSelectedPropertyId,
                            old_value: false.into(),
                            new_value: true.into(),
                        });
                    }
                    for removed_id in changes.removed_items.iter() {
                        let element =
                            self.element_for_possibly_removed_node(tree_state, *removed_id);
                        self.queue.push(QueuedEvent::Simple {
                            element: element.clone(),
                            event_id: UIA_SelectionItem_ElementRemovedFromSelectionEventId,
                        });
                        self.queue.push(QueuedEvent::PropertyChanged {
                            element,
                            property_id: UIA_SelectionItemIsSelectedPropertyId,
                            old_value: true.into(),
                            new_value: false.into(),
                        });
                    }
                }
            }
        }
    }
}

struct SelectionChanges {
    added_items: HashSet<NodeId>,
    removed_items: HashSet<NodeId>,
}

impl TreeChangeHandler for AdapterChangeHandler<'_> {
    fn node_added(&mut self, node: &Node) {
        self.insert_text_change_if_needed(node);
        if filter(node) != FilterResult::Include {
            return;
        }
        let mut buffer = StringBuffer::acquire();
        let mut wrapper = NodeWrapper {
            node,
            string_buffer: &mut buffer,
        };
        if node.is_dialog() {
            let platform_node = self.context.get_or_create_platform_node(node.id());
            let element: IRawElementProviderSimple = platform_node.into_interface();
            self.queue.push(QueuedEvent::Simple {
                element,
                event_id: UIA_Window_WindowOpenedEventId,
            });
        }
        if wrapper.name().is_some() && node.live() != Live::Off {
            let platform_node = self.context.get_or_create_platform_node(node.id());
            let element: IRawElementProviderSimple = platform_node.into_interface();
            self.queue.push(QueuedEvent::Simple {
                element,
                event_id: UIA_LiveRegionChangedEventId,
            });
        }
        if wrapper.is_selection_item_pattern_supported() && wrapper.is_selected() {
            self.handle_selection_state_change(node, true);
        }
    }

    fn node_updated(&mut self, old_node: &Node, new_node: &Node) {
        if old_node.raw_value() != new_node.raw_value() {
            self.insert_text_change_if_needed(new_node);
        }
        let old_node_was_filtered_out = filter(old_node) != FilterResult::Include;
        if filter(new_node) != FilterResult::Include {
            if !old_node_was_filtered_out {
                if old_node.is_dialog() {
                    let platform_node = self.context.get_or_create_platform_node(old_node.id());
                    let element: IRawElementProviderSimple = platform_node.into_interface();
                    self.queue.push(QueuedEvent::Simple {
                        element,
                        event_id: UIA_Window_WindowClosedEventId,
                    });
                }
                let mut old_buffer = StringBuffer::acquire();
                let old_wrapper = NodeWrapper {
                    node: old_node,
                    string_buffer: &mut old_buffer,
                };
                if old_wrapper.is_selection_item_pattern_supported() && old_wrapper.is_selected() {
                    self.handle_selection_state_change(old_node, false);
                }
            }
            return;
        }
        let platform_node = self.context.get_or_create_platform_node(new_node.id());
        let element: IRawElementProviderSimple = platform_node.into_interface();
        let mut old_buffer = StringBuffer::acquire();
        let mut old_wrapper = NodeWrapper {
            node: old_node,
            string_buffer: &mut old_buffer,
        };
        let mut new_buffer = StringBuffer::acquire();
        let mut new_wrapper = NodeWrapper {
            node: new_node,
            string_buffer: &mut new_buffer,
        };
        new_wrapper.enqueue_property_changes(
            &mut self.queue,
            self.context,
            &element,
            &mut old_wrapper,
        );
        let new_name = new_wrapper.name();
        if new_name.is_some()
            && new_node.live() != Live::Off
            && (new_node.live() != old_node.live()
                || old_node_was_filtered_out
                || new_name != old_wrapper.name())
        {
            self.queue.push(QueuedEvent::Simple {
                element,
                event_id: UIA_LiveRegionChangedEventId,
            });
        }
        if old_node_was_filtered_out && new_node.is_dialog() {
            let platform_node = self.context.get_or_create_platform_node(new_node.id());
            let element: IRawElementProviderSimple = platform_node.into_interface();
            self.queue.push(QueuedEvent::Simple {
                element,
                event_id: UIA_Window_WindowOpenedEventId,
            });
        }
        if new_wrapper.is_selection_item_pattern_supported()
            && (new_wrapper.is_selected() != old_wrapper.is_selected()
                || (old_node_was_filtered_out && new_wrapper.is_selected()))
        {
            self.handle_selection_state_change(new_node, new_wrapper.is_selected());
        }
    }

    fn focus_moved(&mut self, _old_node: Option<&Node>, new_node: Option<&Node>) {
        if let Some(new_node) = new_node {
            self.queue.push(focus_event(self.context, new_node.id()));
        }
    }

    fn node_removed(&mut self, node: &Node) {
        self.insert_text_change_if_needed(node);
        self.context.remove_platform_node(node.id());
        if filter(node) != FilterResult::Include {
            return;
        }
        if node.is_dialog() {
            let platform_node = PlatformNode::new(self.context, node.id());
            let element: IRawElementProviderSimple = platform_node.into_interface();
            self.queue.push(QueuedEvent::Simple {
                element,
                event_id: UIA_Window_WindowClosedEventId,
            });
        }
        let mut buffer = StringBuffer::acquire();
        let wrapper = NodeWrapper {
            node,
            string_buffer: &mut buffer,
        };
        if wrapper.is_selection_item_pattern_supported() {
            self.handle_selection_state_change(node, false);
        }
    }

    // TODO: handle other events (#20)
}

const PLACEHOLDER_ROOT_ID: LocalNodeId = LocalNodeId(0);

enum State {
    Inactive {
        hwnd: WindowHandle,
        is_window_focused: bool,
        action_handler: Arc<dyn ActionHandlerNoMut + Send + Sync>,
    },
    Placeholder(Arc<Context>),
    Active(Arc<Context>),
}

impl Debug for State {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            State::Inactive {
                hwnd,
                is_window_focused,
                action_handler: _,
            } => f
                .debug_struct("Inactive")
                .field("hwnd", hwnd)
                .field("is_window_focused", is_window_focused)
                .field("action_handler", &"ActionHandler")
                .finish(),
            State::Placeholder(context) => f.debug_tuple("Placeholder").field(context).finish(),
            State::Active(context) => f.debug_tuple("Active").field(context).finish(),
        }
    }
}

#[derive(Debug)]
pub struct Adapter {
    state: State,
}

impl Adapter {
    /// Creates a new Windows platform adapter.
    ///
    /// The action handler may or may not be called on the thread that owns
    /// the window.
    ///
    /// This must not be called while handling the `WM_GETOBJECT` message,
    /// because this function must initialize UI Automation before
    /// that message is handled. This is necessary to prevent a race condition
    /// that leads to nested `WM_GETOBJECT` messages and, in some cases,
    /// assistive technologies not realizing that the window natively implements.
    /// UIA. See [AccessKit issue #37](https://github.com/AccessKit/accesskit/issues/37)
    /// for more details.
    pub fn new(
        hwnd: HWND,
        is_window_focused: bool,
        action_handler: impl 'static + ActionHandler + Send,
    ) -> Self {
        Self::with_wrapped_action_handler(
            hwnd,
            is_window_focused,
            Arc::new(ActionHandlerWrapper::new(action_handler)),
        )
    }

    // Currently required by the test infrastructure
    pub(crate) fn with_wrapped_action_handler(
        hwnd: HWND,
        is_window_focused: bool,
        action_handler: Arc<dyn ActionHandlerNoMut + Send + Sync>,
    ) -> Self {
        init_uia();

        let state = State::Inactive {
            hwnd: hwnd.into(),
            is_window_focused,
            action_handler,
        };
        Self { state }
    }

    /// If and only if the tree has been initialized, call the provided function
    /// and apply the resulting update. Note: If the caller's implementation of
    /// [`ActivationHandler::request_initial_tree`] initially returned `None`,
    /// the [`TreeUpdate`] returned by the provided function must contain
    /// a full tree.
    ///
    /// If a [`QueuedEvents`] instance is returned, the caller must call
    /// [`QueuedEvents::raise`] on it.
    ///
    /// This method may be safely called on any thread, but refer to
    /// [`QueuedEvents::raise`] for restrictions on the context in which
    /// it should be called.
    pub fn update_if_active(
        &mut self,
        update_factory: impl FnOnce() -> TreeUpdate,
    ) -> Option<QueuedEvents> {
        match &self.state {
            State::Inactive { .. } => None,
            State::Placeholder(context) => {
                let is_window_focused = context.read_tree().state().is_host_focused();
                let tree = Tree::new(update_factory(), is_window_focused);
                *context.tree.write().unwrap() = tree;
                context.is_placeholder.store(false, Ordering::SeqCst);
                let result = context
                    .read_tree()
                    .state()
                    .focus()
                    .map(|node| QueuedEvents(vec![focus_event(context, node.id())]));
                self.state = State::Active(Arc::clone(context));
                result
            }
            State::Active(context) => {
                let mut handler = AdapterChangeHandler::new(context);
                let mut tree = context.tree.write().unwrap();
                tree.update_and_process_changes(update_factory(), &mut handler);
                handler.enqueue_selection_changes(&tree);
                // zgx patch: only what a client listens for (`Context::wanted`).
                let queue = std::mem::take(&mut handler.queue);
                handler.queue = queue
                    .into_iter()
                    .filter(|event| match event {
                        QueuedEvent::Simple { event_id, .. } => handler.wanted(*event_id, None),
                        QueuedEvent::PropertyChanged { property_id, .. } => {
                            handler.wanted(UIA_AutomationPropertyChangedEventId, Some(*property_id))
                        }
                    })
                    .collect();
                Some(QueuedEvents(handler.queue))
            }
        }
    }

    /// Update the tree state based on whether the window is focused.
    ///
    /// If a [`QueuedEvents`] instance is returned, the caller must call
    /// [`QueuedEvents::raise`] on it.
    ///
    /// This method may be safely called on any thread, but refer to
    /// [`QueuedEvents::raise`] for restrictions on the context in which
    /// it should be called.
    pub fn update_window_focus_state(&mut self, is_focused: bool) -> Option<QueuedEvents> {
        match &mut self.state {
            State::Inactive {
                is_window_focused, ..
            } => {
                *is_window_focused = is_focused;
                None
            }
            State::Placeholder(context) => {
                let mut handler = AdapterChangeHandler::new(context);
                let mut tree = context.tree.write().unwrap();
                tree.update_host_focus_state_and_process_changes(is_focused, &mut handler);
                Some(QueuedEvents(handler.queue))
            }
            State::Active(context) => {
                let mut handler = AdapterChangeHandler::new(context);
                let mut tree = context.tree.write().unwrap();
                tree.update_host_focus_state_and_process_changes(is_focused, &mut handler);
                Some(QueuedEvents(handler.queue))
            }
        }
    }

    /// Handle the `WM_GETOBJECT` window message. The accessibility tree
    /// is lazily initialized if necessary using the provided
    /// [`ActivationHandler`] implementation.
    ///
    /// This returns an `Option` so the caller can pass the message
    /// to `DefWindowProc` if AccessKit decides not to handle it.
    /// The optional value is an `Into<LRESULT>` rather than simply an `LRESULT`
    /// so the necessary call to UIA, which may lead to a nested `WM_GETOBJECT`
    /// message, can be done outside of any lock that the caller might hold
    /// on the `Adapter` or window state, while still abstracting away
    /// the details of that call to UIA.
    pub fn handle_wm_getobject<H: ActivationHandler + ?Sized>(
        &mut self,
        wparam: WPARAM,
        lparam: LPARAM,
        activation_handler: &mut H,
    ) -> Option<impl Into<LRESULT> + use<H>> {
        // Don't bother with MSAA object IDs that are asking for something other
        // than the client area of the window. DefWindowProc can handle those.
        // First, cast the lparam to i32, to handle inconsistent conversion
        // behavior in senders.
        let objid = normalize_objid(lparam);
        if objid < 0 && objid != UiaRootObjectId && objid != OBJID_CLIENT.0 {
            return None;
        }

        // zgx patch: only UI Automation's own request for the root asks the
        // activation handler for the tree. MSAA's (`OBJID_CLIENT`, and the
        // ids 0 and above `AccessibleObjectFromEvent` sends) get a
        // placeholder root: Windows' text services send them on every
        // window as it is shown, in-thread, with no assistive technology
        // running, and building the tree for them kept every window's
        // accessibility tree live. A client reading past the placeholder
        // (`Context::read_past_placeholder`) gets the tree then.
        let uia_root = objid == UiaRootObjectId;
        if let State::Inactive {
            hwnd,
            is_window_focused,
            action_handler,
        } = &self.state
        {
            let (hwnd, is_window_focused) = (*hwnd, *is_window_focused);
            let action_handler = Arc::clone(action_handler);
            if uia_root {
                self.state = match activation_handler.request_initial_tree() {
                    Some(initial_state) => {
                        let tree = Tree::new(initial_state, is_window_focused);
                        State::Active(Context::new(hwnd, tree, action_handler, false))
                    }
                    None => {
                        let context = placeholder(hwnd, is_window_focused, action_handler);
                        context.tree_requested.store(true, Ordering::SeqCst);
                        State::Placeholder(context)
                    }
                };
            } else {
                crate::context::log(|| {
                    format!("a placeholder for an MSAA request (object id {objid})")
                });
                self.state =
                    State::Placeholder(placeholder(hwnd, is_window_focused, action_handler));
            }
        } else if uia_root {
            // A placeholder served to MSAA: UI Automation's request for the
            // root asks for the tree now.
            let _ = self.activate(activation_handler);
        }
        let (hwnd, platform_node) = match &self.state {
            State::Inactive { .. } => unreachable!("made active or a placeholder above"),
            State::Placeholder(context) => (context.hwnd, PlatformNode::unspecified_root(context)),
            State::Active(context) => {
                let node_id = context.read_tree().state().root_id();
                (context.hwnd, context.get_or_create_platform_node(node_id))
            }
        };
        let el: IRawElementProviderSimple = platform_node.into_interface();
        Some(WmGetObjectResult {
            hwnd,
            wparam,
            lparam,
            el,
        })
    }
}

impl Adapter {
    /// zgx patch: a placeholder's window was sent `activation_message` (a
    /// client read past the placeholder), or UI Automation asked for the
    /// root: ask the activation handler for the tree, once. A tree given at
    /// once replaces the placeholder (with a focus event, so clients read
    /// again); otherwise the next `update_if_active` does.
    ///
    /// If a [`QueuedEvents`] instance is returned, the caller must call
    /// [`QueuedEvents::raise`] on it, outside of `WM_GETOBJECT`.
    pub fn activate<H: ActivationHandler + ?Sized>(
        &mut self,
        activation_handler: &mut H,
    ) -> Option<QueuedEvents> {
        let State::Placeholder(context) = &self.state else {
            return None;
        };
        if context.tree_requested.swap(true, Ordering::SeqCst) {
            return None;
        }
        let update = activation_handler.request_initial_tree()?;
        self.update_if_active(|| update)
    }
}

/// zgx patch: a placeholder root (role `Window`, no children) until the
/// tree comes.
fn placeholder(
    hwnd: WindowHandle,
    is_window_focused: bool,
    action_handler: Arc<dyn ActionHandlerNoMut + Send + Sync>,
) -> Arc<Context> {
    let placeholder_update = TreeUpdate {
        nodes: vec![(PLACEHOLDER_ROOT_ID, NodeProvider::new(Role::Window))],
        tree: Some(TreeData::new(PLACEHOLDER_ROOT_ID)),
        tree_id: TreeId::ROOT,
        focus: PLACEHOLDER_ROOT_ID,
    };
    let placeholder_tree = Tree::new(placeholder_update, is_window_focused);
    Context::new(hwnd, placeholder_tree, action_handler, true)
}

fn init_uia() {
    // `UiaLookupId` is a cheap way of forcing UIA to initialize itself.
    unsafe {
        UiaLookupId(
            AutomationIdentifierType_Property,
            &ControlType_Property_GUID,
        )
    };
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn normalize_objid(lparam: LPARAM) -> i32 {
    (lparam.0 & 0xFFFFFFFF) as _
}
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn normalize_objid(lparam: LPARAM) -> i32 {
    lparam.0 as _
}

struct WmGetObjectResult {
    hwnd: WindowHandle,
    wparam: WPARAM,
    lparam: LPARAM,
    el: IRawElementProviderSimple,
}

impl From<WmGetObjectResult> for LRESULT {
    fn from(this: WmGetObjectResult) -> Self {
        unsafe { UiaReturnRawElementProvider(this.hwnd.0, this.wparam, this.lparam, &this.el) }
    }
}

/// Events generated by a tree update.
#[must_use = "events must be explicitly raised"]
pub struct QueuedEvents(Vec<QueuedEvent>);

impl QueuedEvents {
    /// Raise all queued events synchronously.
    ///
    /// The window may receive `WM_GETOBJECT` messages during this call.
    /// This means that any locks required by the `WM_GETOBJECT` handler
    /// must not be held when this method is called.
    ///
    /// This method should be called on the thread that owns the window.
    /// It's not clear whether this is a strict requirement of UIA itself,
    /// but based on the known behavior of UIA, MSAA, and some ATs,
    /// it's strongly recommended.
    pub fn raise(self) {
        for event in self.0 {
            match event {
                QueuedEvent::Simple { element, event_id } => {
                    unsafe { UiaRaiseAutomationEvent(&element, event_id) }.unwrap();
                }
                QueuedEvent::PropertyChanged {
                    element,
                    property_id,
                    old_value,
                    new_value,
                } => {
                    unsafe {
                        UiaRaiseAutomationPropertyChangedEvent(
                            &element,
                            property_id,
                            &old_value,
                            &new_value,
                        )
                    }
                    .unwrap();
                }
            }
        }
    }
}

// We explicitly want to allow the queued events to be sent to the UI thread,
// so implement Send even though windows-rs doesn't implement it for all
// contained types. This is safe because we're not using COM threading.
unsafe impl Send for QueuedEvents {}

/// zgx patch: MSAA probes get a placeholder; reading past it, or UI
/// Automation's request for the root, asks for the tree.
#[cfg(test)]
mod placeholder_tests {
    use super::*;
    use accesskit::{ActionRequest, Tree as TreeData};
    use windows::core::{Interface, w};

    struct Counting {
        asked: usize,
    }
    impl ActivationHandler for Counting {
        fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
            self.asked += 1;
            let mut root = NodeProvider::new(Role::Window);
            root.set_children(vec![LocalNodeId(1)]);
            let mut button = NodeProvider::new(Role::Button);
            button.set_label("Button");
            Some(TreeUpdate {
                nodes: vec![(LocalNodeId(0), root), (LocalNodeId(1), button)],
                tree: Some(TreeData::new(LocalNodeId(0))),
                tree_id: TreeId::ROOT,
                focus: LocalNodeId(1),
            })
        }
    }
    struct NoActions;
    impl ActionHandler for NoActions {
        fn do_action(&mut self, _request: ActionRequest) {}
    }

    fn window() -> HWND {
        // SAFETY: a system class's window, destroyed by the test.
        unsafe {
            CreateWindowExW(
                Default::default(),
                w!("STATIC"),
                w!("placeholder test"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                200,
                100,
                None,
                None,
                None,
                None,
            )
        }
        .unwrap()
    }
    /// MSAA's `AccessibleObjectFromEvent`, as text services send it when a
    /// window shows: `OBJID_CLIENT`, then 0, 1, 0 and 2.
    fn probe(adapter: &mut Adapter, handler: &mut Counting) {
        for objid in [OBJID_CLIENT.0, 0, 1, 0, 2] {
            let result = adapter.handle_wm_getobject(
                WPARAM(0xffff_ffff),
                LPARAM(objid as isize),
                handler,
            );
            assert!(result.is_some(), "object id {objid} is answered");
        }
    }
    fn activation_posted(hwnd: HWND) -> bool {
        let message = crate::activation_message();
        let mut msg = MSG::default();
        // SAFETY: our own window's queue, on its thread.
        unsafe { PeekMessageW(&mut msg, Some(hwnd), message, message, PM_REMOVE) }.as_bool()
    }

    #[test]
    fn an_msaa_probe_gets_a_placeholder_and_reading_past_it_asks_for_the_tree() {
        let _lock_guard = crate::tests::MUTEX.lock();
        let hwnd = window();
        // Focused: the tree's arrival raises a focus event.
        let mut adapter = Adapter::new(hwnd, true, NoActions);
        let mut handler = Counting { asked: 0 };
        probe(&mut adapter, &mut handler);
        assert_eq!(handler.asked, 0, "no tree for the probe");
        let State::Placeholder(context) = &adapter.state else {
            panic!("a placeholder: {:?}", adapter.state);
        };
        // What a probe reads of a window, the placeholder answers.
        let root: IRawElementProviderSimple =
            PlatformNode::unspecified_root(context).into_interface();
        for property in [UIA_ControlTypePropertyId, UIA_NamePropertyId, UIA_IsEnabledPropertyId] {
            // SAFETY: a live provider, on its window's thread.
            unsafe { root.GetPropertyValue(property) }.unwrap();
        }
        assert!(!activation_posted(hwnd));
        // Its children: the window is asked to activate, once.
        let fragment: IRawElementProviderFragment = root.cast().unwrap();
        for _ in 0..2 {
            // SAFETY: as above; the placeholder has no children (an error).
            let _ = unsafe { fragment.Navigate(NavigateDirection_FirstChild) };
        }
        assert!(activation_posted(hwnd));
        assert!(!activation_posted(hwnd), "posted once");
        // Its handling: the tree replaces the placeholder, with a focus
        // event for clients to read it again.
        let events = adapter.activate(&mut handler);
        assert_eq!(handler.asked, 1);
        assert!(events.is_some());
        assert!(matches!(adapter.state, State::Active(_)));
        assert!(adapter.activate(&mut handler).is_none());
        assert_eq!(handler.asked, 1);
        // SAFETY: the window made above.
        unsafe { DestroyWindow(hwnd) }.unwrap();
    }

    #[test]
    fn ui_automations_request_for_the_root_asks_for_the_tree_at_once() {
        let _lock_guard = crate::tests::MUTEX.lock();
        let hwnd = window();
        let root_request = |adapter: &mut Adapter, handler: &mut Counting| {
            let result = adapter.handle_wm_getobject(
                WPARAM(0),
                LPARAM(UiaRootObjectId as isize),
                handler,
            );
            assert!(result.is_some());
        };
        // First, as a screen reader's.
        let mut adapter = Adapter::new(hwnd, false, NoActions);
        let mut handler = Counting { asked: 0 };
        root_request(&mut adapter, &mut handler);
        assert_eq!(handler.asked, 1);
        assert!(matches!(adapter.state, State::Active(_)));
        drop(adapter);
        // After a probe's placeholder.
        let mut adapter = Adapter::new(hwnd, false, NoActions);
        let mut handler = Counting { asked: 0 };
        probe(&mut adapter, &mut handler);
        root_request(&mut adapter, &mut handler);
        assert_eq!(handler.asked, 1);
        assert!(matches!(adapter.state, State::Active(_)));
        // SAFETY: the window made above.
        unsafe { DestroyWindow(hwnd) }.unwrap();
    }
}
