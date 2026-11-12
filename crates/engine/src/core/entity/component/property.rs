/// A value a component exposes to tools such as the editor inspector.
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyValue {
    Float(f32),
    Int(i64),
    Bool(bool),
    Text(String),
}

/// One editable (or read-only) field of a component.
#[derive(Clone, Debug)]
pub struct Property {
    pub name: &'static str,
    pub value: PropertyValue,
    pub read_only: bool,
    /// Inclusive range for float values; tools may show a slider when set.
    pub range: Option<(f32, f32)>,
}

impl Property {
    pub fn float(name: &'static str, value: f32) -> Self {
        Self {
            name,
            value: PropertyValue::Float(value),
            read_only: false,
            range: None,
        }
    }

    pub fn ranged(name: &'static str, value: f32, min: f32, max: f32) -> Self {
        Self {
            range: Some((min, max)),
            ..Self::float(name, value)
        }
    }

    pub fn int(name: &'static str, value: i64) -> Self {
        Self {
            name,
            value: PropertyValue::Int(value),
            read_only: false,
            range: None,
        }
    }

    pub fn bool(name: &'static str, value: bool) -> Self {
        Self {
            name,
            value: PropertyValue::Bool(value),
            read_only: false,
            range: None,
        }
    }

    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }
}

impl PropertyValue {
    pub fn as_f32(&self) -> Option<f32> {
        match self {
            PropertyValue::Float(v) => Some(*v),
            PropertyValue::Int(v) => Some(*v as f32),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            PropertyValue::Bool(v) => Some(*v),
            _ => None,
        }
    }
}
