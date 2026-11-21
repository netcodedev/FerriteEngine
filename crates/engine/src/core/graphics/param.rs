//! `Param`: a value that is either fixed or chosen by the level of a
//! graphics setting. In RON a fixed value is written plainly (`60.0`,
//! `true`), so fields that used to be plain numbers keep loading; a value
//! per level is `(setting: "vegetation", low: 30.0, medium: 45.0,
//! high: 60.0, ultra: 90.0)`.

use std::{fmt, marker::PhantomData};

use serde::{
    de::{self, MapAccess, Visitor},
    ser::SerializeStruct,
    Deserialize, Deserializer, Serialize, Serializer,
};

use super::{Level, PerLevel, Resolved};

/// A value fixed, or one per level of the `Level` setting `setting`.
#[derive(Clone, Debug, PartialEq)]
pub enum Param<T> {
    Fixed(T),
    ByLevel { setting: String, values: PerLevel<T> },
}

impl<T: Default> Default for Param<T> {
    fn default() -> Self {
        Param::Fixed(T::default())
    }
}

impl<T> From<T> for Param<T> {
    fn from(value: T) -> Self {
        Param::Fixed(value)
    }
}

impl<T: Copy> Param<T> {
    /// The value for the current settings: a bound value takes the level
    /// its setting has (High when the setting is unknown).
    pub fn resolve(&self, settings: &Resolved) -> T {
        match self {
            Param::Fixed(value) => *value,
            Param::ByLevel { setting, values } => *values.get(settings.level(setting)),
        }
    }

    /// The value at `level` (a fixed value at every level).
    pub fn at(&self, level: Level) -> T {
        match self {
            Param::Fixed(value) => *value,
            Param::ByLevel { values, .. } => *values.get(level),
        }
    }

    /// The value at High: what a bound value is where graphics settings
    /// must not reach (see docs/graphics-settings.md, "The rule").
    pub fn high(&self) -> T {
        self.at(Level::High)
    }
}

impl<T> Param<T> {
    pub fn is_bound(&self) -> bool {
        matches!(self, Param::ByLevel { .. })
    }

    /// The setting a bound value follows.
    pub fn setting(&self) -> Option<&str> {
        match self {
            Param::Fixed(_) => None,
            Param::ByLevel { setting, .. } => Some(setting),
        }
    }

    /// Change every value it holds with `f`.
    pub fn map<U>(&self, mut f: impl FnMut(&T) -> U) -> Param<U> {
        match self {
            Param::Fixed(value) => Param::Fixed(f(value)),
            Param::ByLevel { setting, values } => Param::ByLevel {
                setting: setting.clone(),
                values: PerLevel {
                    low: f(&values.low),
                    medium: f(&values.medium),
                    high: f(&values.high),
                    ultra: f(&values.ultra),
                },
            },
        }
    }
}

impl<T: Clone> Param<T> {
    /// Bound to `setting`, every level starting at `value`.
    pub fn bound(setting: &str, value: T) -> Self {
        Param::ByLevel { setting: setting.to_string(), values: PerLevel::all(value) }
    }
}

impl<T: Serialize> Serialize for Param<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Param::Fixed(value) => value.serialize(serializer),
            Param::ByLevel { setting, values } => {
                let mut s = serializer.serialize_struct("Param", 5)?;
                s.serialize_field("setting", setting)?;
                s.serialize_field("low", &values.low)?;
                s.serialize_field("medium", &values.medium)?;
                s.serialize_field("high", &values.high)?;
                s.serialize_field("ultra", &values.ultra)?;
                s.end()
            }
        }
    }
}

/// The plain values a `Param` can hold, so it can tell them from a map
/// while reading.
pub trait ParamScalar: Sized {
    fn from_f64(value: f64) -> Option<Self>;
    fn from_bool(value: bool) -> Option<Self>;
}

impl ParamScalar for f32 {
    fn from_f64(value: f64) -> Option<Self> {
        Some(value as f32)
    }
    fn from_bool(_: bool) -> Option<Self> {
        None
    }
}

impl ParamScalar for bool {
    fn from_f64(_: f64) -> Option<Self> {
        None
    }
    fn from_bool(value: bool) -> Option<Self> {
        Some(value)
    }
}

impl<'de, T> Deserialize<'de> for Param<T>
where
    T: ParamScalar + Deserialize<'de>,
{
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ParamVisitor(PhantomData))
    }
}

struct ParamVisitor<T>(PhantomData<T>);

impl<T> ParamVisitor<T> {
    fn scalar<E: de::Error>(value: Option<T>, what: de::Unexpected) -> Result<Param<T>, E> {
        value.map(Param::Fixed).ok_or_else(|| E::invalid_type(what, &"a value or (setting: ..., low: ..., ...)"))
    }
}

impl<'de, T> Visitor<'de> for ParamVisitor<T>
where
    T: ParamScalar + Deserialize<'de>,
{
    type Value = Param<T>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a value, or (setting: \"...\", low: ..., medium: ..., high: ..., ultra: ...)")
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
        Self::scalar(T::from_f64(v), de::Unexpected::Float(v))
    }

    fn visit_f32<E: de::Error>(self, v: f32) -> Result<Self::Value, E> {
        self.visit_f64(v as f64)
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
        Self::scalar(T::from_f64(v as f64), de::Unexpected::Signed(v))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
        Self::scalar(T::from_f64(v as f64), de::Unexpected::Unsigned(v))
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
        Self::scalar(T::from_bool(v), de::Unexpected::Bool(v))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut setting: Option<String> = None;
        let (mut low, mut medium, mut high, mut ultra) = (None, None, None, None);
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "setting" => setting = Some(map.next_value()?),
                "low" => low = Some(map.next_value()?),
                "medium" => medium = Some(map.next_value()?),
                "high" => high = Some(map.next_value()?),
                "ultra" => ultra = Some(map.next_value()?),
                other => {
                    return Err(de::Error::unknown_field(
                        other,
                        &["setting", "low", "medium", "high", "ultra"],
                    ))
                }
            }
        }
        let missing = |name: &'static str| de::Error::missing_field(name);
        Ok(Param::ByLevel {
            setting: setting.ok_or_else(|| missing("setting"))?,
            values: PerLevel {
                low: low.ok_or_else(|| missing("low"))?,
                medium: medium.ok_or_else(|| missing("medium"))?,
                high: high.ok_or_else(|| missing("high"))?,
                ultra: ultra.ok_or_else(|| missing("ultra"))?,
            },
        })
    }
}
