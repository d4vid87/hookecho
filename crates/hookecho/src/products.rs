//! The one place a radar product gets its name, its plain-English blurb, and its hotkey.
//!
//! Before this table the six moments had four different sets of labels — `REF`/`VEL` on the toolbox
//! buttons, "Hi-Res Reflectivity" on mobile, "Correlation Coefficient (CC)" in the command palette,
//! and bare numbers in the hotkey hints. A layman met a different vocabulary in every surface.

use wxdata::level2::Moment;

/// How one radar product is named and explained to the user.
pub struct ProductInfo {
    /// Short code (`REF`, `VEL`, …) — matches [`Moment::short_name`] and the CLI.
    pub short: &'static str,
    /// Full name shown in menus, pickers and the product pill.
    pub name: &'static str,
    /// One line of plain English: what this shows and why you'd look at it.
    pub blurb: &'static str,
    /// Number-row hotkey that selects it.
    pub hotkey: u8,
    pub moment: Moment,
}

/// Every product, in hotkey order.
pub const PRODUCTS: [ProductInfo; 6] = [
    ProductInfo {
        short: "REF",
        name: "Reflectivity",
        blurb: "How heavy the precipitation is — the classic green-to-red radar picture",
        hotkey: 1,
        moment: Moment::Reflectivity,
    },
    ProductInfo {
        short: "VEL",
        name: "Velocity",
        blurb: "Wind blowing toward or away from the radar — how you spot rotation",
        hotkey: 2,
        moment: Moment::Velocity,
    },
    ProductInfo {
        short: "SW",
        name: "Spectrum Width",
        blurb: "How turbulent the wind is inside the storm",
        hotkey: 3,
        moment: Moment::SpectrumWidth,
    },
    ProductInfo {
        short: "ZDR",
        name: "Differential Reflectivity",
        blurb: "Whether the drops are flat or round — big rain drops vs hail",
        hotkey: 4,
        moment: Moment::DifferentialReflectivity,
    },
    ProductInfo {
        short: "PHI",
        name: "Specific Differential Phase",
        blurb: "Rainfall rate in the heaviest cores, unaffected by hail",
        hotkey: 5,
        moment: Moment::DifferentialPhase,
    },
    ProductInfo {
        short: "CC",
        name: "Correlation Coefficient",
        blurb: "How uniform the targets are — tells rain from hail, debris and birds",
        hotkey: 6,
        moment: Moment::CorrelationCoefficient,
    },
];

/// Look a product up by its moment.
pub fn info(moment: Moment) -> &'static ProductInfo {
    PRODUCTS
        .iter()
        .find(|p| p.moment == moment)
        .expect("PRODUCTS covers every Moment — see products_cover_every_moment")
}

/// Display name, with the storm-relative qualifier when velocity is storm-relative.
pub fn name(moment: Moment, srv: bool) -> &'static str {
    if srv && moment == Moment::Velocity {
        "Storm-Relative Velocity"
    } else {
        info(moment).name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn products_cover_every_moment() {
        for m in Moment::ALL {
            let p = info(m);
            assert_eq!(
                p.short,
                m.short_name(),
                "short code must match the CLI code"
            );
            assert!(!p.blurb.is_empty());
        }
    }

    #[test]
    fn hotkeys_are_unique_and_contiguous() {
        let mut keys: Vec<u8> = PRODUCTS.iter().map(|p| p.hotkey).collect();
        keys.sort_unstable();
        assert_eq!(keys, (1..=PRODUCTS.len() as u8).collect::<Vec<_>>());
    }

    /// The blurb advertises a hotkey; the hotkey table had better agree.
    #[test]
    fn hotkeys_match_the_real_bindings() {
        for p in &PRODUCTS {
            let key = match p.hotkey {
                1 => egui::Key::Num1,
                2 => egui::Key::Num2,
                3 => egui::Key::Num3,
                4 => egui::Key::Num4,
                5 => egui::Key::Num5,
                6 => egui::Key::Num6,
                n => panic!("no key for hotkey {n}"),
            };
            let bound = crate::hotkeys::DEFAULTS
                .iter()
                .find(|b| b.key == key)
                .map(|b| b.action);
            assert_eq!(
                bound,
                Some(crate::hotkeys::Action::Product(p.moment)),
                "{}",
                p.short
            );
        }
    }

    #[test]
    fn short_codes_round_trip_through_moment() {
        for p in &PRODUCTS {
            assert_eq!(Moment::from_code(p.short), Some(p.moment));
        }
    }
}
