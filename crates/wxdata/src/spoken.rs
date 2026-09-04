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

/// Two-letter region codes as they arrive in an `areaDesc` tail, and how to say them.
///
/// US states plus the territories NWS issues for, and the Canadian provinces ECCC does. A code
/// that is not here is not a region, and the whole area string is then passed through untouched —
/// MeteoAlarm sends `"Bayern"`, and a marine zone sends prose.
const REGIONS: &[(&str, &str)] = &[
    ("AL", "Alabama"),
    ("AK", "Alaska"),
    ("AZ", "Arizona"),
    ("AR", "Arkansas"),
    ("CA", "California"),
    ("CO", "Colorado"),
    ("CT", "Connecticut"),
    ("DE", "Delaware"),
    ("DC", "the District of Columbia"),
    ("FL", "Florida"),
    ("GA", "Georgia"),
    ("HI", "Hawaii"),
    ("ID", "Idaho"),
    ("IL", "Illinois"),
    ("IN", "Indiana"),
    ("IA", "Iowa"),
    ("KS", "Kansas"),
    ("KY", "Kentucky"),
    ("LA", "Louisiana"),
    ("ME", "Maine"),
    ("MD", "Maryland"),
    ("MA", "Massachusetts"),
    ("MI", "Michigan"),
    ("MN", "Minnesota"),
    ("MS", "Mississippi"),
    ("MO", "Missouri"),
    ("MT", "Montana"),
    ("NE", "Nebraska"),
    ("NV", "Nevada"),
    ("NH", "New Hampshire"),
    ("NJ", "New Jersey"),
    ("NM", "New Mexico"),
    ("NY", "New York"),
    ("NC", "North Carolina"),
    ("ND", "North Dakota"),
    ("OH", "Ohio"),
    ("OK", "Oklahoma"),
    ("OR", "Oregon"),
    ("PA", "Pennsylvania"),
    ("RI", "Rhode Island"),
    ("SC", "South Carolina"),
    ("SD", "South Dakota"),
    ("TN", "Tennessee"),
    ("TX", "Texas"),
    ("UT", "Utah"),
    ("VT", "Vermont"),
    ("VA", "Virginia"),
    ("WA", "Washington"),
    ("WV", "West Virginia"),
    ("WI", "Wisconsin"),
    ("WY", "Wyoming"),
    ("PR", "Puerto Rico"),
    ("VI", "the Virgin Islands"),
    ("GU", "Guam"),
    ("AS", "American Samoa"),
    ("MP", "the Northern Mariana Islands"),
    ("AB", "Alberta"),
    ("BC", "British Columbia"),
    ("MB", "Manitoba"),
    ("NB", "New Brunswick"),
    ("NL", "Newfoundland and Labrador"),
    ("NS", "Nova Scotia"),
    ("NT", "the Northwest Territories"),
    ("NU", "Nunavut"),
    ("ON", "Ontario"),
    ("PE", "Prince Edward Island"),
    ("QC", "Quebec"),
    ("SK", "Saskatchewan"),
    ("YT", "Yukon"),
];

/// Codes whose divisions are not counties. Louisiana has parishes; Alaska has boroughs and census
/// areas, which `areaDesc` already spells out; DC, the territories (Puerto Rico has municipios,
/// the islands have districts) and the Canadian provinces have neither.
const NOT_COUNTY: &[&str] = &[
    "AK", "DC", "PR", "VI", "GU", "AS", "MP", "AB", "BC", "MB", "NB", "NL", "NS", "NT", "NU", "ON",
    "PE", "QC", "SK", "YT",
];

/// At most this many county names before the rest become "and N more". Past four the listener has
/// stopped counting and the sentence is still going.
const MAX_AREAS: usize = 4;

/// At most this many towns. The NWS list runs to twenty on a big polygon.
const MAX_CITIES: usize = 5;

