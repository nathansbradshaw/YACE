//! Advisory progress from existing engine reports; never changes job identity or results.

#[derive(Clone, Debug, PartialEq)]
pub struct Work {
    scope: String,
    done: f64,
    total: f64,
}

fn valid(scope: &str, done: f64, total: f64) -> Option<Work> {
    let scope = if scope == "Separating instruments" {
        "Separating vocals and instruments"
    } else {
        scope
    };
    (done.is_finite() && total.is_finite() && total > 0. && done >= 0. && done <= total).then(
        || Work {
            scope: scope.into(),
            done,
            total,
        },
    )
}

/// These are the existing engines' activity formats. Unsupported reports stay
/// indeterminate, rather than guessing progress from elapsed time or log length.
pub fn reported_work(label: &str) -> Option<Work> {
    if let Some(counts) = label
        .strip_prefix("Separated ")
        .and_then(|s| s.strip_suffix(" chunks"))
    {
        let (done, total) = counts.split_once(" of ")?;
        return valid(
            "Separating vocals and instruments",
            done.parse().ok()?,
            total.parse().ok()?,
        );
    }
    let (scope, counts) = label.split_once(" · ")?;
    if scope == "Recognizing lyrics locally" {
        return valid(scope, counts.strip_suffix('%')?.parse().ok()?, 100.);
    }
    if let Some(counts) = counts.strip_suffix(" seconds") {
        let (done, total) = counts.split_once('/')?;
        return valid(scope, done.parse().ok()?, total.parse().ok()?);
    }
    let counts = ["chunk ", "window ", "section "]
        .into_iter()
        .find_map(|prefix| counts.strip_prefix(prefix))?;
    let (current, total) = counts.split_once(" of ")?;
    let current = current.parse::<u64>().ok()?;
    let total = total.parse::<u64>().ok()?;
    if current == 0 || current > total {
        return None;
    }
    valid(scope, (current - 1) as f64, total as f64)
}

