//! Library of native (hand-written, fast) sound generators.
//!
//! Every generator is a [`crate::model::Generator`]; [`create`] returns it as a boxed
//! [`Model`] so bindings can treat native and file-defined models identically.

pub mod ambient;
pub mod fx;
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
    vehicles::Scrape,
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
    ambient::Beam,
    fx::OneShot<fx::Laser>,
    fx::OneShot<fx::Plasma>,
    fx::OneShot<fx::Cannon>,
    fx::OneShot<fx::Rocket>,
    fx::OneShot<fx::MineDrop>,
    fx::OneShot<fx::MineBlast>,
    fx::OneShot<fx::Explosion>,
    fx::OneShot<fx::Impact>,
    fx::OneShot<fx::ShieldHit>,
    fx::OneShot<fx::ShieldUp>,
    fx::OneShot<fx::LockOn>,
    fx::OneShot<fx::Emp>,
    fx::OneShot<fx::Quake>,
    fx::OneShot<fx::Boost>,
    fx::OneShot<fx::Airbrake>,
    fx::OneShot<fx::Pickup>,
    fx::OneShot<fx::Beep>,
);
