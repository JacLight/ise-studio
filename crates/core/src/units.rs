//! Dimensional quantities. Values are stored in one canonical SI unit and
//! accepted at command boundaries as `{"value": 2.5, "unit": "mm"}`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

macro_rules! quantity {
    (
        $(#[$doc:meta])*
        $name:ident, $input:ident, $unit:ident, canonical = $canonical:ident,
        { $($variant:ident = $tag:literal => $factor:expr),+ $(,)? }
    ) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
        pub enum $unit {
            $(#[serde(rename = $tag)] $variant,)+
        }

        impl $unit {
            /// Canonical units per one of this unit.
            pub const fn factor(self) -> f64 {
                match self { $(Self::$variant => $factor,)+ }
            }
        }

        /// Wire form of the quantity.
        #[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
        pub struct $input {
            pub value: f64,
            pub unit: $unit,
        }

        $(#[$doc])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
        pub struct $name(f64);

        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                $input::from(*self).serialize(s)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                Self::try_from($input::deserialize(d)?).map_err(serde::de::Error::custom)
            }
        }

        impl $name {
            pub fn new(value: f64, unit: $unit) -> Self {
                Self(value * unit.factor())
            }

            /// Value in the canonical unit.
            pub const fn canonical(self) -> f64 {
                self.0
            }

            pub fn to(self, unit: $unit) -> f64 {
                self.0 / unit.factor()
            }

            pub fn is_finite(self) -> bool {
                self.0.is_finite()
            }
        }

        impl TryFrom<$input> for $name {
            type Error = String;

            fn try_from(q: $input) -> Result<Self, String> {
                if q.value.is_finite() {
                    Ok(Self::new(q.value, q.unit))
                } else {
                    Err(format!("{} must be a finite number", stringify!($name)))
                }
            }
        }

        impl From<$name> for $input {
            fn from(v: $name) -> Self {
                Self { value: v.0, unit: $unit::$canonical }
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
                <$input as JsonSchema>::json_schema(generator)
            }
        }
    };
}

quantity! {
    /// Length, stored in meters.
    Length, LengthQuantity, LengthUnit, canonical = M,
    { Mm = "mm" => 1e-3, Cm = "cm" => 1e-2, M = "m" => 1.0, In = "in" => 0.0254, Ft = "ft" => 0.3048 }
}

quantity! {
    /// Mass density, stored in kg/m³.
    Density, DensityQuantity, DensityUnit, canonical = KgPerM3,
    { KgPerM3 = "kg/m3" => 1.0, GPerCm3 = "g/cm3" => 1000.0 }
}

quantity! {
    /// Mass, stored in kilograms.
    Mass, MassQuantity, MassUnit, canonical = Kg,
    { G = "g" => 1e-3, Kg = "kg" => 1.0, Lb = "lb" => 0.453_592_37 }
}

impl Length {
    pub fn mm(value: f64) -> Self {
        Self::new(value, LengthUnit::Mm)
    }

    pub fn m(value: f64) -> Self {
        Self::new(value, LengthUnit::M)
    }
}

impl Density {
    pub fn kg_per_m3(value: f64) -> Self {
        Self::new(value, DensityUnit::KgPerM3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_between_units() {
        assert!((Length::new(25.4, LengthUnit::Mm).to(LengthUnit::In) - 1.0).abs() < 1e-12);
        assert_eq!(Density::new(7.85, DensityUnit::GPerCm3).canonical(), 7850.0);
    }

    #[test]
    fn parses_units_and_rejects_non_finite() {
        let l: Length = serde_json::from_str(r#"{"value": 30, "unit": "mm"}"#).unwrap();
        assert!((l.canonical() - 0.03).abs() < 1e-15);
        assert_eq!(serde_json::to_value(l).unwrap()["unit"], "m");
        assert!(serde_json::from_str::<Length>(r#"{"value": 1, "unit": "furlong"}"#).is_err());
        assert!(Length::try_from(LengthQuantity { value: f64::NAN, unit: LengthUnit::M }).is_err());
    }
}
