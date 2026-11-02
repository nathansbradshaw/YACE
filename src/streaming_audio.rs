//! Bounded native audition: mix source-rate frames on a worker, then stretch.
//!
//! Only consumed output advances the source clock. The device-side iterator
//! reads preallocated blocks; decoding, mixing and DSP never run on that thread.
use anyhow::{Context, Result, bail, ensure};
use crossbeam_queue::ArrayQueue;
use serde::Serialize;
use std::{
    collections::VecDeque,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, Thread},
    time::{Duration, Instant},
};
use timestretch::{
    Backend, OutputStamp, Parameters, PreparedSession, ProcessStatus, SessionConfig, SourceAccess,
    StreamSession, TempoRate, TransitionEnvelope, Transport,
};

const BLOCK_FRAMES: usize = 256;
const QUEUE_BLOCKS: usize = 8;
const POOL_BLOCKS: usize = QUEUE_BLOCKS + 2;
const INPUT_FRAMES: usize = 2048;
const WORKER_POLL: Duration = Duration::from_millis(2);

/// Fill interleaved stereo frames at an original-recording frame offset.
/// The reader runs only on the processing worker and should share decoded PCM.
pub type Reader = Box<dyn FnMut(u64, &mut [f32]) -> Result<()> + Send>;

#[derive(Clone, Copy)]
struct Desired {
    epoch: u64,
    seek_frame: u64,
    rate: f64,
    loop_frames: Option<(u64, u64)>,
}
struct Block {
    samples: Box<[f32; BLOCK_FRAMES * 2]>,
    epoch: u64,
    sequence: u64,
    frames: usize,
}
struct Shared {
    desired: Mutex<Desired>,
    epoch: AtomicU64,
    queue: ArrayQueue<Block>,
    free: ArrayQueue<Block>,
    // Never reset on seek. One atomic avoids torn epoch/output-cursor pairs.
    consumed_sequence: AtomicU64,
    position: AtomicU64,
    position_epoch: AtomicU64,
    ready_epoch: AtomicU64,
    finished_epoch: AtomicU64,
    stopped: AtomicBool,
    exited: AtomicBool,
    priming_frames: AtomicU64,
    underflow_frames: AtomicU64,
    achieved_rate: AtomicU64,
    loop_wraps: AtomicU64,
    diagnostics: Mutex<Diagnostics>,
    fault: Mutex<Option<String>>,
}

/// Buffer and algorithm metadata. This is not a DAC latency measurement.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Diagnostics {
    pub queue_capacity_frames: usize,
    pub pooled_blocks: usize,
    pub startup_source_frames: usize,
    pub source_lookahead_frames: usize,
    pub output_alignment_delay_frames: usize,
    pub control_quantum_frames: usize,
    pub pending_output_frames: usize,
    pub unacknowledged_output_frames: usize,
    pub adapter_bytes: usize,
    pub native_buffer_bytes: usize,
    pub fft_plan_bytes: Option<usize>,
    pub source_requests: u64,
    pub collected_output_frames: u64,
}

#[derive(Clone)]
pub struct Controller {
    shared: Arc<Shared>,
    worker: Thread,
    sample_rate: u32,
    source_frames: u64,
}
pub struct AudioSource {
    shared: Arc<Shared>,
    worker: Thread,
    sample_rate: u32,
    block: Option<Block>,
    block_frame: usize,
    stereo: [f32; 2],
    right_channel: bool,
    receipt: Option<u64>,
    observed_epoch: u64,
    envelope: TransitionEnvelope,
    last: [f32; 2],
    was_valid: bool,
}
#[derive(Clone, Copy)]
struct Ledger {
    sequence: u64,
    stamp: OutputStamp,
}

