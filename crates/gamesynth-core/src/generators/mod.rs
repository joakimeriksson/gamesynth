//! Library of native (hand-written, fast) sound generators.
//!
//! Every generator is a [`crate::model::Generator`]; [`create`] returns it as a boxed
//! [`Model`] so bindings can treat native and file-defined models identically.

pub mod ambient;
pub mod nature;
pub mod vehicles;

use crate::model::{Generator, Model, ModelDesc, Native};

macro_rules! registry {
    ($($t:ty),* $(,)?) => {
        /// Names of all native generators.
        pub const NAMES: &[&str] = &[$(<$t as Generator>::NAME),*];

        /// Create a native generator by (case-insensitive) name.
        pub fn create(name: &str, sample_rate: f32) -> Option<Box<dyn Model>> {
            $( if name.eq_ignore_ascii_case(<$t as Generator>::NAME) {
                return Some(Box::new(Native::<$t>::new(sample_rate)));
            } )*
            None
        }

        /// Descriptions (inputs, params, presets) of all native generators.
        pub fn describe_all() -> Vec<ModelDesc> {
            vec![$(Native::<$t>::describe()),*]
        }
    };
}

registry!(
    vehicles::Jet,
    vehicles::Hover,
    vehicles::Combustion,
    vehicles::Motor,
    vehicles::Rotor,
    nature::Wind,
    nature::Rain,
    nature::Fire,
    nature::Stream,
    nature::Ocean,
    ambient::Electric,
    ambient::Drone,
    ambient::Crowd,
    ambient::Radio,
    ambient::Siren,
);
