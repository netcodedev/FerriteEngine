//! A script's parameters as inspector properties, and back. The parameters
//! are a RON map (as serde writes the script's data); numbers, bools and
//! texts become one property each, short lists of numbers (vectors,
//! colours) one per element: `axis x`, `axis y`, `axis z`.

use ron::{
    value::{Map, Number},
    Value,
};

use crate::core::entity::component::{Property, PropertyValue};

use super::label;

const ELEMENTS: [&str; 4] = ["x", "y", "z", "w"];

/// The properties of `data`, ordered as `order` (the struct's fields);
/// values other properties cannot show are left out.
pub fn properties(data: &Value, order: &[&str]) -> Vec<Property> {
    let Value::Map(map) = data else {
        return Vec::new();
    };
    let mut properties = Vec::new();
    for &field in order {
        let Some(value) = get(map, field) else {
            continue;
        };
        match value {
            Value::Seq(items) if !items.is_empty() && items.len() <= ELEMENTS.len() => {
                for (item, element) in items.iter().zip(ELEMENTS) {
                    if let Some(value) = property_value(item) {
                        properties.push(property(format!("{} {element}", label(field)), value));
                    }
                }
            }
            value => {
                if let Some(value) = property_value(value) {
                    properties.push(property(label(field).into_owned(), value));
                }
            }
        }
    }
    properties
}

/// Set the parameter shown as `name` in `data`. Numbers keep their kind (an
/// integer field stays an integer). Returns whether there was one.
pub fn set(data: &mut Value, name: &str, value: PropertyValue) -> bool {
    let Value::Map(map) = data else {
        return false;
    };
    let keys: Vec<String> = map
        .iter()
        .filter_map(|(key, _)| match key {
            Value::String(key) => Some(key.clone()),
            _ => None,
        })
        .collect();
    for key in keys {
        let shown = label(&key);
        let Some(slot) = get_mut(map, &key) else {
            continue;
        };
        if shown == name {
            return replace(slot, value);
        }
        if let Value::Seq(items) = slot {
            for (item, element) in items.iter_mut().zip(ELEMENTS) {
                if format!("{shown} {element}") == name {
                    return replace(item, value);
                }
            }
        }
    }
    false
}

fn property(name: String, value: PropertyValue) -> Property {
    match value {
        PropertyValue::Float(v) => Property::float(name, v),
        PropertyValue::Int(v) => Property::int(name, v),
        PropertyValue::Bool(v) => Property::bool(name, v),
        PropertyValue::Text(v) => Property::text(name, v),
    }
}

fn property_value(value: &Value) -> Option<PropertyValue> {
    Some(match value {
        Value::Bool(v) => PropertyValue::Bool(*v),
        Value::String(v) => PropertyValue::Text(v.clone()),
        Value::Number(number) => match number {
            Number::F32(v) => PropertyValue::Float(v.get()),
            Number::F64(v) => PropertyValue::Float(v.get() as f32),
            number => PropertyValue::Int(integer(number)?),
        },
        _ => return None,
    })
}

fn integer(number: &Number) -> Option<i64> {
    Some(match *number {
        Number::I8(v) => v as i64,
        Number::I16(v) => v as i64,
        Number::I32(v) => v as i64,
        Number::I64(v) => v,
        Number::U8(v) => v as i64,
        Number::U16(v) => v as i64,
        Number::U32(v) => v as i64,
        Number::U64(v) => i64::try_from(v).ok()?,
        _ => return None,
    })
}

/// Put `value` into `slot`, as the kind of value it holds.
fn replace(slot: &mut Value, value: PropertyValue) -> bool {
    let new = match (&*slot, value) {
        (Value::Bool(_), PropertyValue::Bool(v)) => Value::Bool(v),
        (Value::String(_), PropertyValue::Text(v)) => Value::String(v),
        (Value::Number(Number::F64(_)), PropertyValue::Float(v)) => Value::from(v as f64),
        (Value::Number(_), PropertyValue::Float(v)) => Value::from(v),
        (Value::Number(Number::F32(_) | Number::F64(_)), PropertyValue::Int(v)) => {
            Value::from(v as f32)
        }
        // Integers of every width deserialize from an i64 in range.
        (Value::Number(_), PropertyValue::Int(v)) => Value::from(v),
        _ => return false,
    };
    *slot = new;
    true
}

fn get<'a>(map: &'a Map, key: &str) -> Option<&'a Value> {
    map.iter()
        .find(|(k, _)| matches!(k, Value::String(k) if k == key))
        .map(|(_, v)| v)
}

fn get_mut<'a>(map: &'a mut Map, key: &str) -> Option<&'a mut Value> {
    map.iter_mut()
        .find(|(k, _)| matches!(k, Value::String(k) if k == key))
        .map(|(_, v)| v)
}