/// Turn an `areaDesc` into something worth hearing: `"Cleveland, OK; McClain, OK"` becomes
/// `"Cleveland County and McClain County, Oklahoma"`.
///
/// The state is said once per run of counties that share it, because saying "Oklahoma" after every
/// name is how a list stops being a list. A name that already carries its own kind (Parish, City,
/// Borough) keeps it and gains nothing.
///
/// Anything that is not a `Name, XX` list is returned as written, only [`expand`]ed: MeteoAlarm
/// sends a German region and NWS sends marine prose, and inventing "County" for either is worse
/// than reading it plainly.
pub fn spoken_area(area: &str) -> String {
    let mut parts: Vec<(String, &'static str)> = Vec::new();
    for raw in area.split(';') {
        let part = raw.trim();
        if part.is_empty() {
            continue;
        }
        // The tail is the last comma group, and only a known two-letter code counts.
        let Some((name, code)) = part.rsplit_once(", ") else {
            return expand(area);
        };
        let Some((_, region)) = REGIONS.iter().find(|(c, _)| *c == code.trim()) else {
            return expand(area);
        };
        let name = name.trim();
        if name.is_empty() {
            return expand(area);
        }
        parts.push((with_kind(name, code.trim()), region));
    }
    if parts.is_empty() {
        return expand(area);
    }
    let more = parts.len().saturating_sub(MAX_AREAS);
    parts.truncate(MAX_AREAS);

    // Group runs that share a region so the region is spoken once, at the end of its run.
    let mut out = String::new();
    let mut i = 0;
    while i < parts.len() {
        let region = parts[i].1;
        let mut names: Vec<&str> = Vec::new();
        while i < parts.len() && parts[i].1 == region {
            names.push(&parts[i].0);
            i += 1;
        }
        if !out.is_empty() {
            out.push_str("; ");
        }
        out.push_str(&join_and(&names));
        out.push_str(", ");
        out.push_str(region);
    }
    if more > 0 {
        out.push_str(&format!(" and {more} more"));
    }
    out
}

/// `"Cleveland"` in Oklahoma is a county; in Louisiana it would be a parish; in Ontario it is just
/// a place. A name that already says what it is keeps its own word.
fn with_kind(name: &str, code: &str) -> String {
    let already = ["County", "Parish", "Borough", "City", "Municipality", "Census Area", "Island"]
        .iter()
        .any(|k| name.ends_with(k));
    if already || NOT_COUNTY.contains(&code) {
        name.to_string()
    } else if code == "LA" {
        format!("{name} Parish")
    } else {
        format!("{name} County")
    }
}

/// `["a", "b", "c"]` -> `"a, b and c"`. No serial comma: this is read aloud, not printed.
fn join_and(items: &[&str]) -> String {
    match items.split_last() {
        None => String::new(),
        Some((last, [])) => (*last).to_string(),
        Some((last, head)) => format!("{} and {last}", head.join(", ")),
    }
}

/// The towns in the path, from the block every NWS warning carries:
///
/// ```text
/// Locations impacted include...
/// Canton, Salem, Columbiana, Boardman, Alliance, Louisville, Sebring,
/// Minerva, Waynesburg and East Sparta.
/// ```
///
/// Matched on a line ending in `include...` — the wording differs between a severe thunderstorm
/// warning and a flash flood one, but the three dots do not. The list is wrapped at whatever
/// column the office's software chose, so it is unwrapped before splitting, and it ends at the
/// blank line.
pub fn cities(description: &str) -> Vec<String> {
    const KEY: &str = "include...";
    let mut lines = description.lines();
    // Whatever follows the key on its own line is the start of the list. Usually nothing — the
    // list begins on the next line — but an office that puts the first town on the same line
    // used to get no towns read at all.
    let Some(first) = lines.find_map(|l| {
        let at = l.to_ascii_lowercase().find(KEY)?;
        Some(l[at + KEY.len()..].trim())
    }) else {
        return Vec::new();
    };
    let mut block: Vec<&str> = vec![first];
    block.extend(lines.take_while(|l| !l.trim().is_empty()).map(|l| l.trim()));
    let joined = block.join(" ");
    let joined = joined.trim().trim_end_matches('.');
    if joined.is_empty() {
        return Vec::new();
    }
    joined
        .split(',')
        // A serial "and" arrives with or without the comma before it, so both are split here.
        .flat_map(|p| p.split(" and "))
        .map(|p| p.trim().trim_start_matches("and ").trim())
        .filter(|p| !p.is_empty())
        .take(MAX_CITIES)
        .map(|p| p.to_string())
        .collect()
}

/// What the office told people to do, short enough to still be playing when it matters.
///
/// The first paragraph of `instruction` is the call to action; anything after it is background
/// ("Continuous cloud to ground lightning is occurring…"). Offices write the urgent ones in
/// capitals, which a synthesizer spells out letter by letter, so a shouted sentence is lowered
/// back to a spoken one.
pub fn call_to_action(instruction: &str) -> String {
    let para: Vec<&str> = instruction
        .lines()
        .map(|l| l.trim())
        .take_while(|l| !l.is_empty())
        .collect();
    if para.is_empty() {
        return String::new();
    }
    let text = para.join(" ");
    let mut out = String::new();
    let mut sentence = String::new();
    let mut taken = 0;
    for c in text.chars() {
        sentence.push(c);
        if !matches!(c, '.' | '!' | '?') {
            continue;
        }
        let s = unshout(sentence.trim());
        if out.len() + s.len() + 1 > 160 && !out.is_empty() {
            return out;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&s);
        sentence.clear();
        taken += 1;
        if taken == 2 {
            return out;
        }
    }
    // A final fragment with no full stop still counts, so long as it fits.
    let s = unshout(sentence.trim());
    if !s.is_empty() && out.len() + s.len() < 160 {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&s);
    }
    out
}

