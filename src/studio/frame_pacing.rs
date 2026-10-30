//! Presented-frame and note-position update sampling for the native WebView.
//! requestAnimationFrame is a browser API; this minimal JavaScript bridge drives
//! presentation ticks and measures delivery. Audio clocks and chart logic are Rust.
use super::state::Studio;
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
#[derive(Deserialize)]
struct Batch {
    frames: Vec<f64>,
    updates: Vec<f64>,
    audio: String,
    revision: u64,
    update_age_ms: f64,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum FrameEvent {
    Tick,
    Fault {
        message: String,
    },
    Batch {
        #[serde(flatten)]
        batch: Batch,
    },
}
#[derive(Serialize)]
struct Summary {
    version: u32,
    at_ms: u64,
    audio_sha256: String,
    revision: u64,
    samples: usize,
    chart_updates: usize,
    last_chart_update_age_ms: f64,
    display_interval_p50_ms: f64,
    display_interval_p95_ms: f64,
    display_interval_max_ms: f64,
    chart_update_p50_ms: f64,
    chart_update_p95_ms: f64,
    chart_update_max_ms: f64,
    display_gaps_over_50ms: usize,
    chart_update_gaps_over_50ms: usize,
    measurement: &'static str,
}
fn percentile(values: &[f64], fraction: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut ordered = values.to_vec();
    ordered.sort_by(f64::total_cmp);
    ordered[((ordered.len() - 1) as f64 * fraction).round() as usize]
}
fn summarize(batch: Batch) -> Option<Summary> {
    if batch.frames.len() < 30
        || !batch.update_age_ms.is_finite()
        || batch.update_age_ms < 0.0
        || batch
            .frames
            .iter()
            .chain(&batch.updates)
            .any(|v| !v.is_finite() || *v < 0.0)
    {
        return None;
    }
    Some(Summary {
        version: 1,
        at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_millis() as u64,
        audio_sha256: batch.audio,
        revision: batch.revision,
        samples: batch.frames.len(),
        chart_updates: batch.updates.len(),
        last_chart_update_age_ms: batch.update_age_ms,
        display_interval_p50_ms: percentile(&batch.frames, 0.5),
        display_interval_p95_ms: percentile(&batch.frames, 0.95),
        display_interval_max_ms: percentile(&batch.frames, 1.0),
        chart_update_p50_ms: percentile(&batch.updates, 0.5),
        chart_update_p95_ms: percentile(&batch.updates, 0.95),
        chart_update_max_ms: percentile(&batch.updates, 1.0).max(batch.update_age_ms),
        display_gaps_over_50ms: batch.frames.iter().filter(|v| **v > 50.0).count(),
        chart_update_gaps_over_50ms: batch.updates.iter().filter(|v| **v > 50.0).count()
            + usize::from(batch.update_age_ms > 50.0),
        measurement: "Visible browser frames and changed chart-position DOM updates during continuous audio playback; excludes pauses, seeks, loops and revision changes. Not proof of acoustic latency or musical accuracy.",
    })
}
const OBSERVER: &str = r#"
let frames=[], updates=[], prior=null, changed=null, pos=null, rev=null, audio=null;
function sample(now){
 try {
 const view=document.querySelector('[data-preview-position]');
 const playing=view && view.getAttribute('data-preview-playing')==='true' && !document.hidden;
 const next=view ? Number(view.getAttribute('data-preview-position')) : null;
 const version=view ? view.getAttribute('data-preview-revision') : null;
 const source=view ? view.getAttribute('data-preview-audio') : null;
 if(!playing || version!==rev || source!==audio || (pos!==null && next<pos)) {frames=[];updates=[];prior=null;changed=null;pos=null;rev=version;audio=source;}
 if(playing){
  dioxus.send({type:'tick'});
  if(prior!==null)frames.push(now-prior);
  prior=now;
  if(next!==pos){if(changed!==null)updates.push(now-changed);changed=now;pos=next;}
  if(frames.length>=120){dioxus.send({type:'batch',frames,updates,update_age_ms:changed===null?0:now-changed,audio:view.getAttribute('data-preview-audio'),revision:Number(version)});frames=[];updates=[];}
 }
 } catch(error) { dioxus.send({type:'fault',message:String(error)}); return; }
 requestAnimationFrame(sample);
}
await new Promise(() => { requestAnimationFrame(sample); });
"#;
pub fn use_frame_pacing(mut studio: Studio) -> Signal<Instant> {
    let mut last_tick = use_signal(Instant::now);
    use_future(move || async move {
        let mut probe = dioxus_document::eval(OBSERVER);
        loop {
            let event = match probe.recv::<FrameEvent>().await {
                Ok(event) => event,
                Err(error) => {
                    studio.error.set(Some(format!(
                        "Display refresh link stopped ({error}); preview is using its timer clock."
                    )));
                    break;
                }
            };
            let batch = match event {
                FrameEvent::Tick => {
                    last_tick.set(Instant::now());
                    studio.tick(true);
                    continue;
                }
                FrameEvent::Fault { message } => {
                    studio.error.set(Some(format!("Display refresh link stopped ({message}); preview is using its timer clock.")));
                    break;
                }
                FrameEvent::Batch { batch } => batch,
            };
            let current = studio.doc.read().as_ref().is_some_and(|doc| {
                doc.transport.playing
                    && doc.audio_sha256 == batch.audio
                    && doc.revision == batch.revision
            });
            if !current || !(studio.audio_ready)() {
                continue;
            }
            let Some(summary) = summarize(batch) else {
                continue;
            };
            let Some(store) = studio.store.read().clone() else {
                continue;
            };
            // Tiny local diagnostic sidecar; never touches source files/audit decisions.
            if let Ok(mut file) = tempfile::NamedTempFile::new_in(&store.root)
                && let Ok(bytes) = serde_json::to_vec_pretty(&summary)
                && file.write_all(&bytes).is_ok()
            {
                let _ = file.persist(store.root.join("frame-pacing.json"));
            }
        }
    });
    last_tick
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reports_tail_stalls_and_rejects_invalid_samples() {
        let mut frames = vec![16.666; 120];
        frames[119] = 80.0;
        let report = summarize(Batch {
            frames,
            updates: vec![16.666; 100],
            audio: "a".repeat(64),
            revision: 2,
            update_age_ms: 0.,
        })
        .unwrap();
        assert_eq!(report.display_gaps_over_50ms, 1);
        assert_eq!(report.display_interval_max_ms, 80.0);
        assert!(report.display_interval_p95_ms < 17.0);
        assert!(
            summarize(Batch {
                frames: vec![f64::NAN; 120],
                updates: vec![16.; 100],
                audio: String::new(),
                revision: 0,
                update_age_ms: 0.
            })
            .is_none()
        );
        let stalled = summarize(Batch {
            frames: vec![16.666; 120],
            updates: Vec::new(),
            audio: "a".repeat(64),
            revision: 2,
            update_age_ms: 2000.,
        })
        .unwrap();
        assert_eq!(stalled.chart_updates, 0);
        assert_eq!(stalled.chart_update_max_ms, 2000.);
        assert_eq!(stalled.chart_update_gaps_over_50ms, 1);
    }
}