/// Start one bounded worker. Keep the controller while the device owns source.
pub fn start(
    sample_rate: u32,
    source_frames: u64,
    rate: f64,
    reader: Reader,
) -> Result<(Controller, AudioSource)> {
    validate_rate(rate)?;
    ensure!(
        sample_rate > 0 && source_frames > 0,
        "Empty audition source"
    );
    let shared = Arc::new(Shared {
        desired: Mutex::new(Desired {
            epoch: 1,
            seek_frame: 0,
            rate,
            loop_frames: None,
        }),
        epoch: AtomicU64::new(1),
        queue: ArrayQueue::new(QUEUE_BLOCKS),
        free: ArrayQueue::new(POOL_BLOCKS),
        consumed_sequence: AtomicU64::new(0),
        position: AtomicU64::new(0_f64.to_bits()),
        position_epoch: AtomicU64::new(1),
        ready_epoch: AtomicU64::new(0),
        finished_epoch: AtomicU64::new(0),
        stopped: AtomicBool::new(false),
        exited: AtomicBool::new(false),
        priming_frames: AtomicU64::new(0),
        underflow_frames: AtomicU64::new(0),
        achieved_rate: AtomicU64::new(rate.to_bits()),
        loop_wraps: AtomicU64::new(0),
        diagnostics: Mutex::new(Diagnostics::default()),
        fault: Mutex::new(None),
    });
    for _ in 0..POOL_BLOCKS {
        assert!(
            shared
                .free
                .push(Block {
                    samples: Box::new([0.; BLOCK_FRAMES * 2]),
                    epoch: 0,
                    sequence: 0,
                    frames: 0,
                })
                .is_ok()
        );
    }
    let background = shared.clone();
    let join = thread::Builder::new()
        .name("YACE audio processing".into())
        .spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(|| {
                produce(&background, sample_rate, source_frames, reader)
            }));
            let error = match result {
                Ok(Ok(())) => None,
                Ok(Err(error)) => Some(format!("{error:#}")),
                Err(_) => Some("Audio processing worker panicked".into()),
            };
            if let Some(error) = error {
                *background.fault.lock().unwrap_or_else(|p| p.into_inner()) = Some(error);
                background
                    .finished_epoch
                    .store(background.epoch.load(Ordering::Acquire), Ordering::Release);
            }
            background.exited.store(true, Ordering::Release);
        })
        .context("Cannot start audio processing worker")?;
    let worker = join.thread().clone();
    // Source drop requests termination. Do not join a worker on a device thread.
    drop(join);
    let controller = Controller {
        shared: shared.clone(),
        worker: worker.clone(),
        sample_rate,
        source_frames,
    };
    let source = AudioSource {
        shared,
        worker,
        sample_rate,
        block: None,
        block_frame: 0,
        stereo: [0.; 2],
        right_channel: false,
        receipt: None,
        observed_epoch: 1,
        envelope: TransitionEnvelope::default(),
        last: [0.; 2],
        was_valid: true,
    };
    Ok((controller, source))
}
fn validate_rate(rate: f64) -> Result<()> {
    ensure!(
        rate.is_finite() && (0.25..=2.).contains(&rate),
        "Practice rate must be between 0.25 and 2.0"
    );
    Ok(())
}
impl Controller {
    pub fn set_rate(&self, rate: f64) -> Result<()> {
        validate_rate(rate)?;
        self.shared
            .desired
            .lock()
            .map_err(|_| anyhow::anyhow!("Audio control lock poisoned"))?
            .rate = rate;
        self.worker.unpark();
        Ok(())
    }
    /// Refill only the bounded processing window at the held source position.
    /// The device must be paused; no audible samples or play state change.
    pub fn set_rate_while_paused(&self, rate: f64) -> Result<()> {
        validate_rate(rate)?;
        let position = (self.position() * self.sample_rate as f64).floor() as u64;
        let mut desired = self
            .shared
            .desired
            .lock()
            .map_err(|_| anyhow::anyhow!("Audio control lock poisoned"))?;
        if desired.rate == rate {
            return Ok(());
        }
        desired.rate = rate;
        desired.seek_frame = position.min(self.source_frames);
        desired.epoch = desired
            .epoch
            .checked_add(1)
            .context("Audio epoch overflow")?;
        self.shared.epoch.store(desired.epoch, Ordering::Release);
        self.worker.unpark();
        Ok(())
    }
    pub fn requested_rate(&self) -> f64 {
        self.shared
            .desired
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .rate
    }
    pub fn achieved_rate(&self) -> f64 {
        f64::from_bits(self.shared.achieved_rate.load(Ordering::Acquire))
    }
    pub fn seek(&self, seconds: f64) -> Result<()> {
        ensure!(
            seconds.is_finite() && (0. ..=self.duration()).contains(&seconds),
            "Seek outside recording"
        );
        let mut desired = self
            .shared
            .desired
            .lock()
            .map_err(|_| anyhow::anyhow!("Audio control lock poisoned"))?;
        desired.seek_frame = (seconds * self.sample_rate as f64).floor() as u64;
        if let Some((start, end)) = desired.loop_frames
            && desired.seek_frame >= end
        {
            desired.seek_frame = start + (desired.seek_frame - start) % (end - start);
        }
        desired.epoch = desired
            .epoch
            .checked_add(1)
            .context("Audio epoch overflow")?;
        self.shared.epoch.store(desired.epoch, Ordering::Release);
        self.worker.unpark();
        Ok(())
    }
    pub fn set_loop(&self, range: Option<(f64, f64)>) -> Result<()> {
        let frames = if let Some((start, end)) = range {
            ensure!(
                start.is_finite()
                    && end.is_finite()
                    && start >= 0.
                    && end > start
                    && end <= self.duration(),
                "Loop outside recording"
            );
            let start = (start * self.sample_rate as f64).floor() as u64;
            let end = (end * self.sample_rate as f64).floor() as u64;
            ensure!(end > start, "Loop shorter than one source frame");
            Some((start, end))
        } else {
            None
        };
        let position = (self.position() * self.sample_rate as f64).floor() as u64;
        let mut desired = self
            .shared
            .desired
            .lock()
            .map_err(|_| anyhow::anyhow!("Audio control lock poisoned"))?;
        if desired.loop_frames == frames {
            return Ok(());
        }
        desired.loop_frames = frames;
        desired.seek_frame =
            frames.map_or(
                position,
                |(start, end)| if position >= end { start } else { position },
            );
        desired.epoch = desired
            .epoch
            .checked_add(1)
            .context("Audio epoch overflow")?;
        self.shared.epoch.store(desired.epoch, Ordering::Release);
        self.worker.unpark();
        Ok(())
    }
    pub fn duration(&self) -> f64 {
        self.source_frames as f64 / self.sample_rate as f64
    }
    pub fn loop_range(&self) -> Option<(f64, f64)> {
        self.shared
            .desired
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .loop_frames
            .map(|(start, end)| {
                (
                    start as f64 / self.sample_rate as f64,
                    end as f64 / self.sample_rate as f64,
                )
            })
    }
    pub fn position(&self) -> f64 {
        let desired = self
            .shared
            .desired
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let frame = if self.shared.position_epoch.load(Ordering::Acquire) == desired.epoch {
            f64::from_bits(self.shared.position.load(Ordering::Acquire))
        } else {
            desired.seek_frame as f64
        };
        frame / self.sample_rate as f64
    }
    pub fn loop_wraps(&self) -> u64 {
        self.shared.loop_wraps.load(Ordering::Acquire)
    }
    /// Monotonic device-consumption cursor; seeks can skip discarded spans.
    pub fn output_sequence(&self) -> u64 {
        self.shared.consumed_sequence.load(Ordering::Acquire)
    }
    pub fn ready(&self) -> bool {
        self.shared.ready_epoch.load(Ordering::Acquire) == self.shared.epoch.load(Ordering::Acquire)
    }
    /// Call only away from the UI/device callback, for initial prefill.
    pub fn wait_ready(&self, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(error) = self.error() {
                bail!(error);
            }
            if self.ready() {
                return Ok(());
            }
            ensure!(!self.worker_exited(), "Audio worker exited before prefill");
            ensure!(
                Instant::now() < deadline,
                "Audio processing prefill timed out"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }
    pub fn error(&self) -> Option<String> {
        self.shared
            .fault
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
    pub fn worker_exited(&self) -> bool {
        self.shared.exited.load(Ordering::Acquire)
    }
    pub fn diagnostics(&self) -> Diagnostics {
        self.shared
            .diagnostics
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
    pub fn priming_frames(&self) -> u64 {
        self.shared.priming_frames.load(Ordering::Relaxed)
    }
    pub fn underflow_frames(&self) -> u64 {
        self.shared.underflow_frames.load(Ordering::Relaxed)
    }
}

impl AudioSource {
    fn frame(&mut self) -> Option<([f32; 2], Option<u64>)> {
        let epoch = self.shared.epoch.load(Ordering::Acquire);
        if self.observed_epoch != epoch {
            self.observed_epoch = epoch;
            self.envelope.begin(self.last);
        }
        if self
            .block
            .as_ref()
            .is_some_and(|b| b.epoch != epoch || self.block_frame == b.frames)
        {
            assert!(self.shared.free.push(self.block.take().unwrap()).is_ok());
            self.worker.unpark();
        }
        for _ in 0..=QUEUE_BLOCKS {
            if self.block.is_some() {
                break;
            }
            if self.shared.ready_epoch.load(Ordering::Acquire) != epoch {
                break;
            }
            let Some(block) = self.shared.queue.pop() else {
                break;
            };
            if block.epoch == epoch {
                self.block = Some(block);
                self.block_frame = 0;
            } else {
                assert!(self.shared.free.push(block).is_ok());
            }
        }
        let (mut output, receipt) = if let Some(block) = &self.block {
            let offset = self.block_frame * 2;
            let frame = [block.samples[offset], block.samples[offset + 1]];
            self.block_frame += 1;
            (frame, Some(block.sequence + self.block_frame as u64))
        } else {
            if self.shared.finished_epoch.load(Ordering::Acquire) == epoch {
                return None;
            }
            if self.shared.ready_epoch.load(Ordering::Acquire) == epoch {
                self.shared.underflow_frames.fetch_add(1, Ordering::Relaxed);
            } else {
                self.shared.priming_frames.fetch_add(1, Ordering::Relaxed);
            }
            ([0.; 2], None)
        };
        let valid = receipt.is_some();
        if valid != self.was_valid {
            self.envelope.begin(self.last);
            self.was_valid = valid;
        }
        self.envelope.apply(&mut output, 2);
        self.last = output;
        Some((output, receipt))
    }
}
impl Iterator for AudioSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.right_channel {
            self.right_channel = false;
            if let Some(end) = self.receipt.take() {
                self.shared.consumed_sequence.store(end, Ordering::Release);
            }
            return Some(self.stereo[1]);
        }
        let (frame, receipt) = self.frame()?;
        self.stereo = frame;
        self.receipt = receipt;
        self.right_channel = true;
        Some(frame[0])
    }
}
impl rodio::Source for AudioSource {
    fn current_span_len(&self) -> Option<usize> {
        Some(BLOCK_FRAMES * 2)
    }
    fn channels(&self) -> rodio::ChannelCount {
        2.try_into().unwrap()
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        self.sample_rate.try_into().unwrap()
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}
impl Drop for AudioSource {
    fn drop(&mut self) {
        if let Some(block) = self.block.take() {
            assert!(self.shared.free.push(block).is_ok());
        }
        self.shared.stopped.store(true, Ordering::Release);
        self.worker.unpark();
    }
}
fn acknowledge(
    session: &mut StreamSession,
    ledger: &mut VecDeque<Ledger>,
    consumed: u64,
) -> Result<()> {
    while let Some(entry) = ledger.front().copied() {
        if consumed <= entry.sequence {
            break;
        }
        let frames = (consumed - entry.sequence).min(entry.stamp.frames as u64) as usize;
        session.acknowledge(entry.stamp.consumed(frames)?)?;
        if frames != entry.stamp.frames {
            break;
        }
        ledger.pop_front();
    }
    Ok(())
}
fn produce(
    shared: &Shared,
    sample_rate: u32,
    source_frames: u64,
    mut reader: Reader,
) -> Result<()> {
    let initial = *shared.desired.lock().unwrap_or_else(|p| p.into_inner());
    let mut session = PreparedSession::new(SessionConfig {
        channels: 2,
        source_sample_rate: sample_rate,
        output_sample_rate: sample_rate,
        source_access: SourceAccess::Seekable,
        source_frames: Some(source_frames),
        max_inflight_frames: 65_536,
        backend: Backend::Portable,
        parameters: Parameters::keylock(TempoRate::new(initial.rate)?),
        ..SessionConfig::default()
    })?
    .activate();
    // Controls may arrive before this thread starts. Apply that first snapshot,
    // including an immediate seek/loop, before considering its epoch current.
    session.set_loop(initial.loop_frames)?;
    session.seek(initial.seek_frame)?;
    session.set_transport(Transport::Playing);
    let memory = session.owned_memory();
    let mut metrics = Diagnostics {
        queue_capacity_frames: QUEUE_BLOCKS * BLOCK_FRAMES,
        pooled_blocks: POOL_BLOCKS,
        adapter_bytes: memory.adapter_bytes,
        native_buffer_bytes: memory.native_buffer_bytes,
        fft_plan_bytes: memory.fft_plan_bytes,
        ..Default::default()
    };
    let mut input = [0.; INPUT_FRAMES * 2];
    let mut epoch = initial.epoch;
    let mut sequence = 0_u64;
    let mut command = 0_u64;
    let mut pending_command = None;
    let mut ledger = VecDeque::with_capacity(POOL_BLOCKS);
    let mut previous_position = session.position();
    while !shared.stopped.load(Ordering::Acquire) {
        let desired = *shared.desired.lock().unwrap_or_else(|p| p.into_inner());
        if desired.epoch != epoch {
            while let Some(block) = shared.queue.pop() {
                assert!(shared.free.push(block).is_ok());
            }
            ledger.clear();
            session.set_loop(desired.loop_frames)?;
            session.seek(desired.seek_frame)?;
            session.set_transport(Transport::Playing);
            epoch = desired.epoch;
            pending_command = None;
            previous_position = session.position();
        }
        acknowledge(
            &mut session,
            &mut ledger,
            shared.consumed_sequence.load(Ordering::Acquire),
        )?;
        let position = session.position();
        if position < previous_position {
            shared.loop_wraps.fetch_add(1, Ordering::Release);
        }
        previous_position = position;
        shared.position.store(position.to_bits(), Ordering::Release);
        shared.position_epoch.store(epoch, Ordering::Release);
        let applied = session.applied_parameters();
        shared
            .achieved_rate
            .store(applied.tempo().to_bits(), Ordering::Release);
        if pending_command.is_some_and(|id| applied.command_id >= id) {
            pending_command = None;
        }
        if pending_command.is_none() && applied.requested.tempo.get() != desired.rate {
            command = command.checked_add(1).context("Audio command overflow")?;
            session.schedule(
                command,
                session.commit_horizon(),
                Parameters::keylock(TempoRate::new(desired.rate)?),
            )?;
            pending_command = Some(command);
        }
        let timing = session.timing();
        metrics.startup_source_frames = timing.startup_source_frames;
        metrics.source_lookahead_frames = timing.source_lookahead_frames;
        metrics.output_alignment_delay_frames = timing.output_alignment_delay_frames;
        metrics.control_quantum_frames = timing.control_quantum_frames;
        metrics.pending_output_frames = timing.pending_output_frames;
        metrics.unacknowledged_output_frames = timing.unacknowledged_output_frames;
        *shared.diagnostics.lock().unwrap_or_else(|p| p.into_inner()) = metrics.clone();
        if shared.queue.is_full() || session.rendered_finished() {
            shared.ready_epoch.store(epoch, Ordering::Release);
        }
        if session.finished() {
            shared.finished_epoch.store(epoch, Ordering::Release);
            thread::park_timeout(WORKER_POLL);
            continue;
        }
        if shared.queue.is_full() {
            thread::park_timeout(WORKER_POLL);
            continue;
        }
        let report = session.advance(4);
        if report.status == ProcessStatus::NeedSource {
            let request = session
                .source_request()
                .context("Missing audio source request")?;
            let frames = request.frames.min(INPUT_FRAMES);
            reader(request.start_frame, &mut input[..frames * 2])?;
            session.supply(request, &input[..frames * 2])?;
            metrics.source_requests += 1;
        } else if report.status == ProcessStatus::Fault {
            return Err(session.fault().context("Missing audio fault")?.into());
        }
        if session.output_ready() == 0 {
            continue;
        }
        let mut block = shared.free.pop().context("Audio buffer pool exhausted")?;
        let stamp = session.read_output(&mut block.samples[..])?;
        if stamp.frames == 0 {
            assert!(shared.free.push(block).is_ok());
            continue;
        }
        block.frames = stamp.frames;
        block.epoch = epoch;
        block.sequence = sequence;
        ledger.push_back(Ledger { sequence, stamp });
        ensure!(
            ledger.len() <= POOL_BLOCKS,
            "Audio stamp ledger exceeded its bound"
        );
        sequence = sequence
            .checked_add(stamp.frames as u64)
            .context("Audio sequence overflow")?;
        metrics.collected_output_frames += stamp.frames as u64;
        assert!(shared.queue.push(block).is_ok());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tone(sample_rate: u32, rate: f64) -> (Controller, AudioSource) {
        let input: Arc<Vec<f32>> = Arc::new(
            (0..sample_rate * 2)
                .flat_map(|i| {
                    let value = (i as f64 * std::f64::consts::TAU * 440. / sample_rate as f64).sin()
                        as f32
                        * 0.2;
                    [value, -value]
                })
                .collect(),
        );
        start(
            sample_rate,
            input.len() as u64 / 2,
            rate,
            Box::new(move |frame, out| {
                let start = frame as usize * 2;
                out.copy_from_slice(
                    input
                        .get(start..start + out.len())
                        .context("Reader bounds")?,
                );
                Ok(())
            }),
        )
        .unwrap()
    }
    fn until(control: &Controller, mut action: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut progress = (control.position().to_bits(), control.output_sequence());
        let mut progressed_at = Instant::now();
        while !action() {
            assert!(control.error().is_none(), "{:?}", control.error());
            let next = (control.position().to_bits(), control.output_sequence());
            if next != progress {
                progress = next;
                progressed_at = Instant::now();
            }
            // Offline debug DSP can take longer than ten seconds while another
            // release links. Detect a lack of progress, not total render time.
            assert!(
                Instant::now() < deadline && progressed_at.elapsed() < Duration::from_secs(10),
                "Audio protocol stalled: {:?}",
                control.diagnostics()
            );
            thread::sleep(Duration::from_micros(250));
        }
    }
    fn consume(source: &mut AudioSource) -> bool {
        for _ in 0..BLOCK_FRAMES {
            let Some(left) = source.next() else {
                return false;
            };
            let right = source.next().expect("Source ended inside a stereo frame");
            assert!(left.is_finite() && right.is_finite());
            assert!((left + right).abs() < 1e-5, "Stereo alignment changed");
        }
        true
    }
    #[test]
    fn prefill_does_not_advance_source_clock_and_only_complete_frames_are_consumed() {
        let (control, mut source) = tone(44_100, 1.);
        control.wait_ready(Duration::from_secs(5)).unwrap();
        assert_eq!(control.position(), 0.);
        assert_eq!(source.shared.consumed_sequence.load(Ordering::Acquire), 0);
        source.next().unwrap();
        assert_eq!(source.shared.consumed_sequence.load(Ordering::Acquire), 0);
        source.next().unwrap();
        assert_eq!(source.shared.consumed_sequence.load(Ordering::Acquire), 1);
        until(&control, || control.position() > 0.);
        assert!(control.position() <= 1. / 44_100. + 1e-9);
        drop(source);
        until(&control, || control.worker_exited());
    }
    #[test]
    fn source_clock_reaches_exact_end_at_all_supported_rates() {
        for rate in [0.25, 0.5, 0.83, 1., 1.5, 2.] {
            let (control, mut source) = tone(44_100, rate);
            control.wait_ready(Duration::from_secs(5)).unwrap();
            until(&control, || !consume(&mut source));
            assert_eq!(control.position(), 2., "Source clock missed end at {rate}");
            assert!((control.achieved_rate() - rate).abs() / rate < 0.002);
            let diagnostics = control.diagnostics();
            assert_eq!(diagnostics.queue_capacity_frames, 2048);
            assert_eq!(diagnostics.pooled_blocks, 10);
            drop(source);
            until(&control, || control.worker_exited());
        }
    }
    #[test]
    fn immediate_seek_rapid_retarget_and_loop_keep_epochs_and_latest_request() {
        let (control, mut source) = tone(48_000, 1.);
        // In particular this must also work if the worker has not started yet.
        control.seek(0.5).unwrap();
        control.wait_ready(Duration::from_secs(5)).unwrap();
        assert_eq!(control.position(), 0.5);
        for i in 0..10_000 {
            control
                .set_rate(if i % 2 == 0 { 0.25 } else { 2. })
                .unwrap();
        }
        control.set_rate(0.83).unwrap();
        thread::sleep(Duration::from_millis(5));
        assert_eq!(
            control.position(),
            0.5,
            "Rate changes consumed paused audio"
        );
        until(&control, || {
            consume(&mut source);
            (control.achieved_rate() - 0.83).abs() < 0.002
        });
        control.set_loop(Some((0.25, 0.75))).unwrap();
        control.wait_ready(Duration::from_secs(5)).unwrap();
        until(&control, || {
            consume(&mut source);
            control.loop_wraps() >= 3
        });
        assert!((0.25..0.75).contains(&control.position()));
        control.set_loop(None).unwrap();
        control.seek(1.5).unwrap();
        control.wait_ready(Duration::from_secs(5)).unwrap();
        assert_eq!(control.position(), 1.5);
        until(&control, || !consume(&mut source));
        assert_eq!(control.position(), 2.);
        // Manual consumers can seek again at EOF; a device that dropped its
        // source instead creates a fresh source from the same decoded PCM.
        control.seek(1.).unwrap();
        control.wait_ready(Duration::from_secs(5)).unwrap();
        assert_eq!(control.position(), 1.);
        assert!(consume(&mut source));
        drop(source);
        until(&control, || control.worker_exited());
    }
    #[test]
    fn paused_retarget_refills_at_held_position_and_loops_allow_a_lead_in() {
        let (control, mut source) = tone(48_000, 1.);
        control.seek(0.5).unwrap();
        control.wait_ready(Duration::from_secs(5)).unwrap();
        for i in 0..1000 {
            control
                .set_rate_while_paused(if i % 2 == 0 { 0.25 } else { 2. })
                .unwrap();
        }
        control.set_rate_while_paused(0.83).unwrap();
        control.wait_ready(Duration::from_secs(5)).unwrap();
        assert_eq!(control.position(), 0.5);
        assert!((control.achieved_rate() - 0.83).abs() < 0.002);
        control.set_loop(Some((0.5, 1.5))).unwrap();
        control.seek(0.).unwrap();
        control.wait_ready(Duration::from_secs(5)).unwrap();
        assert_eq!(
            control.position(),
            0.,
            "Loop incorrectly discarded its lead-in"
        );
        until(&control, || {
            consume(&mut source);
            control.loop_wraps() >= 1
        });
        assert!((0.5..1.5).contains(&control.position()));
        drop(source);
        until(&control, || control.worker_exited());
    }
    #[test]
    fn invalid_controls_are_atomic_and_reader_failure_is_reported() {
        let (control, source) = tone(48_000, 1.);
        assert!(control.set_rate(f64::NAN).is_err());
        assert!(control.set_rate(0.24).is_err());
        assert!(control.seek(-1.).is_err());
        assert!(control.set_loop(Some((0.5, 3.))).is_err());
        assert_eq!(control.requested_rate(), 1.);
        assert_eq!(control.position(), 0.);
        drop(source);
        until(&control, || control.worker_exited());
        let (control, source) = start(
            48_000,
            48_000,
            1.,
            Box::new(|_, _| bail!("Unavailable source")),
        )
        .unwrap();
        assert!(
            control
                .wait_ready(Duration::from_secs(5))
                .unwrap_err()
                .to_string()
                .contains("Unavailable source")
        );
        drop(source);
        until(&control, || control.worker_exited());
    }
}
