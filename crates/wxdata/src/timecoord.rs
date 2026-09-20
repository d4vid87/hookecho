//! Source-independent selection of frames around an analysis time.

use chrono::{DateTime, Duration, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimePolicy {
    Exact,
    Nearest,
    NearestPast,
    InterpolateLinear,
    HoldLast,
    ForecastLead,
}

pub fn policy_for(class: crate::field::DataClass) -> TimePolicy {
    match class {
        crate::field::DataClass::Observed | crate::field::DataClass::Analysis => {
            TimePolicy::NearestPast
        }
        crate::field::DataClass::Forecast => TimePolicy::ForecastLead,
        crate::field::DataClass::Derived => TimePolicy::Exact,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimedFrame<T> {
    pub valid: DateTime<Utc>,
    pub value: T,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Aligned<T> {
    Frame(TimedFrame<T>),
    Interpolate {
        before: TimedFrame<T>,
        after: TimedFrame<T>,
        weight_after: f32,
    },
}

impl<T: Copy> Aligned<T> {
    pub fn displayed_time(self) -> DateTime<Utc> {
        match self {
            Self::Frame(frame) => frame.valid,
            Self::Interpolate { before, .. } => before.valid,
        }
    }
}

/// Select a source frame for `analysis_time`. Input may be unsorted.
///
/// `InterpolateLinear` returns the two bracketing frames and a weight; the caller remains
/// responsible for allowing interpolation only for a compatible continuous descriptor.
pub fn align<T: Copy>(
    frames: &[TimedFrame<T>],
    analysis_time: DateTime<Utc>,
    policy: TimePolicy,
    tolerance: Duration,
) -> Option<Aligned<T>> {
    let within = |time: DateTime<Utc>| (time - analysis_time).abs() <= tolerance;
    match policy {
        TimePolicy::Exact => frames
            .iter()
            .copied()
            .find(|frame| frame.valid == analysis_time)
            .map(Aligned::Frame),
        TimePolicy::Nearest | TimePolicy::ForecastLead => frames
            .iter()
            .copied()
            .filter(|frame| within(frame.valid))
            .min_by_key(|frame| (frame.valid - analysis_time).abs())
            .map(Aligned::Frame),
        TimePolicy::NearestPast | TimePolicy::HoldLast => frames
            .iter()
            .copied()
            .filter(|frame| frame.valid <= analysis_time && within(frame.valid))
            .max_by_key(|frame| frame.valid)
            .map(Aligned::Frame),
        TimePolicy::InterpolateLinear => {
            let before = frames
                .iter()
                .copied()
                .filter(|frame| frame.valid <= analysis_time && within(frame.valid))
                .max_by_key(|frame| frame.valid)?;
            if before.valid == analysis_time {
                return Some(Aligned::Frame(before));
            }
            let after = frames
                .iter()
                .copied()
                .filter(|frame| frame.valid >= analysis_time && within(frame.valid))
                .min_by_key(|frame| frame.valid)?;
            let span = (after.valid - before.valid).num_milliseconds();
            if span <= 0 {
                return Some(Aligned::Frame(before));
            }
            let elapsed = (analysis_time - before.valid).num_milliseconds();
            Some(Aligned::Interpolate {
                before,
                after,
                weight_after: elapsed as f32 / span as f32,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 19, 12, minute, 0)
            .unwrap()
    }

    #[test]
    fn alignment_is_order_independent_and_honors_tolerance() {
        let frames = [
            TimedFrame {
                valid: at(10),
                value: 10,
            },
            TimedFrame {
                valid: at(0),
                value: 0,
            },
            TimedFrame {
                valid: at(20),
                value: 20,
            },
        ];
        assert_eq!(
            align(
                &frames,
                at(14),
                TimePolicy::NearestPast,
                Duration::minutes(5)
            ),
            Some(Aligned::Frame(frames[0]))
        );
        assert_eq!(
            align(
                &frames,
                at(16),
                TimePolicy::NearestPast,
                Duration::minutes(5)
            ),
            None
        );
    }

    #[test]
    fn interpolation_returns_brackets_and_weight() {
        let frames = [
            TimedFrame {
                valid: at(20),
                value: 20,
            },
            TimedFrame {
                valid: at(10),
                value: 10,
            },
        ];
        let Some(Aligned::Interpolate {
            before,
            after,
            weight_after,
        }) = align(
            &frames,
            at(15),
            TimePolicy::InterpolateLinear,
            Duration::minutes(10),
        )
        else {
            panic!("expected interpolation")
        };
        assert_eq!((before.value, after.value), (10, 20));
        assert!((weight_after - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn data_class_selects_a_scientifically_safe_default() {
        use crate::field::DataClass;
        assert_eq!(policy_for(DataClass::Observed), TimePolicy::NearestPast);
        assert_eq!(policy_for(DataClass::Analysis), TimePolicy::NearestPast);
        assert_eq!(policy_for(DataClass::Forecast), TimePolicy::ForecastLead);
        assert_eq!(policy_for(DataClass::Derived), TimePolicy::Exact);
    }
}
