use super::*;

fn device(id: &str, selector: InterfaceSelector) -> Device {
    Device {
        id: id.into(),
        vid: selector.vendor_id,
        pid: selector.product_id,
        usage_page: selector.usage_page,
        usage: selector.usage,
        ..Device::default()
    }
}

#[test]
fn selector_aliases_distinguish_add_open_and_duplicate_configurations() {
    let selector = InterfaceSelector {
        vendor_id: 1,
        product_id: 2,
        usage_page: 3,
        usage: 4,
    };

    assert_eq!(selector_aliases(&[], selector), SelectorAliases::None);
    assert_eq!(
        selector_aliases(&[device("first", selector)], selector),
        SelectorAliases::One("first".into())
    );
    assert_eq!(
        selector_aliases(
            &[device("first", selector), device("second", selector)],
            selector
        ),
        SelectorAliases::Multiple(2)
    );
}

#[test]
fn selector_summary_formats_every_identifier_as_four_digit_hexadecimal() {
    let selector = InterfaceSelector {
        vendor_id: 0x4359,
        product_id: 0,
        usage_page: 0xff60,
        usage: 0x61,
    };

    assert_eq!(format_selector(selector), "4359:0000 · FF60:0061");
}