/// `"TAKE COVER NOW!"` → `"Take cover now!"`. A sentence with any lower case in it is left alone,
/// so an ordinary sentence containing an abbreviation is untouched.
fn unshout(s: &str) -> String {
    if s.chars().any(|c| c.is_ascii_lowercase()) {
        return s.to_string();
    }
    capitalize(&s.to_lowercase())
}

/// Where the warning sits relative to a place the listener actually knows.
///
/// `bearing_deg` is the direction from the place to the warning, which is the one a listener can
/// act on. Zero distance is not "0 miles from Home" — it is the polygon sitting on top of it.
pub fn relation(name: &str, km: f64, bearing_deg: Option<f32>, metric: bool) -> String {
    if name.is_empty() {
        return String::new();
    }
    if km <= 0.05 {
        return format!("covering {name}");
    }
    match bearing_deg {
        Some(b) => format!(
            "{} {} of {name}",
            spoken_distance(km, metric),
            spoken_compass(b)
        ),
        None => format!("{} from {name}", spoken_distance(km, metric)),
    }
}

/// Spelled out, not abbreviated: a synthesizer reading "km" says "kay em".
fn spoken_distance(km: f64, metric: bool) -> String {
    let (n, unit) = if metric {
        (km.round() as i32, "kilometer")
    } else {
        ((km * 0.621_371).round() as i32, "mile")
    };
    if n == 1 {
        format!("1 {unit}")
    } else {
        format!("{n} {unit}s")
    }
}

/// A warning that never happened, for hearing what a real one will sound like.
///
/// Settings speaks this so the whole chain — tone, engine, voice, script — can be checked without
/// waiting for weather, and the tests read the same fixture so the button and the assertions can
/// never drift apart.
pub fn demo_alert() -> AlertInfo {
    AlertInfo {
        id: "urn:hookecho:demo".into(),
        event: "Tornado Warning".into(),
        headline: String::new(),
        area: "Cleveland, OK; McClain, OK".into(),
        description: "At 7 02 PM, a confirmed tornado was located near Norman.\n\n\
                      Locations impacted include...\n\
                      Norman, Moore, and Noble."
            .into(),
        instruction: "TAKE COVER NOW! Move to a basement or an interior room on the lowest floor \
                      of a sturdy building.\n\nAvoid windows."
            .into(),
        expires: None,
        max_hail_in: Some(1.75),
        max_wind: None,
        tornado_detection: Some("OBSERVED".into()),
        damage_threat: Some("CATASTROPHIC".into()),
        source: None,
        motion: Some(crate::overlay::StormMotion {
            deg: 225.0,
            kt: 40.0,
            points: vec![],
        }),
        vtec: None,
    }
}

