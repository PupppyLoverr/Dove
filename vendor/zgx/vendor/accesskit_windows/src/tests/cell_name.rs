// zgx patch: a table's cell or header with no label is named by its
// contents in UI Automation.

use accesskit::{ActivationHandler, Node, NodeId, Role, Tree, TreeId, TreeUpdate};
use windows::Win32::System::Variant::VARIANT;

use super::{simple::NullActionHandler, *};

const WINDOW_ID: NodeId = NodeId(0);

fn text(value: &str) -> Node {
    let mut node = Node::new(Role::Label);
    node.set_value(value);
    node
}

fn with_children(role: Role, children: Vec<NodeId>) -> Node {
    let mut node = Node::new(role);
    node.set_children(children);
    node
}

struct GridActivationHandler;

impl ActivationHandler for GridActivationHandler {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        let mut labelled = Node::new(Role::Cell);
        labelled.set_label("Labelled");
        labelled.set_children(vec![NodeId(11)]);
        // An empty label is none.
        let mut empty = Node::new(Role::Cell);
        empty.set_label("");
        empty.set_children(vec![NodeId(14)]);
        let mut grid = with_children(Role::Grid, vec![NodeId(2)]);
        grid.set_label("Orders");
        Some(TreeUpdate {
            nodes: vec![
                (WINDOW_ID, with_children(Role::Window, vec![NodeId(1)])),
                (NodeId(1), grid),
                (
                    NodeId(2),
                    with_children(
                        Role::Row,
                        vec![NodeId(3), NodeId(5), NodeId(8), NodeId(10), NodeId(13)],
                    ),
                ),
                // Text in a child.
                (NodeId(3), with_children(Role::Cell, vec![NodeId(4)])),
                (NodeId(4), text("#1001")),
                // Two texts, one deeper in a container.
                (
                    NodeId(5),
                    with_children(Role::Cell, vec![NodeId(6), NodeId(7)]),
                ),
                (NodeId(6), text("Customer")),
                (
                    NodeId(7),
                    with_children(Role::GenericContainer, vec![NodeId(12)]),
                ),
                (NodeId(12), text("One")),
                // Nothing in it.
                (NodeId(8), with_children(Role::Cell, vec![NodeId(9)])),
                (NodeId(9), Node::new(Role::GenericContainer)),
                // Its own label, kept.
                (NodeId(10), labelled),
                (NodeId(11), text("Ignored")),
                (NodeId(13), empty),
                (NodeId(14), text("$3.70")),
            ],
            tree: Some(Tree::new(WINDOW_ID)),
            tree_id: TreeId::ROOT,
            focus: WINDOW_ID,
        })
    }
}

/// The names of the grid's cells, in order, as UIA gives them.
fn cell_names(s: &Scope) -> Result<Vec<String>> {
    let root = unsafe { s.uia.ElementFromHandle(s.window.0) }?;
    let condition = unsafe {
        s.uia.CreatePropertyCondition(
            UIA_ControlTypePropertyId,
            &VARIANT::from(UIA_DataItemControlTypeId.0),
        )
    }?;
    let items = unsafe { root.FindAll(TreeScope_Descendants, &condition) }?;
    let mut names = Vec::new();
    for i in 0..unsafe { items.Length() }? {
        let item = unsafe { items.GetElement(i) }?;
        names.push(unsafe { item.CurrentName() }?.to_string());
    }
    Ok(names)
}

#[test]
fn cells_without_labels_are_named_by_their_contents() -> Result<()> {
    super::scope(
        "Cell name test",
        GridActivationHandler,
        NullActionHandler,
        |s| {
            let names = cell_names(s)?;
            // The row (a DataItem too) has no name of its own; the cells do.
            assert!(names.contains(&"#1001".to_string()), "{names:?}");
            assert!(names.contains(&"Customer One".to_string()), "{names:?}");
            assert!(names.contains(&"Labelled".to_string()), "{names:?}");
            assert!(names.contains(&"$3.70".to_string()), "{names:?}");
            assert!(
                !names.iter().any(|name| name.contains("Ignored")),
                "{names:?}"
            );
            Ok(())
        },
    )
}
