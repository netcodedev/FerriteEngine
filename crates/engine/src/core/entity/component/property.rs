use std::borrow::Cow;

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
    /// Usually a literal; owned for per-item names such as `model 3`.
    pub name: Cow<'static, str>,
    pub value: PropertyValue,
    pub read_only: bool,
    /// Inclusive range for float values; tools may show a slider when set.
    pub range: Option<(f32, f32)>,
    /// For text values: the allowed choices; tools show a picker.
    pub options: Option<Vec<String>>,
    /// A bool that is always `false` and that tools show as a button:
    /// setting it `true` does the thing its name says (see `action`).
    pub action: bool,
}

impl Property {
    pub fn float(name: impl Into<Cow<'static, str>>, value: f32) -> Self {
        Self {
            name: name.into(),
            value: PropertyValue::Float(value),
            read_only: false,
            range: None,
            options: None,
            action: false,
        }
    }

    pub fn ranged(name: impl Into<Cow<'static, str>>, value: f32, min: f32, max: f32) -> Self {
        Self {
            range: Some((min, max)),
            ..Self::float(name, value)
        }
    }

    pub fn int(name: impl Into<Cow<'static, str>>, value: i64) -> Self {
        Self {
            name: name.into(),
            value: PropertyValue::Int(value),
            read_only: false,
            range: None,
            options: None,
            action: false,
        }
    }

    pub fn bool(name: impl Into<Cow<'static, str>>, value: bool) -> Self {
        Self {
            name: name.into(),
            value: PropertyValue::Bool(value),
            read_only: false,
            range: None,
            options: None,
            action: false,
        }
    }

    /// Free-form text value.
    pub fn text(name: impl Into<Cow<'static, str>>, value: String) -> Self {
        Self {
            name: name.into(),
            value: PropertyValue::Text(value),
            read_only: false,
            range: None,
            options: None,
            action: false,
        }
    }

    /// Text value picked from `options`.
    pub fn choice(
        name: impl Into<Cow<'static, str>>,
        value: String,
        options: Vec<String>,
    ) -> Self {
        Self {
            name: name.into(),
            value: PropertyValue::Text(value),
            read_only: false,
            range: None,
            options: Some(options),
            action: false,
        }
    }

    /// A button named `name`: clicking it sets the property to `true`
    /// (the component does what it names; it reads `false` again).
    pub fn action(name: impl Into<Cow<'static, str>>) -> Self {
        Self {
            action: true,
            ..Self::bool(name, false)
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

    pub fn as_text(&self) -> Option<&str> {
        match self {
            PropertyValue::Text(v) => Some(v),
            _ => None,
        }
    }
}
