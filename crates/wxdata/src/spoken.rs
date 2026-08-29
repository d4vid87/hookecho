//! Turning an alert into something worth hearing.
//!
//! The old spoken line was `"{event} for {area} until {time}"`, which starts with the four words
//! the listener already guessed and buries the two that matter: what it is going to do, and
//! whether it is coming at you. Driving, you get one sentence before your attention goes back to
//! the road, so the hazard, the place and the direction go in that sentence.
//!
//! It also read raw NWS text, which is written to be *seen*: "60 MPH", "SW of", "1.75 IN".
//! A synthesizer says "sixty em pee aitch". [`expand`] fixes that.
//!
//! Pure string work, no engine and no platform: desktop, Android and the browser all speak
//! whatever this returns.

use crate::overlay::AlertInfo;

/// 16-point compass bearing, spoken in full.
fn spoken_compass(deg: f32) -> &'static str {
    const D: [&str; 16] = [
        "north",
        "north northeast",
        "northeast",
        "east northeast",
        "east",
        "east southeast",
        "southeast",
        "south southeast",
        "south",
        "south southwest",
        "southwest",
        "west southwest",
        "west",
        "west northwest",
        "northwest",
        "north northwest",
    ];
    D[((deg / 22.5).round() as usize) % 16]
}

/// Abbreviations as issued, and how to say them. Matched on whole words, longest first, so `MPH`
/// wins before `PH` could and `NNE` before `N`.
const ABBREV: &[(&str, &str)] = &[
    ("MPH", "miles per hour"),
    ("KTS", "knots"),
    ("KT", "knots"),
    ("TSTM", "thunderstorm"),
    ("TSTMS", "thunderstorms"),
    ("PDS", "particularly dangerous situation"),
    ("NWS", "National Weather Service"),
    ("EMER", "emergency"),
    ("SVR", "severe"),
    ("TOR", "tornado"),
    ("FFW", "flash flood warning"),
    ("CO", "county"),
    ("CNTY", "county"),
    ("MI", "miles"),
    ("NM", "nautical miles"),
    ("IN", "inch"),
    ("FT", "feet"),
    ("AM", "A M"),
    ("PM", "P M"),
    ("CDT", "central time"),
    ("CST", "central time"),
    ("EDT", "eastern time"),
    ("EST", "eastern time"),
    ("MDT", "mountain time"),
    ("MST", "mountain time"),
    ("PDT", "pacific time"),
    ("PST", "pacific time"),
    ("N", "north"),
    ("S", "south"),
    ("E", "east"),
    ("W", "west"),
    ("NE", "northeast"),
    ("NW", "northwest"),
    ("SE", "southeast"),
    ("SW", "southwest"),
    ("NNE", "north northeast"),
    ("ENE", "east northeast"),
    ("ESE", "east southeast"),
    ("SSE", "south southeast"),
    ("SSW", "south southwest"),
    ("WSW", "west southwest"),
    ("WNW", "west northwest"),
    ("NNW", "north northwest"),
];

