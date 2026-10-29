//! CFPropertyList ↔ JSON: так помощник запоминает прежние значения DNS служб
//! в состоянии на диске и возвращает их точно (строки, массивы, числа,
//! булевы, словари — всё, что встречается в `Setup:/Network/Service/*/DNS`).
#![cfg(target_os = "macos")]

use braid_platform::macos::cf_json::{from_value, to_value};
use serde_json::json;

#[test]
fn dns_dictionary_survives_round_trip() {
    let original = json!({
        "ServerAddresses": ["1.1.1.1", "8.8.8.8"],
        "SearchDomains": ["lan"],
        "SupplementalMatchOrders": [100000, 200000],
        "Nested": {"Flag": true, "Ratio": 0.5, "Name": "кириллица"}
    });
    let plist = from_value(&original).expect("словарь переводится в CFPropertyList");
    assert_eq!(to_value(&plist), original);
}

#[test]
fn unsupported_json_is_rejected_instead_of_corrupting_state() {
    assert!(from_value(&json!(null)).is_none());
    assert!(from_value(&json!({"ServerAddresses": [null]})).is_none());
    let plain = from_value(&json!("8.8.8.8")).unwrap();
    assert_eq!(to_value(&plain), json!("8.8.8.8"));
}
