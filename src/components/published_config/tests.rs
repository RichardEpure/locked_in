use super::*;
use crate::config::{Device, EditableConfig};

#[test]
fn acknowledgement_is_immediate_and_older_delivery_cannot_regress_any_reader_or_close_policy() {
    let mut dom = VirtualDom::new(VNode::empty);
    dom.rebuild_in_place();
    dom.in_scope(ScopeId::ROOT, || {
        let initial = PublishedConfig::prepare_for_test(EditableConfig::default(), 1).unwrap();
        let publication = Signal::new(Some(initial.clone()));
        let close = Signal::new(WindowCloseBehaviour::WindowCloses);
        let context = PublishedConfigContext::new(publication, close, true);
        assert_eq!(*close.peek(), WindowCloseBehaviour::WindowHides);
        let mut candidate = initial.editable().as_ref().clone();
        candidate.settings.close_to_tray = false;
        candidate.devices.push(Device {
            id: "new".into(),
            name: "Saved".into(),
            report_length: 32,
            ..Device::default()
        });
        let saved = PublishedConfig::prepare_for_test(candidate, 2).unwrap();
        context.acknowledge(saved.clone());
        // The parent can clear its creation draft now: the saved device is already
        // visible to both the list and editor through this shared context.
        let reader = context;
        assert_eq!(reader.required().editable().devices[0].id, "new");
        context.acknowledge(initial);
        context.acknowledge(saved.clone());
        assert!(Arc::ptr_eq(&reader.required(), &saved));
        assert_eq!(*close.peek(), WindowCloseBehaviour::WindowCloses);
    });
}
