use super::*;
use dioxus::core::DynamicNode;
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use crate::{
    automation_runtime::{EventSourceState, FocusInput, RuntimeInputs},
    components::PublishedConfigContext,
    config::{Automation, ConfigCoordinator, ConfigStore, Device},
    focused_window::ForegroundObservation,
    hid::{HidBackend, HidError, HidInventory},
    platform::autostart::{LaunchAtLogin, LaunchAtLoginOutcome},
};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    directory: PathBuf,
    coordinator: Arc<ConfigCoordinator>,
}

struct ConfirmedStartup;

impl LaunchAtLogin for ConfirmedStartup {
    fn reconcile(&self, desired: bool) -> LaunchAtLoginOutcome {
        LaunchAtLoginOutcome::confirmed(desired)
    }
}

impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "locked-in-action-consumer-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        let coordinator = Arc::new(
            ConfigCoordinator::initial_load(
                Arc::new(ConfigStore::new(directory.join("config.toml"))),
                Arc::new(ConfirmedStartup),
            )
            .unwrap(),
        );
        Self {
            directory,
            coordinator,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

struct NoHardware;

impl HidBackend for NoHardware {
    fn inventory(&self) -> HidInventory {
        HidInventory::default()
    }

    fn refresh(&mut self) -> HidInventory {
        self.inventory()
    }

    fn send_report(&mut self, _: &Device, _: &[u8]) -> Result<(), HidError> {
        panic!("rendering destinations must not dispatch HID reports")
    }
}

fn action_editor_host() -> Element {
    consume_context::<Arc<AtomicUsize>>().fetch_add(1, Ordering::Relaxed);
    let coordinator = consume_context::<Arc<ConfigCoordinator>>();
    let publication = use_signal(|| Some(coordinator.current()));
    let close = use_signal(|| dioxus::desktop::WindowCloseBehaviour::WindowCloses);
    use_context_provider(|| PublishedConfigContext::new(publication, close, false));
    let inventory = use_signal(|| Arc::new(HidInventory::default()));
    use_context_provider(|| HidInventoryContext::new(inventory));
    let draft = use_signal(|| AutomationDraft::create(1, Automation::default()));
    rsx! {
        ActionEditor {
            draft,
            case_index: None,
            action_index: 0,
            action: SendAction { id: "test-action".into(), ..SendAction::default() },
        }
    }
}

fn editor_scope(dom: &VirtualDom) -> ScopeId {
    let root = dom.get_scope(ScopeId::APP).unwrap().root_node();
    root.dynamic_nodes
        .iter()
        .enumerate()
        .find_map(|(index, node)| {
            if let DynamicNode::Component(component) = node {
                component.mounted_scope_id(index, root, dom)
            } else {
                None
            }
        })
        .unwrap()
}

fn dynamic_text(node: &VNode) -> Vec<String> {
    node.dynamic_nodes
        .iter()
        .flat_map(|node| match node {
            DynamicNode::Text(text) => vec![text.value.clone()],
            DynamicNode::Fragment(children) => children.iter().flat_map(dynamic_text).collect(),
            _ => Vec::new(),
        })
        .collect()
}

#[test]
fn mounted_action_destinations_follow_durable_publication_without_parent_rerender() {
    let fixture = Fixture::new();
    let initial = fixture.coordinator.current();
    let (_focus_sender, focus_receiver) =
        tokio::sync::watch::channel(ForegroundObservation::default());
    let (runtime, owner) = AutomationRuntime::start(
        Some(fixture.coordinator.subscribe()),
        RuntimeInputs {
            focused_window: FocusInput::new(focus_receiver, EventSourceState::Available),
        },
        NoHardware,
    )
    .unwrap();
    let parent_renders = Arc::new(AtomicUsize::new(0));
    let mut dom = VirtualDom::new(action_editor_host);
    dom.provide_root_context(fixture.coordinator.clone());
    dom.provide_root_context(runtime);
    dom.provide_root_context(parent_renders.clone());
    dom.rebuild_in_place();
    dom.render_immediate_to_vec();
    let mounted_editor = editor_scope(&dom);
    let initial_parent_renders = parent_renders.load(Ordering::Relaxed);

    let mut candidate = initial.editable().as_ref().clone();
    candidate.devices.push(Device {
        id: "deck".into(),
        name: "Unsaved deck".into(),
        report_length: 32,
        ..Device::default()
    });
    assert!(
        !dynamic_text(dom.get_scope(mounted_editor).unwrap().root_node())
            .contains(&"Unsaved deck".into())
    );

    candidate.devices[0].name = "Saved deck".into();
    let saved = fixture
        .coordinator
        .update(initial.revision(), candidate)
        .unwrap();
    dom.in_scope(ScopeId::APP, || {
        consume_context::<PublishedConfigContext>().acknowledge(saved.clone())
    });
    dom.render_immediate_to_vec();
    assert!(
        dynamic_text(dom.get_scope(mounted_editor).unwrap().root_node())
            .contains(&"Saved deck".into())
    );

    let mut renamed = saved.editable().as_ref().clone();
    renamed.devices[0].name = "Renamed deck".into();
    let renamed = fixture
        .coordinator
        .update(saved.revision(), renamed)
        .unwrap();
    dom.in_scope(ScopeId::APP, || {
        let context = consume_context::<PublishedConfigContext>();
        context.acknowledge(renamed.clone());
        context.acknowledge(saved);
    });
    dom.render_immediate_to_vec();
    let text = dynamic_text(dom.get_scope(mounted_editor).unwrap().root_node());
    assert!(text.contains(&"Renamed deck".into()));
    assert!(!text.contains(&"Saved deck".into()));

    let mut removed = renamed.editable().as_ref().clone();
    removed.devices.clear();
    let removed = fixture
        .coordinator
        .update(renamed.revision(), removed)
        .unwrap();
    dom.in_scope(ScopeId::APP, || {
        consume_context::<PublishedConfigContext>().acknowledge(removed)
    });
    dom.render_immediate_to_vec();
    assert!(
        !dynamic_text(dom.get_scope(mounted_editor).unwrap().root_node())
            .contains(&"Renamed deck".into())
    );
    assert_eq!(editor_scope(&dom), mounted_editor);
    assert_eq!(
        parent_renders.load(Ordering::Relaxed),
        initial_parent_renders
    );
    drop(dom);
    owner.shutdown_and_join(Duration::from_secs(1));
}

#[test]
fn mixed_test_feedback_reports_successes_and_failures() {
    let feedback = test_feedback(TestDispatchResult {
        sent: 1,
        failures: vec!["Deck: disconnected".into(), "Pad: ambiguous".into()],
    });

    assert_eq!(
        feedback,
        (
            false,
            "Sent to 1 device; 2 failed: Deck: disconnected; Pad: ambiguous".into()
        )
    );
}