#[derive(Clone, Debug)]
struct Sample {
    stage: String,
    work: Work,
    first_done: f64,
    first_ms: u64,
    last_ms: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Tracker {
    sample: Option<Sample>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Feedback {
    pub operation: Option<String>,
    pub percent: Option<u32>,
    pub estimate: String,
}
impl Default for Feedback {
    fn default() -> Self {
        Self {
            operation: None,
            percent: None,
            estimate: "Estimating time after the first chunks…".into(),
        }
    }
}

impl Tracker {
    pub fn observe(&mut self, stage: &str, label: &str, counts: Option<(u64, u64)>, now: u64) {
        let parsed = reported_work(label);
        let work = if let Some((done, total)) = counts {
            valid(
                parsed.as_ref().map_or_else(
                    || label.split(" · ").next().unwrap_or(label),
                    |work| work.scope.as_str(),
                ),
                done as f64,
                total as f64,
            )
        } else {
            parsed
        };
        let Some(work) = work else {
            self.sample = None;
            return;
        };
        let reset = self.sample.as_ref().is_none_or(|sample| {
            sample.stage != stage
                || sample.work.scope != work.scope
                || sample.work.total != work.total
                || work.done < sample.work.done
                || now < sample.last_ms
        });
        if reset {
            self.sample = Some(Sample {
                stage: stage.into(),
                first_done: work.done,
                first_ms: now,
                last_ms: now,
                work,
            });
        } else if let Some(sample) = &mut self.sample
            && work.done > sample.work.done
        {
            sample.work = work;
            sample.last_ms = now;
        }
    }
    pub fn feedback(&self, now: u64) -> Feedback {
        let Some(sample) = &self.sample else {
            return Feedback::default();
        };
        let percent = Some(
            (100. * sample.work.done / sample.work.total)
                .floor()
                .clamp(0., 100.) as u32,
        );
        let advanced = sample.work.done - sample.first_done;
        let elapsed = sample.last_ms.saturating_sub(sample.first_ms);
        let estimate = if sample.work.done >= sample.work.total {
            "Finishing this step…".into()
        } else if advanced <= 0. || elapsed < 2000 {
            Feedback::default().estimate
        } else {
            let predicted_ms = (elapsed as f64 / advanced * (sample.work.total - sample.work.done))
                .ceil()
                .min(u64::MAX as f64) as u64;
            let remaining = predicted_ms.saturating_sub(now.saturating_sub(sample.last_ms));
            if remaining == 0 {
                "This step is taking longer than estimated…".into()
            } else if remaining < 60_000 {
                "About a minute or less left in this step".into()
            } else {
                let low = (remaining as f64 * 0.75 / 60_000.).ceil().max(1.) as u64;
                let high = (remaining as f64 * 1.5 / 60_000.)
                    .ceil()
                    .max(low as f64 + 1.) as u64;
                format!("About {low}–{high} min left in this step")
            }
        };
        Feedback {
            operation: Some(sample.work.scope.clone()),
            percent,
            estimate,
        }
    }
}

pub fn part_label(id: &str) -> &str {
    match id {
        "guitar" => "Guitar",
        "bass" => "Bass",
        "drums" => "Drums",
        "vocals" => "Vocals",
        "piano" => "Piano (Keys)",
        "pro_piano" => "Pro Piano (Pro Keys)",
        other => other,
    }
}
pub fn elapsed(ms: u64) -> String {
    let seconds = ms / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reports_started_chunks_as_unfinished_and_done_chunks_as_finished() {
        for (label, done, total) in [
            ("Separating vocals and instruments · chunk 1 of 8", 0., 8.),
            ("Separated 2 of 8 chunks", 2., 8.),
            ("Detecting piano notes · window 3 of 9", 2., 9.),
            ("Comparing vocal pitch · section 2 of 10", 1., 10.),
            ("Detecting vocal contour · chunk 2 of 8", 1., 8.),
            ("Transcribing drums · 20/120 seconds", 20., 120.),
            ("Recognizing lyrics locally · 45%", 45., 100.),
        ] {
            let work = reported_work(label).unwrap();
            assert_eq!((work.done, work.total), (done, total));
        }
        for label in [
            "Loading models…",
            "Downloading pitch · 20 MiB",
            "Confidence · 90%",
            "Detecting · chunk 0 of 8",
            "Detecting · chunk 9 of 8",
            "Detecting · chunk 10 of 8",
            "Detecting · NaN/10 seconds",
            "Detecting · 1/0 seconds",
        ] {
            assert!(reported_work(label).is_none(), "{label}");
        }
    }
    #[test]
    fn estimates_use_observed_work_and_never_run_a_fake_percentage_timer() {
        let mut tracker = Tracker::default();
        tracker.observe(
            "separate",
            "Separating vocals and instruments · chunk 1 of 8",
            None,
            1000,
        );
        assert!(tracker.feedback(10_000).estimate.starts_with("Estimating"));
        tracker.observe("separate", "Separated 2 of 8 chunks", None, 61_000);
        let feedback = tracker.feedback(61_000);
        assert_eq!(feedback.percent, Some(25));
        assert_eq!(feedback.estimate, "About 3–5 min left in this step");
        assert_eq!(tracker.feedback(100_000).percent, Some(25));
        assert!(tracker.feedback(300_000).estimate.contains("longer"));
        tracker.observe("separate", "Separated 8 of 8 chunks", None, 310_000);
        assert_eq!(tracker.feedback(310_000).estimate, "Finishing this step…");
    }
    #[test]
    fn resets_estimates_on_new_stages_counters_and_unmeasured_work() {
        let mut tracker = Tracker::default();
        tracker.observe("notes", "Transcribing drums · 0/120 seconds", None, 0);
        tracker.observe("notes", "Transcribing drums · 20/120 seconds", None, 10_000);
        assert!(!tracker.feedback(10_000).estimate.starts_with("Estimating"));
        tracker.observe("notes", "Transcribing drums · 0/120 seconds", None, 11_000);
        assert!(tracker.feedback(11_000).estimate.starts_with("Estimating"));
        tracker.observe("vocals", "Recognizing lyrics locally · 50%", None, 20_000);
        assert!(tracker.feedback(20_000).estimate.starts_with("Estimating"));
        tracker.observe("vocals", "Saving completed analysis…", None, 21_000);
        assert_eq!(tracker.feedback(21_000).percent, None);
    }
    #[test]
    fn native_counts_override_rounded_activity_text() {
        let mut tracker = Tracker::default();
        tracker.observe(
            "notes",
            "Transcribing drums · 0/120 seconds",
            Some((441, 44100)),
            0,
        );
        assert_eq!(tracker.feedback(0).percent, Some(1));
    }
    #[test]
    fn native_separation_start_and_finish_reports_share_a_rate() {
        let mut tracker = Tracker::default();
        tracker.observe(
            "separation",
            "Separating instruments · chunk 1 of 8",
            Some((0, 8)),
            0,
        );
        tracker.observe(
            "separation",
            "Separated 1 of 8 chunks",
            Some((1, 8)),
            30_000,
        );
        tracker.observe(
            "separation",
            "Separating instruments · chunk 2 of 8",
            Some((1, 8)),
            30_001,
        );
        tracker.observe(
            "separation",
            "Separated 2 of 8 chunks",
            Some((2, 8)),
            60_000,
        );
        assert_eq!(
            tracker.feedback(60_000).estimate,
            "About 3–5 min left in this step"
        );
    }
}
