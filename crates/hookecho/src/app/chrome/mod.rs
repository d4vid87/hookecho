//! Desktop chrome: everything drawn around the map. Split out of app.rs so each surface is its
//! own file. `overlay` owns the floating map-first surfaces: the search pill, the right-edge
//! control column and the panels that slide over the map.

mod chips;
mod chaser_dashboard;
mod issues;
pub(super) use issues::RadarIssueLog;
mod clearview;
mod zeta_workspace;
mod zeta_transport;
mod zeta_display;
mod priority;
mod overlay;
mod permalink;
pub(crate) use overlay::compact;
mod registry;
mod radar_identity;
mod scrubber;
mod state;
pub(crate) use state::PrimarySurface;
mod window_frame;
mod windows;
mod workbench;

use super::*;
