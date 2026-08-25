//! Desktop chrome: everything drawn around the map. Split out of app.rs so each surface is its
//! own file. `overlay` owns the floating map-first surfaces: the search pill, the right-edge
//! control column and the panels that slide over the map.

mod chips;
mod overlay;
mod registry;
mod scrubber;
mod windows;

use super::*;