/// Expand NWS shorthand into words a synthesizer can say.
///
/// Only whole all-caps words are touched: a place called Independence keeps its `IN`, and lower
/// case prose is left exactly as written. Anything not in the table is passed through, because a
/// wrong expansion is worse than an unexpanded one — "N" heard as "north" in the wrong place is a
/// direction the listener acts on.
pub fn expand(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if !word.is_empty() {
            let hit = word
                .chars()
                .all(|c| c.is_ascii_uppercase())
                .then(|| ABBREV.iter().find(|(a, _)| *a == word).map(|(_, b)| *b))
                .flatten();
            match hit {
                Some(rep) => out.push_str(rep),
                None => out.push_str(word),
            }
            word.clear();
        }
    };
    for c in text.chars() {
        if c.is_ascii_alphabetic() {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// The sentence spoken when a warning arrives: hazard, place, motion, then the expiry.
///
/// `place` is the watched location the warning hit, or the alert's own area when it hit none.
/// `until` is the already-formatted clock time, or empty.
pub fn warning_script(a: &AlertInfo, place: &str, until: &str) -> String {
    let mut s = String::new();
    // Lead with the escalated wording when there is any — "Tornado emergency" first, not fourth.
    let lead = a
        .damage_threat
        .as_deref()
        .filter(|t| t.eq_ignore_ascii_case("DESTRUCTIVE") || t.eq_ignore_ascii_case("CATASTROPHIC"))
        .map(|t| format!("{} {}", expand(t).to_lowercase(), a.event.to_lowercase()))
        .unwrap_or_else(|| a.event.to_lowercase());
    s.push_str(&capitalize(&lead));
    if !place.is_empty() {
        s.push_str(" for ");
        s.push_str(&expand(place));
    }
    if let Some(m) = &a.motion {
        // `deg` is the FROM bearing as issued; the heading it travels toward is the other way,
        // and the heading is the only half a listener can act on.
        s.push_str(&format!(
            ", moving {} at {} miles per hour",
            spoken_compass((m.deg + 180.0) % 360.0),
            (m.kt * 1.15078).round() as i32
        ));
    }
    if let Some(h) = a.max_hail_in {
        s.push_str(&format!(", hail to {h} inch"));
    }
    if let Some(w) = &a.max_wind {
        s.push_str(&format!(", winds {}", expand(w)));
    }
    if !until.is_empty() {
        s.push_str(", until ");
        s.push_str(until);
    }
    s.push('.');
    s
}

/// The optional chase update: where the nearest storm is relative to you, spoken.
///
/// `bearing_deg` is the direction from the listener to the storm, `km` the distance, and
/// `moving_toward_deg` the heading the storm travels *toward* — which is what storm-cell tables
/// carry, unlike an alert's motion field. Nothing is flipped here.
pub fn position_script(
    name: &str,
    bearing_deg: f32,
    km: f64,
    moving_toward_deg: Option<f32>,
    metric: bool,
) -> String {
    // Spelled out, not abbreviated: a synthesizer reading "km" says "kay em".
    let (n, unit) = if metric {
        (km.round() as i32, "kilometers")
    } else {
        ((km * 0.621_371).round() as i32, "miles")
    };
    let mut s = format!("{name} is {n} {unit} to your {}", spoken_compass(bearing_deg));
    if let Some(d) = moving_toward_deg {
        s.push_str(&format!(", moving {}", spoken_compass(d)));
    }
    s.push('.');
    s
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::StormMotion;

    fn tor() -> AlertInfo {
        AlertInfo {
            id: "urn:x".into(),
            event: "Tornado Warning".into(),
            headline: String::new(),
            area: "Cleveland County".into(),
            description: String::new(),
            instruction: String::new(),
            expires: None,
            max_hail_in: None,
            max_wind: None,
            tornado_detection: Some("OBSERVED".into()),
            damage_threat: Some("CATASTROPHIC".into()),
            source: None,
            motion: Some(StormMotion {
                deg: 225.0, // from the southwest, so travelling northeast
                kt: 40.0,
                points: vec![],
            }),
            vtec: None,
        }
    }

    #[test]
    fn the_hazard_place_and_heading_come_first() {
        let s = warning_script(&tor(), "Home", "7 15 PM");
        assert!(
            s.starts_with("Catastrophic tornado warning for Home, moving northeast at 46 miles"),
            "{s}"
        );
        assert!(s.ends_with("until 7 15 PM."), "{s}");
    }

    #[test]
    fn abbreviations_expand_only_as_whole_capital_words() {
        assert_eq!(expand("60 MPH winds"), "60 miles per hour winds");
        assert_eq!(expand("2 MI SW of Norman"), "2 miles southwest of Norman");
        // Lower case prose and mixed-case place names are left alone — a wrong expansion is
        // worse than none, since a listener acts on a direction.
        assert_eq!(expand("Independence, Indiana"), "Independence, Indiana");
        assert_eq!(expand("in the county"), "in the county");
    }

    #[test]
    fn a_plain_warning_still_reads_cleanly() {
        let mut a = tor();
        a.damage_threat = None;
        a.motion = None;
        a.max_hail_in = Some(1.75);
        assert_eq!(
            warning_script(&a, "", ""),
            "Tornado warning, hail to 1.75 inch."
        );
    }

    #[test]
    fn a_position_update_names_the_side_it_is_on() {
        // 225 here is a heading, not a from-bearing: cell tables give direction of travel.
        let s = position_script("The storm", 90.0, 32.2, Some(225.0), false);
        assert_eq!(s, "The storm is 20 miles to your east, moving southwest.");
        let s = position_script("The storm", 90.0, 32.2, Some(225.0), true);
        assert_eq!(s, "The storm is 32 kilometers to your east, moving southwest.");
    }
}