/// The sentence spoken when a warning arrives.
///
/// Order is what a listener can act on, soonest first: what it is, where it is relative to them,
/// which counties it covers, the towns in its path, how it is moving, and what to do about it.
/// The tone that precedes this says a warning exists; every word here says something the tone
/// could not.
///
/// `relation` is how the warning sits against a watched place ("covering Home", "12 miles
/// northeast of Home"), or empty when it hit none. It used to be a banner fragment, which is why
/// the voice once said "Tornado warning for covers Home".
pub fn warning_script(a: &AlertInfo, relation: &str, until: &str) -> String {
    let mut s = String::new();
    // Lead with the escalated wording when there is any -- "Tornado emergency" first, not fourth.
    let lead = a
        .damage_threat
        .as_deref()
        .filter(|t| t.eq_ignore_ascii_case("DESTRUCTIVE") || t.eq_ignore_ascii_case("CATASTROPHIC"))
        .map(|t| format!("{} {}", expand(t).to_lowercase(), a.event.to_lowercase()))
        .unwrap_or_else(|| a.event.to_lowercase());
    s.push_str(&capitalize(&lead));
    if !relation.is_empty() {
        // Not expanded: every word here is ours except the marker's name, and a marker called
        // "NW Farm" or "HQ" is a name, not shorthand. `expand` turned "IN" into "inch".
        s.push(' ');
        s.push_str(relation);
    }
    let area = spoken_area(&a.area);
    if !area.is_empty() {
        // The comma only earns its place once something already follows the hazard.
        if !relation.is_empty() {
            s.push(',');
        }
        s.push_str(" for ");
        s.push_str(&area);
    }
    let cities = cities(&a.description);
    if !cities.is_empty() {
        let refs: Vec<&str> = cities.iter().map(String::as_str).collect();
        s.push_str(", including ");
        s.push_str(&join_and(&refs));
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
    let cta = call_to_action(&a.instruction);
    if !cta.is_empty() {
        s.push(' ');
        s.push_str(&expand(&cta));
    }
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
    let mut s = format!(
        "{name} is {} to your {}",
        spoken_distance(km, metric),
        spoken_compass(bearing_deg)
    );
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

    #[test]
    fn the_hazard_place_and_heading_come_first() {
        let s = warning_script(&demo_alert(), "covering Home", "7 15 PM");
        assert_eq!(
            s,
            "Catastrophic tornado warning covering Home, for Cleveland County and McClain County, \
             Oklahoma, including Norman, Moore and Noble, moving northeast at 46 miles per hour, \
             hail to 1.75 inch, until 7 15 PM. Take cover now! Move to a basement or an interior \
             room on the lowest floor of a sturdy building."
        );
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
    fn a_marker_name_is_a_name_not_shorthand() {
        let mut a = demo_alert();
        a.description = String::new();
        a.instruction = String::new();
        a.motion = None;
        a.max_hail_in = None;
        a.damage_threat = None;
        let s = warning_script(&a, "covering NW Farm", "");
        assert!(s.starts_with("Tornado warning covering NW Farm, for "), "{s}");
    }

    #[test]
    fn a_plain_warning_still_reads_cleanly() {
        let mut a = demo_alert();
        a.area = "Cleveland County".into();
        a.damage_threat = None;
        a.motion = None;
        a.description = String::new();
        a.instruction = String::new();
        assert_eq!(
            warning_script(&a, "", ""),
            "Tornado warning for Cleveland County, hail to 1.75 inch."
        );
    }

    #[test]
    fn counties_gain_their_kind_and_the_state_is_said_once() {
        assert_eq!(
            spoken_area("Cleveland, OK; McClain, OK"),
            "Cleveland County and McClain County, Oklahoma"
        );
        // Louisiana has parishes, and a name that already says what it is keeps its own word.
        assert_eq!(spoken_area("Orleans, LA"), "Orleans Parish, Louisiana");
        assert_eq!(
            spoken_area("St. Louis City, MO"),
            "St. Louis City, Missouri"
        );
        // Two states: each run names its own, rather than one trailing state for both.
        assert_eq!(
            spoken_area("Cowley, KS; Kay, OK"),
            "Cowley County, Kansas; Kay County, Oklahoma"
        );
        // Canada has no counties to invent.
        assert_eq!(
            spoken_area("Ottawa North - Kanata - Orléans, ON"),
            "Ottawa North - Kanata - Orléans, Ontario"
        );
        // Puerto Rico has municipios. "San Juan County" is a place in Utah.
        assert_eq!(spoken_area("San Juan, PR"), "San Juan, Puerto Rico");
        // Not a `Name, XX` list at all: read it as written rather than guess.
        assert_eq!(spoken_area("Bayern"), "Bayern");
        assert_eq!(
            spoken_area("Coastal waters from Fenwick Island DE to Chincoteague VA"),
            "Coastal waters from Fenwick Island DE to Chincoteague VA"
        );
    }

    #[test]
    fn a_long_county_list_stops_being_a_list() {
        assert_eq!(
            spoken_area("A, OK; B, OK; C, OK; D, OK; E, OK; F, OK"),
            "A County, B County, C County and D County, Oklahoma and 2 more"
        );
    }

    #[test]
    fn the_towns_are_read_off_the_wrapped_block() {
        // Verbatim shape of a real product: the list is wrapped at the office's column and ends
        // at the blank line, and the serial "and" arrives with a comma in front of it.
        let d = "IMPACT...Expect damage to trees and power lines.\n\n\
                 Locations impacted include...\n\
                 Canton, Salem, Columbiana, Boardman, Alliance, Louisville, Sebring,\n\
                 Minerva, Waynesburg, and Maximo.\n\n\
                 HAIL...1.00 IN";
        assert_eq!(
            cities(d),
            ["Canton", "Salem", "Columbiana", "Boardman", "Alliance"]
        );
        assert!(cities("No such block here.").is_empty());
        // The first town on the same line as the key, which used to read as no towns at all.
        assert_eq!(
            cities("Locations impacted include... Norman, Moore\nand Noble.\n\nHAIL...1 IN"),
            ["Norman", "Moore", "Noble"]
        );
        assert!(cities("Locations impacted include...\n\nNothing listed.").is_empty());
    }

    #[test]
    fn the_call_to_action_is_two_sentences_and_never_shouted() {
        let i = "TAKE COVER NOW! Move to a basement or an interior room on the lowest floor of a \
                 sturdy building.\n\nContinuous cloud to ground lightning is occurring.";
        let s = call_to_action(i);
        assert!(s.starts_with("Take cover now!"), "{s}");
        // The second paragraph is background, not an instruction.
        assert!(!s.contains("lightning"), "{s}");
        assert!(s.len() <= 160, "{} chars: {s}", s.len());
        assert_eq!(call_to_action(""), "");
    }

    #[test]
    fn a_relation_says_which_side_of_you_it_is_on() {
        assert_eq!(relation("Home", 0.0, Some(45.0), false), "covering Home");
        assert_eq!(
            relation("Home", 19.3, Some(45.0), false),
            "12 miles northeast of Home"
        );
        assert_eq!(
            relation("Home", 19.3, None, true),
            "19 kilometers from Home"
        );
        // One is one, not "1 miles".
        assert_eq!(relation("Home", 1.61, None, false), "1 mile from Home");
        assert_eq!(relation("", 5.0, None, false), "");
    }

    #[test]
    fn a_position_update_names_the_side_it_is_on() {
        // 225 here is a heading, not a from-bearing: cell tables give direction of travel.
        let s = position_script("The storm", 90.0, 32.2, Some(225.0), false);
        assert_eq!(s, "The storm is 20 miles to your east, moving southwest.");
        let s = position_script("The storm", 90.0, 32.2, Some(225.0), true);
        assert_eq!(
            s,
            "The storm is 32 kilometers to your east, moving southwest."
        );
    }
}
