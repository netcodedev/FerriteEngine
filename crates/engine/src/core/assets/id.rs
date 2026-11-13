use std::{fmt, str::FromStr};

use rand::prelude::*;
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

/// Stable identity of an asset. Random, never reused, and independent of
/// the asset's name and location — renaming or moving an asset does not
/// break what refers to it. Written as 16 hex digits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssetId(u64);

impl AssetId {
    pub fn new() -> Self {
        AssetId(rand::rng().random::<u64>())
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl Default for AssetId {
    fn default() -> Self {
        Self::new()
    }
}

impl From<u64> for AssetId {
    fn from(id: u64) -> Self {
        AssetId(id)
    }
}

impl fmt::Display for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl FromStr for AssetId {
    type Err = std::num::ParseIntError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        u64::from_str_radix(text, 16).map(AssetId)
    }
}

impl Serialize for AssetId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for AssetId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse()
            .map_err(|_| de::Error::custom(format!("'{text}' is not an asset id")))
    }
}

/// What a component stores to use an asset: the asset and, for assets that
/// hold several things (a model file with several trees), optionally one of
/// them. Without a `part` it means the asset as a whole.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct AssetRef {
    pub id: AssetId,
    /// Name of a variant of a model asset.
    #[serde(default)]
    pub part: Option<String>,
}

impl AssetRef {
    pub fn whole(id: AssetId) -> Self {
        Self { id, part: None }
    }

    pub fn part(id: AssetId, part: impl Into<String>) -> Self {
        Self {
            id,
            part: Some(part.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_round_trips_as_hex_text() {
        let id = AssetId::from(0x00ab_cdef_0123_4567);
        assert_eq!(id.to_string(), "00abcdef01234567");
        assert_eq!("00abcdef01234567".parse::<AssetId>().unwrap(), id);
        assert!("not hex".parse::<AssetId>().is_err());
    }

    #[test]
    fn reference_serialises_with_optional_part() {
        let id = AssetId::from(0x1f);
        let whole = ron::to_string(&AssetRef::whole(id)).unwrap();
        assert_eq!(ron::from_str::<AssetRef>(&whole).unwrap(), AssetRef::whole(id));
        // Files written without a part still load.
        let parsed: AssetRef = ron::from_str("(id: \"000000000000001f\")").unwrap();
        assert_eq!(parsed, AssetRef::whole(id));
        let part = AssetRef::part(id, "Oak");
        let text = ron::to_string(&part).unwrap();
        assert_eq!(ron::from_str::<AssetRef>(&text).unwrap(), part);
    }
}
