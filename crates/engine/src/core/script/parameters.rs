//! A script's parameters as inspector properties, and back. The parameters
//! are a RON map (as serde writes the script's data); numbers, bools and
//! texts become one property each, short lists of numbers (vectors,
//! colours) one per element: `axis x`, `axis y`, `axis z`. A value bound
//! to a graphics setting (`graphics::Param`, written `(setting: ...,
//! low: ..., ...)`) shows the setting it follows (`range follows`) and
//! one property per level (`range low`, ...).

use ron::{
    value::{Map, Number},
    Value,
};

use crate::core::entity::component::{Property, PropertyValue};

use super::label;

const ELEMENTS: [&str; 4] = ["x", "y", "z", "w"];

/// The fields of a value bound to a graphics setting, levels first.
const LEVELS: [&str; 4] = ["low", "medium", "high", "ultra"];
const SETTING: &str = "setting";

/// Whether `value` is a `graphics::Param` bound to a setting.
fn is_bound(value: &Value) -> bool {
    match value {
        Value::Map(map) => get(map, SETTING).is_some() && LEVELS.iter().all(|level| get(map, level).is_some()),
        _ => false,
    }
}

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
            Value::Map(bound) if is_bound(value) => {
                let shown = label(field);
                let follows = match get(bound, SETTING) {
                    Some(Value::String(setting)) => setting.clone(),
                    _ => String::new(),
                };
                let mut options = vec![crate::core::graphics::FIXED.to_string()];
                options.extend(crate::core::graphics::level_settings());
                if !options.contains(&follows) {
                    options.push(follows.clone());
                }
                properties.push(Property::choice(format!("{shown} follows"), follows, options));
                for level in LEVELS {
                    if let Some(value) = get(bound, level).and_then(property_value) {
                        properties.push(property(format!("{shown} {level}"), value));
                    }
                }
            }
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
        if is_bound(slot) {
            if let Some(done) = set_bound(slot, &shown, name, &value) {
                return done;
            }
            continue;
        }
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

/// Set `name` of the bound value `slot` shown as `shown`; `None` if it is
/// none of its properties.
fn set_bound(slot: &mut Value, shown: &str, name: &str, value: &PropertyValue) -> Option<bool> {
    let rest = name.strip_prefix(shown)?;
    let Value::Map(map) = slot else { return Some(false) };
    if rest == " follows" {
        let Some(setting) = value.as_text().map(str::trim) else { return Some(false) };
        if setting == crate::core::graphics::FIXED {
            // Fixed from now on, at what it was at High.
            let high = get(map, "high").cloned();
            return Some(match high {
                Some(high) => {
                    *slot = high;
                    true
                }
                None => false,
            });
        }
        return Some(match get_mut(map, SETTING) {
            Some(slot) if !setting.is_empty() => {
                *slot = Value::String(setting.to_string());
                true
            }
            _ => false,
        });
    }
    let level = rest.strip_prefix(' ').filter(|level| LEVELS.contains(level))?;
    Some(get_mut(map, level).is_some_and(|slot| replace(slot, value.clone())))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::graphics::{Param, PerLevel};

    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Torch {
        range: Param<f32>,
        plain: f32,
    }

    fn data(torch: &Torch) -> Value {
        ron::from_str(&ron::to_string(torch).unwrap()).unwrap()
    }

    fn back(value: &Value) -> Torch {
        value.clone().into_rust().unwrap()
    }

    #[test]
    fn a_bound_parameter_shows_each_level_and_what_it_follows() {
        let torch = Torch {
            range: Param::ByLevel { setting: "effects".into(), values: PerLevel::new(4.0, 6.0, 8.0, 12.0) },
            plain: 1.0,
        };
        let mut value = data(&torch);
        let names: Vec<String> =
            properties(&value, &["range", "plain"]).into_iter().map(|p| p.name.to_string()).collect();
        assert_eq!(names, ["range follows", "range low", "range medium", "range high", "range ultra", "plain"]);

        assert!(set(&mut value, "range low", PropertyValue::Float(3.0)));
        assert!(set(&mut value, "range follows", PropertyValue::Text("shadows".into())));
        assert!(set(&mut value, "plain", PropertyValue::Float(2.0)));
        assert_eq!(
            back(&value),
            Torch {
                range: Param::ByLevel { setting: "shadows".into(), values: PerLevel::new(3.0, 6.0, 8.0, 12.0) },
                plain: 2.0
            }
        );
        // Fixed again: at its High value, a plain number from now on.
        assert!(set(&mut value, "range follows", PropertyValue::Text("fixed".into())));
        assert_eq!(back(&value).range, Param::Fixed(8.0));
        assert!(set(&mut value, "range", PropertyValue::Float(5.0)));
        assert_eq!(back(&value).range, Param::Fixed(5.0));
    }
}
