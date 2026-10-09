//! Library of native (hand-written, fast) sound generators.
//!
//! Every generator is a [`crate::model::Generator`]; [`create`] returns it as a boxed
//! [`Model`] so bindings can treat native and file-defined models identically.

pub mod ambient;
pub mod foley;
pub mod fx;
pub mod materials;
pub mod nature;
pub mod piston;
pub mod sport;
pub mod ui;
pub mod vehicles;
pub mod wildlife;

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
    piston::Piston,
    vehicles::Motor,
    vehicles::Rotor,
    vehicles::Scrape,
    vehicles::Tyre,
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
    sport::Skate,
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
    fx::OneShot<fx::Bell>,
    fx::OneShot<fx::Finish>,
    fx::OneShot<fx::MetalCrash>,
    fx::OneShot<fx::MudSplash>,
    fx::OneShot<fx::SuspensionThud>,
    fx::OneShot<fx::Debris>,
    fx::OneShot<fx::RockHit>,
    fx::OneShot<fx::PuckStick>,
    fx::OneShot<fx::PuckBoards>,
    fx::OneShot<fx::PuckGlass>,
    fx::OneShot<fx::PuckPost>,
    fx::OneShot<fx::PuckPad>,
    fx::OneShot<fx::GoalHorn>,
    fx::OneShot<fx::Buzzer>,
    fx::OneShot<fx::OrganCharge>,
    fx::OneShot<fx::OrganLetsGo>,
    materials::Strike,
    materials::Roll,
    foley::Footstep,
    ui::Ui<ui::Hover>,
    ui::Ui<ui::Click>,
    ui::Ui<ui::ToggleOn>,
    ui::Ui<ui::ToggleOff>,
    ui::Ui<ui::Confirm>,
    ui::Ui<ui::Cancel>,
    ui::Ui<ui::Error>,
    ui::Ui<ui::Notify>,
    ui::Ui<ui::Open>,
    ui::Ui<ui::Close>,
    ui::Ui<ui::Slider>,
    ui::Ui<ui::Typing>,
    ui::Ui<ui::Coin>,
    ui::Ui<ui::Collect>,
    ui::Ui<ui::Combo>,
    ui::Ui<ui::LevelUp>,
    ui::Ui<ui::Countdown>,
    ui::Ui<ui::Go>,
    ui::Ui<ui::Fanfare>,
    wildlife::Birds,
    wildlife::Insects,
    wildlife::Frogs,
    wildlife::Leaves,
    wildlife::Thunder,
);
