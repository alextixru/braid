//! CFPropertyList ↔ `serde_json::Value`: помощник запоминает прежние
//! значения ключей `Setup:/Network/Service/<id>/DNS` в состоянии на диске
//! и возвращает их точно; платформа читает `State:/Network/Global/*`.
//! Поддерживаются строки, числа, булевы, массивы и словари — всё, что
//! бывает в словарях DNS. Данные и даты в JSON не переводятся (`Null`),
//! а обратно `Null` не принимается, чтобы не записать в систему мусор.

use core_foundation::array::CFArray;
use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::propertylist::{CFPropertyList, CFPropertyListSubClass};
use core_foundation::string::CFString;
use serde_json::{Map, Value};
use std::ffi::c_void;

/// CFPropertyList → JSON. Неподдерживаемые типы → `Null`.
pub fn to_value(plist: &CFPropertyList) -> Value {
    let type_id = plist.type_of();
    if type_id == CFString::type_id() {
        return plist
            .downcast::<CFString>()
            .map(|s| Value::String(s.to_string()))
            .unwrap_or(Value::Null);
    }
    if type_id == CFBoolean::type_id() {
        return plist
            .downcast::<CFBoolean>()
            .map(|b| Value::Bool(bool::from(b)))
            .unwrap_or(Value::Null);
    }
    if type_id == CFNumber::type_id() {
        return plist
            .downcast::<CFNumber>()
            .and_then(|n| {
                // SAFETY: n — живой CFNumber.
                let is_float = unsafe {
                    core_foundation::number::CFNumberIsFloatType(n.as_concrete_TypeRef())
                } != 0;
                if is_float {
                    n.to_f64()
                        .and_then(|f| serde_json::Number::from_f64(f).map(Value::Number))
                } else {
                    n.to_i64().map(|i| Value::Number(i.into()))
                }
            })
            .unwrap_or(Value::Null);
    }
    if type_id == CFArray::<*const c_void>::type_id() {
        let Some(array) = plist.downcast::<CFArray>() else {
            return Value::Null;
        };
        return Value::Array(
            array
                .get_all_values()
                .into_iter()
                // SAFETY: значения массива живы, пока жив `array`; get rule.
                .map(|raw| to_value(&unsafe { CFPropertyList::wrap_under_get_rule(raw) }))
                .collect(),
        );
    }
    if type_id == CFDictionary::<*const c_void, *const c_void>::type_id() {
        let Some(dictionary) = plist.downcast::<CFDictionary>() else {
            return Value::Null;
        };
        let (keys, values) = dictionary.get_keys_and_values();
        let mut object = Map::new();
        for (key, value) in keys.into_iter().zip(values) {
            // SAFETY: ключи словарей SCDynamicStore — CFString; get rule.
            let key = unsafe { CFType::wrap_under_get_rule(key) };
            let Some(key) = key.downcast::<CFString>() else {
                continue;
            };
            // SAFETY: значение живо, пока жив словарь; get rule.
            let value = unsafe { CFPropertyList::wrap_under_get_rule(value) };
            object.insert(key.to_string(), to_value(&value));
        }
        return Value::Object(object);
    }
    Value::Null
}

/// JSON → CFPropertyList. `None`, если встретился `Null` или число вне i64/f64.
pub fn from_value(value: &Value) -> Option<CFPropertyList> {
    Some(match value {
        Value::Null => return None,
        Value::Bool(b) => CFBoolean::from(*b).to_CFPropertyList(),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                CFNumber::from(i).to_CFPropertyList()
            } else {
                CFNumber::from(n.as_f64()?).to_CFPropertyList()
            }
        }
        Value::String(s) => CFString::new(s).to_CFPropertyList(),
        Value::Array(items) => {
            let items: Vec<CFType> = items
                .iter()
                .map(|item| from_value(item).map(CFPropertyList::into_CFType))
                .collect::<Option<_>>()?;
            CFArray::from_CFTypes(&items)
                .to_untyped()
                .to_CFPropertyList()
        }
        Value::Object(object) => {
            let pairs: Vec<(CFString, CFType)> = object
                .iter()
                .map(|(key, item)| {
                    from_value(item).map(|plist| (CFString::new(key), plist.into_CFType()))
                })
                .collect::<Option<_>>()?;
            CFDictionary::from_CFType_pairs(&pairs)
                .to_untyped()
                .to_CFPropertyList()
        }
    })
}
