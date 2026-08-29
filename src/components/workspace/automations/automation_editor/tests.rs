use super::*;

#[test]
fn first_edit_is_not_replaced_by_a_new_publication_revision() {
    let base = Automation {
        id: "automation-1".into(),
        name: "Durable".into(),
        ..Automation::default()
    };
    let mut draft = base.clone();
    draft.name = "First edit".into();
    let published = Automation {
        name: "New publication".into(),
        ..base.clone()
    };

    let update = clean_editor_publication(2, &[published], &base.id, false, 1, &base, &draft);

    assert_eq!(update, None);
    assert_eq!(draft.name, "First edit");
    assert_eq!(base.name, "Durable");
}

#[test]
fn clean_editor_advances_to_a_new_publication_revision() {
    let base = Automation {
        id: "automation-1".into(),
        name: "Durable".into(),
        ..Automation::default()
    };
    let published = Automation {
        name: "Published update".into(),
        ..base.clone()
    };

    let update = clean_editor_publication(
        2,
        std::slice::from_ref(&published),
        &base.id,
        false,
        1,
        &base,
        &base,
    );

    assert_eq!(update, Some(published));
}
