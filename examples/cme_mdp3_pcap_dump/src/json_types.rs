//! Types the schema's own are mapped onto.

use chrono::{DateTime, SecondsFormat};
use primitive_fixed_point_decimal::ConstScaleFpdec;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zerocopy::byteorder::little_endian::{I64, U64};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned};

/// CME fixes this exponent at -9, which is what a const-scale decimal already is.
pub type Dec9 = ConstScaleFpdec<i64, 9>;

/// A price as it sits on the wire.
///
/// Not a `Deref` to the decimal: this is align 1 so it can be read at any offset in a packet,
/// while the decimal is align 8, and handing out a reference to one would be misaligned. The
/// bytes are the same, the references are not.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned)]
pub struct Price9(pub I64);

impl Price9 {
    pub fn get(self) -> Dec9 {
        Dec9::from_mantissa(self.0.get())
    }
}

impl PartialEq<Dec9> for Price9 {
    fn eq(&self, other: &Dec9) -> bool {
        self.get() == *other
    }
}

impl From<Price9> for Dec9 {
    fn from(price: Price9) -> Self {
        price.get()
    }
}

impl Serialize for Price9 {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.get().serialize(s)
    }
}

impl<'de> Deserialize<'de> for Price9 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Dec9::deserialize(d).map(|dec| Self(I64::new(dec.mantissa())))
    }
}

pub mod utc_timestamp {
    use super::*;
    use serde::{Deserialize, Deserializer};

    pub fn serialize<S: Serializer>(nanos: &U64, s: S) -> Result<S::Ok, S::Error> {
        let at = DateTime::from_timestamp_nanos(nanos.get() as i64);
        s.serialize_str(&at.to_rfc3339_opts(SecondsFormat::Nanos, true))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<U64, D::Error> {
        let text = String::deserialize(d)?;
        let at = DateTime::parse_from_rfc3339(&text).map_err(serde::de::Error::custom)?;
        let nanos = at
            .timestamp_nanos_opt()
            .ok_or_else(|| serde::de::Error::custom("timestamp out of range"))?;
        Ok(U64::new(nanos as u64))
    }
}
