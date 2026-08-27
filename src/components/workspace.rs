use std::{collections::HashSet, process::Command};

use dioxus::{desktop::use_window, prelude::*};

use crate::{
    CAPTURE_TARGET_SIGNAL, CAPTURED_WINDOW_SIGNAL, CONFIG_LOAD_ERROR, CONFIG_REVISION_SIGNAL,
    CONFIG_SIGNAL, CaptureTarget, DIRTY_EDITOR_SIGNAL, SERVICE_READY, UNSAVED_ENTITY_SIGNAL,
    app_log, arm_capture, cancel_capture,
    config::{
        self, Automation, AutomationCase, Device, LogLevel, MatchOperator, SendAction,
        TextCondition, WindowMatcher,
    },
    hid,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Automations,
    Devices,
    Settings,
}

static INVALID_REPORT_IDS: GlobalSignal<HashSet<String>> = Signal::global(HashSet::new);

#[component]
pub fn Workspace() -> Element {
    if let Some(error) = CONFIG_LOAD_ERROR.as_ref() {
        return rsx! {
            div { class: "load-error",
                div { class: "brand", span { class: "brand__mark", "LI" } span { "Locked In" } }
                section { h1 { "Configuration could not be loaded" }
                    p { "Locked In will not overwrite this file. Convert it to schema version 2, then restart the application." }
                    pre { "{error}" }
                    button { class: "button secondary", onclick: move |_| if let Ok(path) = config::config_path() { let _ = Command::new("notepad.exe").arg(path).spawn(); }, "Open config file" }
                }
            }
        };
    }
    let mut section = use_signal(|| Section::Automations);
    let selected_automation = use_signal(|| None::<String>);
    let selected_device = use_signal(|| None::<String>);
    let service_ready = SERVICE_READY.load(std::sync::atomic::Ordering::Relaxed);
    let navigation_locked =
        DIRTY_EDITOR_SIGNAL.read().is_some() || CAPTURE_TARGET_SIGNAL.read().is_some();

    rsx! {
        div {
            class: "app-shell",
            tabindex: "0",
            autofocus: true,
            onkeydown: move |event| {
                let key = event.key().to_string().to_lowercase();
                if event.modifiers().contains(Modifiers::CONTROL) {
                    let script = match key.as_str() {
                        "n" => Some("document.querySelector('[aria-label^=\"New\"].icon-button.primary')?.click()"),
                        "s" => Some("document.querySelector('.save-bar .button.primary:not([disabled])')?.click()"),
                        "f" => Some("document.querySelector('.search')?.focus()"),
                        _ => None,
                    };
                    if let Some(script) = script {
                        event.prevent_default();
                        spawn(async move { let _ = document::eval(script).await; });
                    }
                } else if key == "escape" {
                    spawn(async move { let _ = document::eval("const button = document.querySelector('.modal-backdrop [aria-label=\"Close\"]') || document.querySelector('.save-bar .button.ghost:not([disabled])'); button?.click()").await; });
                } else if key == "delete" {
                    spawn(async move { let _ = document::eval("if (!['INPUT','SELECT','TEXTAREA'].includes(document.activeElement?.tagName)) document.querySelector('.workspace-header .danger-ghost')?.click()").await; });
                }
            },
            nav {
                class: "app-nav",
                div { class: "brand", span { class: "brand__mark", "LI" } span { "Locked In" } }
                button {
                    class: if section() == Section::Automations { "nav-item active" } else { "nav-item" },
                    disabled: navigation_locked && section() != Section::Automations,
                    onclick: move |_| section.set(Section::Automations),
                    span { class: "nav-item__icon", "A" }
                    span { class: "nav-item__label", "Automations" }
                }
                button {
                    class: if section() == Section::Devices { "nav-item active" } else { "nav-item" },
                    disabled: navigation_locked && section() != Section::Devices,
                    onclick: move |_| section.set(Section::Devices),
                    span { class: "nav-item__icon", "D" }
                    span { class: "nav-item__label", "Devices" }
                }
                button {
                    class: if section() == Section::Settings { "nav-item active" } else { "nav-item" },
                    disabled: navigation_locked && section() != Section::Settings,
                    onclick: move |_| section.set(Section::Settings),
                    span { class: "nav-item__icon", "S" }
                    span { class: "nav-item__label", "Settings" }
                }
                div { class: "app-nav__status", span { class: if service_ready { "status-dot online" } else { "status-dot error" } } if service_ready { "Automation service active" } else { "Automation service unavailable" } }
            }
            match section() {
                Section::Automations => rsx! { AutomationsView { selected: selected_automation } },
                Section::Devices => rsx! { DevicesView { selected: selected_device } },
                Section::Settings => rsx! { SettingsView {} },
            }
            if CAPTURED_WINDOW_SIGNAL.read().is_some() && CAPTURE_TARGET_SIGNAL.read().is_none() {
                CaptureDialog {}
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct SelectionProps {
    selected: Signal<Option<String>>,
}

#[component]
fn AutomationsView(props: SelectionProps) -> Element {
    let mut selected = props.selected;
    let mut query = use_signal(String::new);
    let pending_delete = use_signal(|| None::<String>);
    let automations = CONFIG_SIGNAL.read().automations.clone();
    let normalized_query = query().to_lowercase();
    let revision = *CONFIG_REVISION_SIGNAL.read();
    let navigation_locked =
        DIRTY_EDITOR_SIGNAL.read().is_some() || CAPTURE_TARGET_SIGNAL.read().is_some();

    rsx! {
        aside {
            class: "entity-list",
            header {
                div { h1 { "Automations" } p { "Ordered event decisions" } }
                button {
                    class: "icon-button primary",
                    aria_label: "New automation",
                    title: "New automation (Ctrl+N)",
                    disabled: navigation_locked,
                    onclick: move |_| {
                        let mut config = CONFIG_SIGNAL.read().clone();
                        let id = config.next_id("automation");
                        config.automations.push(Automation { id: id.clone(), ..Automation::default() });
                        *CONFIG_SIGNAL.write() = config;
                        *UNSAVED_ENTITY_SIGNAL.write() = Some(format!("automation:{id}"));
                        selected.set(Some(id));
                    },
                    "+"
                }
            }
            input {
                class: "search",
                placeholder: "Search automations",
                value: "{query}",
                oninput: move |event| query.set(event.value()),
            }
            div {
                class: "entity-list__items",
                for automation in automations.iter().filter(|item| item.name.to_lowercase().contains(&normalized_query)).cloned() {
                    button {
                        key: "{automation.id}",
                        class: if selected().as_deref() == Some(&automation.id) { "entity-row selected" } else { "entity-row" },
                        disabled: navigation_locked && selected().as_deref() != Some(&automation.id),
                        onclick: {
                            let id = automation.id.clone();
                            move |_| selected.set(Some(id.clone()))
                        },
                        span { class: if automation.enabled { "status-dot online" } else { "status-dot" } }
                        span { class: "entity-row__copy", strong { "{automation.name}" } small { "{automation.cases.len()} cases" } }
                    }
                }
            }
        }
        section {
            class: "workspace",
            if let Some(id) = selected().filter(|id| automations.iter().any(|automation| automation.id == *id)) {
                AutomationEditor { key: "{id}-{revision}", id, selected, pending_delete }
            } else {
                EmptyState { title: "Select an automation", copy: "Create or select an automation to configure its event, ordered cases, and report routes." }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct AutomationEditorProps {
    id: String,
    selected: Signal<Option<String>>,
    pending_delete: Signal<Option<String>>,
}

#[component]
fn AutomationEditor(props: AutomationEditorProps) -> Element {
    let id = props.id.clone();
    let mut selected = props.selected;
    let mut pending_delete = props.pending_delete;
    let original = CONFIG_SIGNAL
        .read()
        .automations
        .iter()
        .find(|item| item.id == id)
        .cloned()
        .unwrap_or_else(|| Automation {
            id: id.clone(),
            ..Automation::default()
        });
    let mut draft = use_signal(|| original.clone());
    let mut message = use_signal(|| None::<(bool, String)>);
    let snapshot = draft();
    let editor_token = format!("automation:{id}");
    let is_new = UNSAVED_ENTITY_SIGNAL.read().as_deref() == Some(editor_token.as_str());
    let dirty = snapshot != original || is_new;
    let invalid_report_ids = INVALID_REPORT_IDS.read();
    let has_invalid_report = snapshot
        .cases
        .iter()
        .flat_map(|case| &case.actions)
        .chain(&snapshot.otherwise_actions)
        .any(|action| invalid_report_ids.contains(&action.id));
    let capture_automation_id = id.clone();
    use_effect(move || {
        let Some(target) = CAPTURE_TARGET_SIGNAL.read().clone() else {
            return;
        };
        let Some(captured) = CAPTURED_WINDOW_SIGNAL.read().clone() else {
            return;
        };
        if target.automation_id != capture_automation_id {
            return;
        }
        let mut automation = draft.write();
        let Some(case) = automation
            .cases
            .iter_mut()
            .find(|case| case.id == target.case_id)
        else {
            drop(automation);
            *CAPTURE_TARGET_SIGNAL.write() = None;
            return;
        };
        let matcher_id = next_child_id(
            "captured",
            case.applications
                .iter()
                .chain(&case.exceptions)
                .map(|matcher| matcher.id.as_str()),
        );
        let list = if target.exception {
            &mut case.exceptions
        } else {
            &mut case.applications
        };
        list.push(WindowMatcher {
            id: matcher_id,
            title: captured.title.map(TextCondition::contains),
            class: captured.class.map(TextCondition::contains),
            exe: captured
                .exe
                .map(|path| TextCondition::equals(path.to_string_lossy())),
        });
        drop(automation);
        *CAPTURE_TARGET_SIGNAL.write() = None;
        *CAPTURED_WINDOW_SIGNAL.write() = None;
    });
    let effect_token = editor_token.clone();
    let effect_original = original.clone();
    use_effect(move || {
        let pending = UNSAVED_ENTITY_SIGNAL.read().as_deref() == Some(effect_token.as_str());
        if draft() != effect_original || pending {
            *DIRTY_EDITOR_SIGNAL.write() = Some(effect_token.clone());
        } else if DIRTY_EDITOR_SIGNAL.read().as_deref() == Some(effect_token.as_str()) {
            *DIRTY_EDITOR_SIGNAL.write() = None;
        }
    });
    let cleanup_token = editor_token.clone();
    let cleanup_id = id.clone();
    use_drop(move || {
        if DIRTY_EDITOR_SIGNAL.read().as_deref() == Some(cleanup_token.as_str()) {
            *DIRTY_EDITOR_SIGNAL.write() = None;
        }
        if CAPTURE_TARGET_SIGNAL
            .read()
            .as_ref()
            .is_some_and(|target| target.automation_id == cleanup_id)
        {
            cancel_capture();
        }
    });
    let cancel_original = original.clone();
    let cancel_id = id.clone();
    let cancel_token = editor_token.clone();

    rsx! {
        header {
            class: "workspace-header",
            div {
                div { class: "eyebrow", "FOCUSED WINDOW AUTOMATION" }
                h2 { "{snapshot.name}" if dirty { span { class: "dirty-dot", title: "Unsaved changes", "•" } } }
                p { "First matching case runs. Otherwise is used only when no case matches." }
            }
            div { class: "toolbar",
                button {
                    class: "button ghost",
                    disabled: dirty,
                    onclick: move |_| {
                        let mut config = CONFIG_SIGNAL.read().clone();
                        let mut copy = draft();
                        copy.id = config.next_id(&format!("{}-copy", copy.id));
                        copy.name = format!("{} Copy", copy.name);
                        copy.enabled = false;
                        let copy_id = copy.id.clone();
                        config.automations.push(copy);
                        *CONFIG_SIGNAL.write() = config;
                        *UNSAVED_ENTITY_SIGNAL.write() = Some(format!("automation:{copy_id}"));
                        selected.set(Some(copy_id));
                    },
                    "Duplicate"
                }
                button {
                    class: "button danger-ghost",
                    onclick: {
                        let id = props.id.clone();
                        move |_| {
                            if pending_delete().as_deref() == Some(&id) {
                                let mut next = CONFIG_SIGNAL.read().clone();
                                next.automations.retain(|item| item.id != id);
                                match next.save() {
                                    Ok(()) => { *CONFIG_SIGNAL.write() = next; *DIRTY_EDITOR_SIGNAL.write() = None; *UNSAVED_ENTITY_SIGNAL.write() = None; pending_delete.set(None); selected.set(None); }
                                    Err(error) => message.set(Some((false, format!("Delete failed: {error}")))),
                                }
                            } else {
                                pending_delete.set(Some(id.clone()));
                                message.set(Some((false, "Click Delete again to confirm".into())));
                            }
                        }
                    },
                    "Delete"
                }
            }
        }
        div {
            class: "editor-scroll",
            section { class: "editor-card overview-card",
                div { class: "section-heading", span { class: "step", "01" } div { h3 { "Automation" } p { "Identity and activation state" } } }
                div { class: "form-grid two",
                    label { "Name" input { value: "{snapshot.name}", oninput: move |event| draft.write().name = event.value() } }
                    label { class: "toggle-field", span { "Enabled" } input { type: "checkbox", checked: snapshot.enabled, onchange: move |event| draft.write().enabled = event.checked() } small { "Incomplete disabled automations can be saved as drafts." } }
                }
            }
            section { class: "editor-card trigger-card",
                div { class: "section-heading", span { class: "step", "02" } div { h3 { "When" } p { "The event that starts evaluation" } } }
                div { class: "trigger-summary", span { class: "trigger-icon", "W" } div { strong { "Focused window changes" } small { "Evaluate title, class, and executable metadata" } } span { class: "pill", "Windows" } }
            }
            section { class: "editor-card",
                div { class: "section-heading split", span { class: "step", "03" } div { h3 { "Cases" } p { "Evaluated from top to bottom; first match wins" } }
                    button { class: "button secondary", onclick: move |_| add_case(&mut draft), "+ Add case" }
                }
                if snapshot.cases.is_empty() {
                    div { class: "inline-empty", "No cases yet. Add a case or use an Otherwise action." }
                }
                for (case_index, case) in snapshot.cases.iter().cloned().enumerate() {
                    CaseEditor { key: "{case.id}", draft, case_index, case }
                }
            }
            section { class: "editor-card otherwise-card",
                div { class: "section-heading split", span { class: "step muted", "ELSE" } div { h3 { "Otherwise" } p { "Runs only when no case matches" } }
                    button { class: "button secondary", onclick: move |_| add_action(&mut draft, None), "+ Add action" }
                }
                for (action_index, action) in snapshot.otherwise_actions.iter().cloned().enumerate() {
                    ActionEditor { key: "{action.id}", draft, case_index: None, action_index, action }
                }
                if snapshot.otherwise_actions.is_empty() { div { class: "inline-empty compact", "Optional. Leave empty to do nothing when no cases match." } }
            }
        }
        footer { class: "save-bar",
            if has_invalid_report { span { class: "message error", "Complete or correct every hexadecimal report before saving" } }
            else if let Some((success, text)) = message() { span { class: if success { "message success" } else { "message error" }, "{text}" } } else { span {} }
            div { class: "toolbar",
                button { class: "button ghost", disabled: !dirty, onclick: move |_| {
                    if UNSAVED_ENTITY_SIGNAL.read().as_deref() == Some(cancel_token.as_str()) {
                        CONFIG_SIGNAL.write().automations.retain(|automation| automation.id != cancel_id);
                        *UNSAVED_ENTITY_SIGNAL.write() = None;
                        *DIRTY_EDITOR_SIGNAL.write() = None;
                        selected.set(None);
                    } else {
                        draft.set(cancel_original.clone());
                    }
                }, "Cancel" }
                button {
                    class: "button primary",
                    disabled: !dirty || has_invalid_report,
                    onclick: move |_| {
                        let mut next = CONFIG_SIGNAL.read().clone();
                        if let Some(index) = next.automations.iter().position(|item| item.id == props.id) {
                            next.automations[index] = draft();
                        }
                        let errors = next.validate();
                        if errors.is_empty() {
                            match next.save() {
                                Ok(()) => {
                                    *CONFIG_SIGNAL.write() = next;
                                    *UNSAVED_ENTITY_SIGNAL.write() = None;
                                    *DIRTY_EDITOR_SIGNAL.write() = None;
                                    message.set(Some((true, "Automation saved".into())));
                                }
                                Err(error) => message.set(Some((false, format!("Save failed: {error}")))),
                            }
                        } else {
                            let text = errors.iter().take(3).map(|error| error.message.as_str()).collect::<Vec<_>>().join("; ");
                            message.set(Some((false, text)));
                        }
                    },
                    "Save automation"
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct CaseEditorProps {
    draft: Signal<Automation>,
    case_index: usize,
    case: AutomationCase,
}

#[component]
fn CaseEditor(props: CaseEditorProps) -> Element {
    let mut draft = props.draft;
    let case_index = props.case_index;
    let case = props.case;
    let case_count = draft.read().cases.len();
    let priority = case_index + 1;
    rsx! {
        article { class: "case-card",
            header { class: "case-card__header",
                span { class: "priority", "{priority}" }
                input { class: "case-name", aria_label: "Case name", placeholder: "Case name", value: "{case.name}", oninput: move |event| draft.write().cases[case_index].name = event.value() }
                div { class: "toolbar tight",
                    button { class: "icon-button", title: "Move up", disabled: case_index == 0, onclick: move |_| draft.write().cases.swap(case_index, case_index - 1), "↑" }
                    button { class: "icon-button", title: "Move down", disabled: case_index + 1 == case_count, onclick: move |_| draft.write().cases.swap(case_index, case_index + 1), "↓" }
                    button { class: "icon-button danger", title: "Delete case", onclick: move |_| { draft.write().cases.remove(case_index); }, "×" }
                }
            }
            div { class: "case-card__body",
                MatcherGroup { draft, case_index, exceptions: false, matchers: case.applications.clone() }
                MatcherGroup { draft, case_index, exceptions: true, matchers: case.exceptions.clone() }
                div { class: "actions-heading", div { h4 { "Send" } p { "One report per action, routed to selected devices" } }
                    button { class: "button secondary small", onclick: move |_| add_action(&mut draft, Some(case_index)), "+ Add action" }
                }
                for (action_index, action) in case.actions.iter().cloned().enumerate() {
                    ActionEditor { key: "{action.id}", draft, case_index: Some(case_index), action_index, action }
                }
                if case.actions.is_empty() { div { class: "inline-empty compact", "No report actions configured." } }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct MatcherGroupProps {
    draft: Signal<Automation>,
    case_index: usize,
    exceptions: bool,
    matchers: Vec<WindowMatcher>,
}

#[component]
fn MatcherGroup(props: MatcherGroupProps) -> Element {
    let mut draft = props.draft;
    let case_index = props.case_index;
    let exceptions = props.exceptions;
    let title = if exceptions {
        "Except when"
    } else {
        "Applications"
    };
    let copy = if exceptions {
        "Any match here skips this case"
    } else {
        "Any application may match; populated fields are ANDed"
    };
    rsx! {
        div { class: if exceptions { "matcher-group exceptions" } else { "matcher-group" },
            div { class: "matcher-group__heading", div { h4 { "{title}" } p { "{copy}" } }
                div { class: "toolbar tight",
                    button { class: "button ghost small", onclick: move |_| {
                        let automation = draft.read();
                        arm_capture(Some(CaptureTarget::new(automation.id.clone(), automation.cases[case_index].id.clone(), exceptions)));
                    }, "Capture next (F3)" }
                    button { class: "button secondary small", onclick: move |_| add_matcher(&mut draft, case_index, exceptions), "+ Add matcher" }
                }
            }
            for (matcher_index, matcher) in props.matchers.iter().cloned().enumerate() {
                MatcherEditor { key: "{matcher.id}", draft, case_index, exceptions, matcher_index, matcher }
            }
            if props.matchers.is_empty() && exceptions { div { class: "inline-empty compact", "No exceptions." } }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct MatcherEditorProps {
    draft: Signal<Automation>,
    case_index: usize,
    exceptions: bool,
    matcher_index: usize,
    matcher: WindowMatcher,
}

#[component]
fn MatcherEditor(props: MatcherEditorProps) -> Element {
    let mut draft = props.draft;
    let case_index = props.case_index;
    let exceptions = props.exceptions;
    let matcher_index = props.matcher_index;
    let matcher = props.matcher;
    rsx! {
        div { class: "matcher-card",
            ConditionRow { draft, case_index, exceptions, matcher_index, field: "title", label: "Window title", condition: matcher.title.clone(), default_operator: MatchOperator::Contains }
            ConditionRow { draft, case_index, exceptions, matcher_index, field: "class", label: "Window class", condition: matcher.class.clone(), default_operator: MatchOperator::Contains }
            ConditionRow { draft, case_index, exceptions, matcher_index, field: "exe", label: "Executable", condition: matcher.exe.clone(), default_operator: MatchOperator::Equals }
            button { class: "icon-button danger matcher-remove", title: "Remove matcher", onclick: move |_| {
                let case = &mut draft.write().cases[case_index];
                if exceptions { case.exceptions.remove(matcher_index); } else { case.applications.remove(matcher_index); }
            }, "×" }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct ConditionRowProps {
    draft: Signal<Automation>,
    case_index: usize,
    exceptions: bool,
    matcher_index: usize,
    field: &'static str,
    label: &'static str,
    condition: Option<TextCondition>,
    default_operator: MatchOperator,
}

#[component]
fn ConditionRow(props: ConditionRowProps) -> Element {
    let mut draft = props.draft;
    let value = props
        .condition
        .as_ref()
        .map(|condition| condition.value.clone())
        .unwrap_or_default();
    let operator = props
        .condition
        .as_ref()
        .map_or(props.default_operator, |condition| condition.operator);
    let case_sensitive = props
        .condition
        .as_ref()
        .is_some_and(|condition| condition.case_sensitive);
    let operator_props = props.clone();
    let value_props = props.clone();
    let case_props = props.clone();
    rsx! {
        div { class: "condition-row",
            span { class: "condition-label", "{props.label}" }
            select { value: operator_name(operator), onchange: move |event| update_condition(&mut draft, &operator_props, Some(parse_operator(&event.value())), None, None),
                option { value: "contains", "contains" }
                option { value: "equals", "equals" }
                option { value: "regex", "regex" }
            }
            input { placeholder: "Not used", value: "{value}", oninput: move |event| update_condition(&mut draft, &value_props, None, Some(event.value()), None) }
            label { class: "case-check", input { type: "checkbox", checked: case_sensitive, onchange: move |event| update_condition(&mut draft, &case_props, None, None, Some(event.checked())) } "Aa" }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct ActionEditorProps {
    draft: Signal<Automation>,
    case_index: Option<usize>,
    action_index: usize,
    action: SendAction,
}

#[component]
fn ActionEditor(props: ActionEditorProps) -> Element {
    let mut draft = props.draft;
    let action = props.action;
    let devices = CONFIG_SIGNAL.read().devices.clone();
    let mut test_result = use_signal(|| None::<(bool, String)>);
    let mut report_input = use_signal(|| hex::encode(&action.report));
    let report_text = report_input();
    let parsed_report = parse_report_hex(&report_text);
    let report_length = parsed_report.as_ref().map_or(action.report.len(), Vec::len);
    let action_id = action.id.clone();
    let effect_action_id = action_id.clone();
    use_effect(move || {
        if parse_report_hex(&report_input()).is_ok() {
            INVALID_REPORT_IDS.write().remove(&effect_action_id);
        } else {
            INVALID_REPORT_IDS.write().insert(effect_action_id.clone());
        }
    });
    use_drop(move || {
        INVALID_REPORT_IDS.write().remove(&action_id);
    });
    rsx! {
        div { class: "action-card",
            div { class: "action-card__top",
                input { class: "action-label", placeholder: "Optional action label", value: "{action.label}", oninput: move |event| with_action_mut(&mut draft, props.case_index, props.action_index, |action| action.label = event.value()) }
                button { class: "button secondary small", onclick: {
                    let mut action = action.clone();
                    move |_| {
                        let Ok(report) = parse_report_hex(&report_input()) else {
                            test_result.set(Some((false, "Report must contain complete hexadecimal bytes".into())));
                            return;
                        };
                        action.report = report;
                        let config = CONFIG_SIGNAL.read();
                        let validation_errors = config.validate_action(&action);
                        if !validation_errors.is_empty() {
                            test_result.set(Some((false, validation_errors.iter().map(|error| error.message.as_str()).collect::<Vec<_>>().join("; "))));
                            return;
                        }
                        let mut failures = Vec::new();
                        let mut sent = 0;
                        for device_id in &action.device_ids {
                            if let Some(device) = config.devices.iter().find(|device| device.id == *device_id) {
                                match device.send_report(&action.report) { Ok(_) => sent += 1, Err(error) => failures.push(format!("{}: {error}", device.name)) }
                            }
                        }
                        if failures.is_empty() && sent > 0 { test_result.set(Some((true, format!("Sent to {sent} device(s)")))); }
                        else { test_result.set(Some((false, if failures.is_empty() { "Select a destination".into() } else { failures.join("; ") }))); }
                    }
                }, "Test" }
                button { class: "icon-button danger", title: "Remove action", onclick: move |_| remove_action(&mut draft, props.case_index, props.action_index), "×" }
            }
            div { class: "action-fields",
                label { "Report (hex)" div { class: if parsed_report.is_ok() { "hex-input" } else { "hex-input invalid" }, span { "0x" } input { value: "{report_text}", placeholder: "87", oninput: move |event| {
                    let value = event.value();
                    report_input.set(value.clone());
                    if let Ok(bytes) = parse_report_hex(&value) { with_action_mut(&mut draft, props.case_index, props.action_index, |action| action.report = bytes); }
                } } small { if parsed_report.is_ok() { "{report_length} bytes" } else { "invalid" } } } }
                div { class: "destinations", span { class: "field-label", "Destinations" }
                    if devices.is_empty() { small { class: "muted-copy", "Add a device first" } }
                    for device in devices {
                        label { class: "destination-chip",
                            input { type: "checkbox", checked: action.device_ids.contains(&device.id), onchange: {
                                let id = device.id.clone();
                                move |event| {
                                    with_action_mut(&mut draft, props.case_index, props.action_index, |action| {
                                        if event.checked() && !action.device_ids.contains(&id) { action.device_ids.push(id.clone()); }
                                        else if !event.checked() { action.device_ids.retain(|item| item != &id); }
                                    });
                                }
                            } }
                            span { class: if hid::is_connected(&device) { "status-dot online" } else { "status-dot" } }
                            "{device.name}"
                            small { "{device.report_length} B" }
                        }
                    }
                }
            }
            if let Some((success, text)) = test_result() { small { class: if success { "message success" } else { "message error" }, "{text}" } }
        }
    }
}

#[component]
fn DevicesView(props: SelectionProps) -> Element {
    let mut selected = props.selected;
    let mut query = use_signal(String::new);
    let mut discovered = use_signal(hid::discovered_interfaces);
    let devices = CONFIG_SIGNAL.read().devices.clone();
    let discovered_count = discovered().len();
    let normalized_query = query().to_lowercase();
    let navigation_locked =
        DIRTY_EDITOR_SIGNAL.read().is_some() || CAPTURE_TARGET_SIGNAL.read().is_some();
    rsx! {
        aside { class: "entity-list",
            header { div { h1 { "Devices" } p { "Reusable HID destinations" } }
                button { class: "icon-button primary", aria_label: "New device", disabled: navigation_locked, onclick: move |_| {
                    let mut config = CONFIG_SIGNAL.read().clone();
                    let id = config.next_id("device");
                    config.devices.push(Device { id: id.clone(), name: "New device".into(), report_length: 32, ..Device::default() });
                    *CONFIG_SIGNAL.write() = config;
                    *UNSAVED_ENTITY_SIGNAL.write() = Some(format!("device:{id}"));
                    selected.set(Some(id));
                }, "+" }
            }
            input { class: "search", placeholder: "Search devices", value: "{query}", oninput: move |event| query.set(event.value()) }
            button { class: "discovery-refresh", onclick: move |_| discovered.set(hid::discovered_interfaces()),
                span { "Connected interfaces" } small { "{discovered_count} found · Refresh" }
            }
            if !discovered().is_empty() {
                details { class: "discovery-list",
                    summary { "Adopt connected interface" }
                    for interface in discovered() {
                        button { class: "discovery-row", disabled: navigation_locked, onclick: move |_| {
                            let mut config = CONFIG_SIGNAL.read().clone();
                            if let Some(existing) = config.devices.iter().find(|device|
                                device.vid == interface.vendor_id && device.pid == interface.product_id
                                    && device.usage_page == interface.usage_page && device.usage == interface.usage
                            ) {
                                selected.set(Some(existing.id.clone()));
                            } else {
                                let id = config.next_id(&interface.name);
                                config.devices.push(Device {
                                    id: id.clone(), name: interface.name.clone(), vid: interface.vendor_id,
                                    pid: interface.product_id, usage_page: interface.usage_page,
                                    usage: interface.usage, report_length: 32, report_id: 0,
                                });
                                *CONFIG_SIGNAL.write() = config;
                                *UNSAVED_ENTITY_SIGNAL.write() = Some(format!("device:{id}"));
                                selected.set(Some(id));
                            }
                        },
                            strong { "{interface.name}" }
                            small { "{interface.vendor_id:04X}:{interface.product_id:04X} · {interface.usage_page}:{interface.usage}" }
                        }
                    }
                }
            }
            div { class: "entity-list__items",
                for device in devices.iter().filter(|device| device.name.to_lowercase().contains(&normalized_query)).cloned() {
                    button { key: "{device.id}", class: if selected().as_deref() == Some(&device.id) { "entity-row selected" } else { "entity-row" }, disabled: navigation_locked && selected().as_deref() != Some(&device.id), onclick: {
                        let id = device.id.clone(); move |_| selected.set(Some(id.clone()))
                    }, span { class: if hid::is_connected(&device) { "status-dot online" } else { "status-dot" } }
                        span { class: "entity-row__copy", strong { "{device.name}" } small { "VID {device.vid:04X} · PID {device.pid:04X}" } }
                    }
                }
            }
        }
        section { class: "workspace",
            if let Some(id) = selected().filter(|id| devices.iter().any(|device| device.id == *id)) { DeviceEditor { key: "{id}", id, selected } }
            else { EmptyState { title: "Select a device", copy: "Add a connected or manual HID interface, then reuse it across report actions." } }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct EntityIdProps {
    id: String,
    selected: Signal<Option<String>>,
}

#[component]
fn DeviceEditor(props: EntityIdProps) -> Element {
    let mut selected = props.selected;
    let delete_id = props.id.clone();
    let save_id = props.id.clone();
    let original = CONFIG_SIGNAL
        .read()
        .devices
        .iter()
        .find(|item| item.id == props.id)
        .cloned()
        .unwrap_or_default();
    let mut draft = use_signal(|| original.clone());
    let mut message = use_signal(|| None::<(bool, String)>);
    let mut delete_confirm = use_signal(|| false);
    let snapshot = draft();
    let editor_token = format!("device:{}", props.id);
    let is_new = UNSAVED_ENTITY_SIGNAL.read().as_deref() == Some(editor_token.as_str());
    let dirty = snapshot != original || is_new;
    let cancel_original = original.clone();
    let cancel_id = props.id.clone();
    let cancel_token = editor_token.clone();
    let effect_token = editor_token.clone();
    let effect_original = original.clone();
    use_effect(move || {
        let pending = UNSAVED_ENTITY_SIGNAL.read().as_deref() == Some(effect_token.as_str());
        if draft() != effect_original || pending {
            *DIRTY_EDITOR_SIGNAL.write() = Some(effect_token.clone());
        } else if DIRTY_EDITOR_SIGNAL.read().as_deref() == Some(effect_token.as_str()) {
            *DIRTY_EDITOR_SIGNAL.write() = None;
        }
    });
    let cleanup_token = editor_token.clone();
    use_drop(move || {
        if DIRTY_EDITOR_SIGNAL.read().as_deref() == Some(cleanup_token.as_str()) {
            *DIRTY_EDITOR_SIGNAL.write() = None;
        }
    });
    let references = CONFIG_SIGNAL
        .read()
        .automations
        .iter()
        .filter(|automation| {
            automation
                .cases
                .iter()
                .flat_map(|case| &case.actions)
                .chain(&automation.otherwise_actions)
                .any(|action| action.device_ids.contains(&props.id))
        })
        .map(|automation| automation.name.clone())
        .collect::<Vec<_>>();
    let references_text = references.join(", ");
    let delete_references = references.clone();
    let delete_references_text = references_text.clone();
    rsx! {
        header { class: "workspace-header", div { div { class: "eyebrow", "HID DESTINATION" } h2 { "{snapshot.name}" } p { if hid::is_connected(&snapshot) { "Connected and available" } else { "Saved offline · waiting for matching interface" } } }
            button { class: "button danger-ghost", onclick: move |_| {
                if !delete_references.is_empty() {
                    message.set(Some((false, format!("Used by: {delete_references_text}"))));
                } else if delete_confirm() {
                    let mut next = CONFIG_SIGNAL.read().clone();
                    next.devices.retain(|device| device.id != delete_id);
                    match next.save() {
                        Ok(()) => { *CONFIG_SIGNAL.write() = next; *DIRTY_EDITOR_SIGNAL.write() = None; *UNSAVED_ENTITY_SIGNAL.write() = None; selected.set(None); }
                        Err(error) => message.set(Some((false, format!("Delete failed: {error}")))),
                    }
                } else {
                    delete_confirm.set(true);
                    message.set(Some((false, "Click Delete again to confirm".into())));
                }
            }, "Delete" }
        }
        div { class: "editor-scroll",
            section { class: "editor-card", div { class: "section-heading", span { class: "step", "01" } div { h3 { "Identity" } p { "Stable references used by automations" } } }
                div { class: "form-grid two", label { "Name" input { value: "{snapshot.name}", oninput: move |event| draft.write().name = event.value() } } label { "Stable ID" input { value: "{snapshot.id}", disabled: true } } }
            }
            section { class: "editor-card", div { class: "section-heading", span { class: "step", "02" } div { h3 { "HID interface" } p { "Decimal values; firmware-level configuration remains visible" } } }
                div { class: "form-grid three",
                    NumericField { label: "Vendor ID", value: snapshot.vid, on_change: move |value| draft.write().vid = value }
                    NumericField { label: "Product ID", value: snapshot.pid, on_change: move |value| draft.write().pid = value }
                    NumericField { label: "Usage page", value: snapshot.usage_page, on_change: move |value| draft.write().usage_page = value }
                    NumericField { label: "Usage", value: snapshot.usage, on_change: move |value| draft.write().usage = value }
                    NumericField { label: "Report length", value: snapshot.report_length, on_change: move |value| draft.write().report_length = value }
                    label { "Report ID" input { type: "number", min: "0", max: "255", value: "{snapshot.report_id}", oninput: move |event| if let Ok(value) = event.value().parse() { draft.write().report_id = value } } }
                }
                div { class: "device-note", strong { "Connected-device adoption" } p { "Discovery remains available from the device list refresh; manual values are never hidden or replaced without confirmation." } }
            }
            if !references.is_empty() { section { class: "editor-card references", h3 { "Used by" } p { "{references_text}" } } }
        }
        footer { class: "save-bar",
            if let Some((success, text)) = message() { span { class: if success { "message success" } else { "message error" }, "{text}" } } else { span {} }
            div { class: "toolbar", button { class: "button ghost", disabled: !dirty, onclick: move |_| {
                if UNSAVED_ENTITY_SIGNAL.read().as_deref() == Some(cancel_token.as_str()) {
                    CONFIG_SIGNAL.write().devices.retain(|device| device.id != cancel_id);
                    *UNSAVED_ENTITY_SIGNAL.write() = None;
                    *DIRTY_EDITOR_SIGNAL.write() = None;
                    selected.set(None);
                } else {
                    draft.set(cancel_original.clone());
                }
            }, "Cancel" }
                button { class: "button primary", disabled: !dirty, onclick: move |_| {
                    let mut next = CONFIG_SIGNAL.read().clone();
                    if let Some(index) = next.devices.iter().position(|item| item.id == save_id) { next.devices[index] = draft(); }
                    let errors = next.validate();
                    if errors.is_empty() { match next.save() { Ok(()) => { *CONFIG_SIGNAL.write() = next; *UNSAVED_ENTITY_SIGNAL.write() = None; *DIRTY_EDITOR_SIGNAL.write() = None; message.set(Some((true, "Device saved".into()))); }, Err(error) => message.set(Some((false, error.to_string()))) } }
                    else { message.set(Some((false, errors[0].message.clone()))); }
                }, "Save device" }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct NumericFieldProps {
    label: &'static str,
    value: u16,
    on_change: EventHandler<u16>,
}

#[component]
fn NumericField(props: NumericFieldProps) -> Element {
    rsx! { label { "{props.label}" input { type: "number", min: "0", max: "65535", value: "{props.value}", oninput: move |event| if let Ok(value) = event.value().parse() { props.on_change.call(value) } } } }
}

#[component]
fn SettingsView() -> Element {
    let window = use_window();
    let reload_window = window.clone();
    let original = CONFIG_SIGNAL.read().settings.clone();
    let previous_startup = original.start_with_windows;
    let mut draft = use_signal(|| original.clone());
    let mut message = use_signal(String::new);
    let snapshot = draft();
    let cancel_original = original.clone();
    let effect_original = original.clone();
    use_effect(move || {
        if draft() != effect_original {
            *DIRTY_EDITOR_SIGNAL.write() = Some("settings".into());
        } else if DIRTY_EDITOR_SIGNAL.read().as_deref() == Some("settings") {
            *DIRTY_EDITOR_SIGNAL.write() = None;
        }
    });
    use_drop(move || {
        if DIRTY_EDITOR_SIGNAL.read().as_deref() == Some("settings") {
            *DIRTY_EDITOR_SIGNAL.write() = None;
        }
    });
    rsx! {
        section { class: "workspace settings-workspace",
            header { class: "workspace-header", div { div { class: "eyebrow", "APPLICATION" } h2 { "Settings" } p { "Startup, tray, configuration, and diagnostics" } } }
            div { class: "editor-scroll",
                section { class: "editor-card", div { class: "section-heading", span { class: "step", "01" } div { h3 { "Startup and tray" } p { "Locked In continues running when its window is closed" } } }
                    label { class: "setting-row", div { strong { "Start minimized" } small { "Hide the window after launch" } } input { type: "checkbox", checked: snapshot.start_minimized, onchange: move |event| draft.write().start_minimized = event.checked() } }
                    label { class: "setting-row", div { strong { "Close to tray" } small { "Keep automations active after closing the window" } } input { type: "checkbox", checked: snapshot.close_to_tray, onchange: move |event| draft.write().close_to_tray = event.checked() } }
                    label { class: "setting-row", div { strong { "Start with Windows" } small { "Register for the current user; no administrator access required" } } input { type: "checkbox", checked: snapshot.start_with_windows, onchange: move |event| draft.write().start_with_windows = event.checked() } }
                }
                section { class: "editor-card", div { class: "section-heading", span { class: "step", "02" } div { h3 { "Configuration" } p { "Edit TOML externally, then explicitly reload" } } }
                    div { class: "settings-actions",
                        button { class: "button secondary", onclick: move |_| if let Ok(path) = config::config_path() { let _ = Command::new("notepad.exe").arg(path).spawn(); }, "Open config file" }
                        button { class: "button secondary", onclick: move |_| match config::Config::load() {
                            Ok(config) => match crate::win::set_start_with_windows(config.settings.start_with_windows) {
                                Ok(()) => {
                                    app_log::set_level(config.settings.log_level);
                                    reload_window.set_close_behavior(if config.settings.close_to_tray { dioxus::desktop::WindowCloseBehaviour::WindowHides } else { dioxus::desktop::WindowCloseBehaviour::WindowCloses });
                                    draft.set(config.settings.clone());
                                    *CONFIG_SIGNAL.write() = config;
                                    *CONFIG_REVISION_SIGNAL.write() += 1;
                                    *DIRTY_EDITOR_SIGNAL.write() = None;
                                    message.set("Configuration and runtime settings reloaded".into());
                                }
                                Err(error) => message.set(format!("Reload failed while applying startup setting: {error}")),
                            },
                            Err(error) => message.set(format!("Reload failed: {error}")),
                        }, "Reload from disk" }
                    }
                }
                section { class: "editor-card", div { class: "section-heading", span { class: "step", "03" } div { h3 { "Diagnostics" } p { "Rotating plain-text logs for event and HID debugging" } } }
                    div { class: "form-grid two", label { "Log level" select { value: log_level_name(snapshot.log_level), onchange: move |event| draft.write().log_level = parse_log_level(&event.value()), option { value: "error", "Error" } option { value: "info", "Info" } option { value: "debug", "Debug" } } }
                        div { class: "settings-actions align-end", button { class: "button secondary", onclick: move |_| if let Ok(path) = app_log::log_directory() { let _ = Command::new("explorer.exe").arg(path).spawn(); }, "Open log folder" } }
                    }
                }
            }
            footer { class: "save-bar", span { class: "message success", "{message}" }
                div { class: "toolbar",
                button { class: "button ghost", disabled: snapshot == original, onclick: move |_| { draft.set(cancel_original.clone()); *DIRTY_EDITOR_SIGNAL.write() = None; }, "Cancel" }
                button { class: "button primary", disabled: snapshot == original, onclick: move |_| {
                    let mut next = CONFIG_SIGNAL.read().clone(); next.settings = draft();
                    if let Err(error) = crate::win::set_start_with_windows(next.settings.start_with_windows) {
                        message.set(format!("Startup setting failed: {error}"));
                    } else {
                        match next.save() {
                            Ok(()) => {
                                app_log::set_level(next.settings.log_level);
                                window.set_close_behavior(if next.settings.close_to_tray { dioxus::desktop::WindowCloseBehaviour::WindowHides } else { dioxus::desktop::WindowCloseBehaviour::WindowCloses });
                                *CONFIG_SIGNAL.write() = next;
                                *DIRTY_EDITOR_SIGNAL.write() = None;
                                message.set("Settings saved".into());
                            }
                            Err(error) => {
                                match crate::win::set_start_with_windows(previous_startup) {
                                    Ok(()) => message.set(format!("Save failed: {error}")),
                                    Err(rollback_error) => message.set(format!("Save failed: {error}; startup rollback also failed: {rollback_error}")),
                                }
                            }
                        }
                    }
                }, "Save settings" }
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct EmptyStateProps {
    title: &'static str,
    copy: &'static str,
}

#[component]
fn EmptyState(props: EmptyStateProps) -> Element {
    rsx! { div { class: "empty-state", div { class: "empty-state__glyph", "↗" } h2 { "{props.title}" } p { "{props.copy}" } } }
}

#[component]
fn CaptureDialog() -> Element {
    let captured = CAPTURED_WINDOW_SIGNAL.read().clone().unwrap_or_default();
    let mut automation_id = use_signal(String::new);
    let mut case_id = use_signal(String::new);
    let mut exception = use_signal(|| false);
    let mut message = use_signal(String::new);
    let config = CONFIG_SIGNAL.read().clone();
    let selected_cases = config
        .automations
        .iter()
        .find(|automation| automation.id == automation_id())
        .map(|automation| automation.cases.clone())
        .unwrap_or_default();
    let title = captured.title.clone().unwrap_or_default();
    let class = captured.class.clone().unwrap_or_default();
    let exe = captured
        .exe
        .as_ref()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_default();

    rsx! {
        div { class: "modal-backdrop",
            section { class: "capture-dialog",
                header { div { div { class: "eyebrow", "CAPTURED WINDOW" } h2 { "Assign matcher" } p { "Review the captured metadata, then choose an automation case." } }
                    button { class: "icon-button", aria_label: "Close", onclick: move |_| *CAPTURED_WINDOW_SIGNAL.write() = None, "×" }
                }
                div { class: "capture-metadata",
                    div { span { "Title" } code { "{title}" } }
                    div { span { "Class" } code { "{class}" } }
                    div { span { "Executable" } code { "{exe}" } }
                }
                div { class: "form-grid two",
                    label { "Automation" select { value: "{automation_id}", onchange: move |event| { automation_id.set(event.value()); case_id.set(String::new()); },
                        option { value: "", "Select automation" }
                        for automation in &config.automations { option { value: "{automation.id}", "{automation.name}" } }
                    } }
                    label { "Case" select { value: "{case_id}", disabled: automation_id().is_empty(), onchange: move |event| case_id.set(event.value()),
                        option { value: "", "Select case" }
                        for case in &selected_cases { option { value: "{case.id}", "{case.name}" } }
                    } }
                }
                label { class: "exception-toggle", input { type: "checkbox", checked: exception(), onchange: move |event| exception.set(event.checked()) } "Add as an exception matcher" }
                if !message().is_empty() { p { class: "message error", "{message}" } }
                footer { class: "toolbar modal-actions",
                    button { class: "button ghost", onclick: move |_| *CAPTURED_WINDOW_SIGNAL.write() = None, "Cancel" }
                    button { class: "button primary", disabled: automation_id().is_empty() || case_id().is_empty(), onclick: move |_| {
                        let mut next = CONFIG_SIGNAL.read().clone();
                        let target_automation_id = automation_id();
                        if DIRTY_EDITOR_SIGNAL.read().is_some() {
                            message.set("Save or cancel the open draft, or use Capture next inside that editor".into());
                            return;
                        }
                        let Some(automation) = next.automations.iter_mut().find(|automation| automation.id == target_automation_id) else {
                            message.set("Automation no longer exists".into()); return;
                        };
                        let Some(case) = automation.cases.iter_mut().find(|case| case.id == case_id()) else {
                            message.set("Case no longer exists".into()); return;
                        };
                        let matcher_id = next_child_id("captured", case.applications.iter().chain(&case.exceptions).map(|matcher| matcher.id.as_str()));
                        let list = if exception() { &mut case.exceptions } else { &mut case.applications };
                        list.push(WindowMatcher {
                            id: matcher_id,
                            title: captured.title.clone().map(TextCondition::contains),
                            class: captured.class.clone().map(TextCondition::contains),
                            exe: captured.exe.as_ref().map(|path| TextCondition::equals(path.to_string_lossy())),
                        });
                        match next.save() {
                            Ok(()) => { *CONFIG_SIGNAL.write() = next; *CONFIG_REVISION_SIGNAL.write() += 1; *CAPTURED_WINDOW_SIGNAL.write() = None; }
                            Err(error) => message.set(format!("Could not save matcher: {error}")),
                        }
                    }, "Add matcher" }
                }
            }
        }
    }
}

fn add_case(draft: &mut Signal<Automation>) {
    let snapshot = draft.read();
    let index = snapshot.cases.len() + 1;
    let id = next_child_id("case", snapshot.cases.iter().map(|case| case.id.as_str()));
    drop(snapshot);
    draft.write().cases.push(AutomationCase {
        id,
        name: format!("Case {index}"),
        ..AutomationCase::default()
    });
}

fn add_matcher(draft: &mut Signal<Automation>, case_index: usize, exceptions: bool) {
    let case = &mut draft.write().cases[case_index];
    let id = next_child_id(
        "matcher",
        case.applications
            .iter()
            .chain(&case.exceptions)
            .map(|matcher| matcher.id.as_str()),
    );
    let list = if exceptions {
        &mut case.exceptions
    } else {
        &mut case.applications
    };
    list.push(WindowMatcher {
        id,
        ..WindowMatcher::default()
    });
}

fn add_action(draft: &mut Signal<Automation>, case_index: Option<usize>) {
    let snapshot = draft.read();
    let id = next_child_id(
        "action",
        snapshot
            .cases
            .iter()
            .flat_map(|case| case.actions.iter())
            .chain(snapshot.otherwise_actions.iter())
            .map(|action| action.id.as_str()),
    );
    drop(snapshot);
    let action = SendAction {
        id,
        ..SendAction::default()
    };
    if let Some(index) = case_index {
        draft.write().cases[index].actions.push(action);
    } else {
        draft.write().otherwise_actions.push(action);
    }
}

fn with_action_mut(
    draft: &mut Signal<Automation>,
    case_index: Option<usize>,
    action_index: usize,
    update: impl FnOnce(&mut SendAction),
) {
    let mut automation = draft.write();
    let action = if let Some(index) = case_index {
        &mut automation.cases[index].actions[action_index]
    } else {
        &mut automation.otherwise_actions[action_index]
    };
    update(action);
}

fn remove_action(draft: &mut Signal<Automation>, case_index: Option<usize>, action_index: usize) {
    if let Some(index) = case_index {
        draft.write().cases[index].actions.remove(action_index);
    } else {
        draft.write().otherwise_actions.remove(action_index);
    }
}

fn update_condition(
    draft: &mut Signal<Automation>,
    props: &ConditionRowProps,
    operator: Option<MatchOperator>,
    value: Option<String>,
    case_sensitive: Option<bool>,
) {
    let case = &mut draft.write().cases[props.case_index];
    let matcher = if props.exceptions {
        &mut case.exceptions[props.matcher_index]
    } else {
        &mut case.applications[props.matcher_index]
    };
    let slot = match props.field {
        "title" => &mut matcher.title,
        "class" => &mut matcher.class,
        _ => &mut matcher.exe,
    };
    let mut condition = slot.clone().unwrap_or(TextCondition {
        operator: props.default_operator,
        value: String::new(),
        case_sensitive: false,
    });
    if let Some(operator) = operator {
        condition.operator = operator;
    }
    if let Some(value) = value {
        condition.value = value;
    }
    if let Some(case_sensitive) = case_sensitive {
        condition.case_sensitive = case_sensitive;
    }
    *slot = if condition.value.is_empty() {
        None
    } else {
        Some(condition)
    };
}

fn operator_name(operator: MatchOperator) -> &'static str {
    match operator {
        MatchOperator::Equals => "equals",
        MatchOperator::Contains => "contains",
        MatchOperator::Regex => "regex",
    }
}

fn parse_report_hex(value: &str) -> Result<Vec<u8>, hex::FromHexError> {
    hex::decode(
        value
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>(),
    )
}
fn parse_operator(value: &str) -> MatchOperator {
    match value {
        "equals" => MatchOperator::Equals,
        "regex" => MatchOperator::Regex,
        _ => MatchOperator::Contains,
    }
}
fn log_level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Error => "error",
        LogLevel::Info => "info",
        LogLevel::Debug => "debug",
    }
}
fn parse_log_level(value: &str) -> LogLevel {
    match value {
        "error" => LogLevel::Error,
        "debug" => LogLevel::Debug,
        _ => LogLevel::Info,
    }
}

fn next_child_id<'a>(prefix: &str, existing: impl Iterator<Item = &'a str>) -> String {
    let existing = existing.collect::<HashSet<_>>();
    (1..)
        .map(|index| format!("{prefix}-{index}"))
        .find(|candidate| !existing.contains(candidate.as_str()))
        .expect("identifier search is finite")
}
