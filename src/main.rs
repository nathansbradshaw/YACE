use anyhow::{Context, Result, ensure};
use chart_engine::{
    mapping::Transcription,
    models,
    pipeline::{self, Build},
    timing::TempoMap,
};
use clap::{Args, Parser, Subcommand};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(name = "yace", version, about = "YACE — Yet Another Charting Engine")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Open the shared Rust chart editor through a browser on this computer.
    #[cfg(feature = "browser")]
    Serve {
        #[arg(long, default_value = "output/browser-workspace")]
        workspace: PathBuf,
        #[arg(long, default_value_t = 3030)]
        port: u16,
    },
    /// Per-instrument and per-section evidence, shared with Part analysis in the editor.
    Parts {
        #[command(subcommand)]
        action: PartsCommand,
    },
    /// Compact source-bound review packets and guarded AI editor actions.
    Ai {
        #[command(subcommand)]
        action: AiCommand,
    },
    /// Read-only comparisons with human-authored charts on matching audio.
    Benchmark {
        #[command(subcommand)]
        action: BenchmarkCommand,
    },
    /// Recoverable local-audio generation jobs used by the native app.
    Generation {
        #[command(subcommand)]
        action: GenerationCommand,
    },
    /// Make editable clean-version plans from aligned lyrics and export audio.
    Clean {
        #[command(subcommand)]
        action: CleanCommand,
    },
    /// Native timeline editor and identical headless automation commands.
    Editor {
        #[command(subcommand)]
        action: EditorCommand,
    },
    /// Edit/import/validate/export lead vocals without requiring AI models.
    Vocals {
        #[command(subcommand)]
        action: VocalCommand,
    },
    /// Refine saved bass candidates against a time-aligned separated bass stem.
    Refine {
        events: PathBuf,
        #[arg(long)]
        bass_audio: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Independent audio/pattern diagnostics and synchronized listening clips.
    Review {
        folder: PathBuf,
        #[arg(long)]
        drum_audio: PathBuf,
        #[arg(long)]
        bass_audio: Option<PathBuf>,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Download or verify revision-pinned, checksum-verified pretrained weights.
    Models {
        #[command(subcommand)]
        action: ModelCommand,
    },
    /// Report the instrument tracks present in one song folder.
    Inspect { folder: PathBuf },
    /// Read-only recursive library inventory, including missing instruments.
    Scan { folder: PathBuf },
    /// Run the integrated native demucs-rs separator and keep all six stems.
    Separate {
        #[arg(long)]
        audio: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long, default_value = "models")]
        model_dir: PathBuf,
        /// Persistent source/model/parameter cache; defaults to model-dir/stem-cache.
        #[arg(long)]
        stem_cache: Option<PathBuf>,
    },
    /// Bind an existing six-stem cache to its recording with explicit provenance.
    ImportStems {
        #[arg(long)]
        audio: PathBuf,
        #[arg(long)]
        stems: PathBuf,
        /// New manifest JSON file; existing files are never overwritten.
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long)]
        alignment_method: String,
        /// Describe the actual source/alignment history and any uncertainty.
        #[arg(long)]
        alignment_details: String,
    },
    /// Transcribe audio and generate a new song folder. BPM is explicit until
    /// native beat tracking is ported; existing charts retain their tempo maps.
    Generate {
        #[arg(long)]
        audio: PathBuf,
        #[arg(long)]
        bpm: f64,
        #[command(flatten)]
        options: OutputOptions,
        #[command(flatten)]
        inference: InferenceOptions,
    },
    /// Add missing instruments to a separate copy of an existing song.
    Enhance {
        folder: PathBuf,
        #[arg(long)]
        audio: Option<PathBuf>,
        #[command(flatten)]
        options: OutputOptions,
        #[command(flatten)]
        inference: InferenceOptions,
    },
    /// Save inspectable transcription events without generating a chart.
    Transcribe {
        #[arg(long)]
        audio: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[command(flatten)]
        inference: InferenceOptions,
    },
    /// Build from editable event JSON or a General MIDI performance.
    Build {
        #[arg(long)]
        events: PathBuf,
        #[arg(long)]
        audio: PathBuf,
        #[arg(long)]
        existing: Option<PathBuf>,
        #[arg(long)]
        bpm: Option<f64>,
        #[command(flatten)]
        options: OutputOptions,
    },
}

#[derive(Subcommand)]
enum PartsCommand {
    /// Explicit optional five-part drum separation. Models are local and unbundled.
    SplitKit {
        project: PathBuf,
        #[arg(long)]
        models: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Export a source-bound separated-part listening clip (at most20seconds).
    Clip {
        project: PathBuf,
        #[arg(long)]
        kit: PathBuf,
        #[arg(long, value_parser=["kick","snare","toms","hihat","cymbals"])]
        part: String,
        #[arg(long)]
        start: f64,
        #[arg(long)]
        end: f64,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Independent model-free guitar timing evidence; does not assign notes or frets.
    GuitarAttacks {
        #[arg(long)]
        audio: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Model-free overlapping drum frequency views, not isolated kit pieces.
    DrumBands {
        #[arg(long)]
        audio: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Inspect the current saved project without modifying notes or decisions.
    Analyze {
        project: PathBuf,
        /// Optional new JSON file; the complete report is also printed.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Inspect explicit immutable chart/audio inputs, without creating a project.
    Files {
        #[arg(long)]
        audio: PathBuf,
        #[arg(long)]
        chart: PathBuf,
        #[arg(long)]
        manifest: Option<PathBuf>,
        #[arg(long)]
        events: Option<PathBuf>,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

#[derive(Args)]
struct AiReviewOptions {
    project: PathBuf,
    /// Exact track name, e.g. ExpertDoubleBass or Vocals.
    #[arg(long)]
    track: Option<String>,
    /// Save the packet to a new file; JSON is also printed to stdout.
    #[arg(short, long)]
    output: Option<PathBuf>,
    #[arg(long, default_value_t = 96)]
    note_limit: usize,
    #[arg(long, default_value_t = 1.0)]
    context_seconds: f64,
    #[arg(long)]
    include_hidden: bool,
    #[arg(long)]
    include_approved: bool,
    /// Continue after an eligible section ID from the current queue.
    #[arg(long)]
    after: Option<String>,
}
#[derive(Subcommand)]
enum AiCommand {
    /// Read a bounded pending-review page; no model or mutation.
    Review {
        #[command(flatten)]
        options: AiReviewOptions,
        #[arg(long, default_value_t = 3)]
        limit: usize,
    },
    /// Read the next single pending region/track; no automatic approval.
    Next {
        #[command(flatten)]
        options: AiReviewOptions,
    },
    /// Apply one existing editor Action using a fresh packet and AI actor.
    Apply {
        project: PathBuf,
        #[arg(long)]
        packet: PathBuf,
        #[arg(long)]
        request: PathBuf,
    },
}

fn ai_review(options: AiReviewOptions, limit: usize) -> Result<()> {
    let packet = chart_engine::ai_harness::review(
        &chart_engine::editor::Store::new(&options.project),
        chart_engine::ai_harness::Options {
            track: options.track,
            limit,
            note_limit: options.note_limit,
            context_seconds: options.context_seconds,
            include_hidden: options.include_hidden,
            include_approved: options.include_approved,
            after: options.after,
        },
    )?;
    if let Some(output) = options.output {
        write_new(&output, &serde_json::to_vec_pretty(&packet)?)?;
    }
    print(&packet)
}

fn bounded_json<T: serde::de::DeserializeOwned>(path: &Path, max: u64) -> Result<T> {
    use std::io::Read;
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(max + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= max,
        "AI JSON input exceeds its bounded size"
    );
    Ok(serde_json::from_slice(&bytes)?)
}

#[derive(Subcommand)]
enum BenchmarkCommand {
    /// Analyze an oracle vocal stem with a pinned optional model; no chart targets are read.
    VocalPitch {
        #[arg(long)]
        audio: PathBuf,
        #[arg(long)]
        model: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Export Expert targets (Guitar/Bass/Drums/ProDrums/Vocals); lyrics are text only.
    Inspect {
        #[arg(long = "notes", alias = "midi", value_name = "NOTES")]
        midi: PathBuf,
        #[arg(long, value_enum)]
        instrument: chart_engine::benchmark::Instrument,
        #[arg(long, default_value_t = 0.)]
        delay_ms: f64,
        #[arg(long)]
        lyrics_output: Option<PathBuf>,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Compare Expert attacks, ProDrums hits, guitar holds, or vocal pitch on identical audio.
    Compare {
        #[arg(
            long = "reference",
            alias = "reference-midi",
            value_name = "REFERENCE_NOTES"
        )]
        reference_midi: PathBuf,
        #[arg(
            long = "generated",
            alias = "generated-midi",
            value_name = "GENERATED_NOTES"
        )]
        generated_midi: PathBuf,
        #[arg(long)]
        reference_audio: PathBuf,
        #[arg(long)]
        generated_audio: PathBuf,
        #[arg(long, default_value_t = 0.)]
        reference_delay_ms: f64,
        #[arg(long, default_value_t = 0.)]
        generated_delay_ms: f64,
        #[arg(long, value_enum)]
        instrument: chart_engine::benchmark::Instrument,
        #[arg(short, long)]
        output: PathBuf,
    },
}

#[derive(Subcommand)]
enum GenerationCommand {
    /// Create a new job from the same versioned request used by the app.
    Create {
        #[arg(long)]
        request: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Run or resume a saved job. Completed inputs/stems/events are reused.
    Run { job: PathBuf },
    /// Read progress and errors without starting any work.
    Show { job: PathBuf },
}

#[derive(Subcommand)]
enum EditorCommand {
    /// Copy a .chart song into a recoverable editing project.
    Create {
        song: PathBuf,
        #[arg(long)]
        markers: Option<PathBuf>,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Inspect notes, transport, flags, decisions and revision as JSON.
    Show { project: PathBuf },
    /// Apply one revision-checked JSON command, including while the UI is open.
    Apply {
        project: PathBuf,
        #[arg(long)]
        request: PathBuf,
    },
    /// Open the native timeline without requiring AI models.
    Open { project: PathBuf },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum CleanModeArg {
    FullMix,
    VocalOnly,
}
#[derive(Subcommand)]
enum CleanCommand {
    /// Recognize English lyrics locally and build explicit-word ranges from audio.
    Detect {
        #[arg(long)]
        audio: PathBuf,
        #[arg(long)]
        policy: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "full-mix")]
        mode: CleanModeArg,
        #[arg(long)]
        model_dir: Option<PathBuf>,
        /// Require an already downloaded checksum-verified lyric model.
        #[arg(long)]
        no_download: bool,
        #[arg(long, requires = "alignment")]
        vocal_stem: Option<PathBuf>,
        #[arg(long, requires = "vocal_stem")]
        alignment: Option<PathBuf>,
        /// New directory for generated words.json, plan.json and recognition.json.
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Match supplied word timings against common explicit markers or a custom policy.
    Plan {
        #[arg(long)]
        words: PathBuf,
        /// Custom policy JSON; otherwise use common English explicit markers.
        #[arg(long)]
        policy: Option<PathBuf>,
        #[arg(long, value_enum)]
        mode: CleanModeArg,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Export clean.wav plus its plan/receipt into a new directory.
    Render {
        #[arg(long)]
        audio: PathBuf,
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        vocal_stem: Option<PathBuf>,
        #[arg(long)]
        alignment: Option<PathBuf>,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Record supplied stem alignment provenance and exact input hashes.
    BindStem {
        #[arg(long)]
        audio: PathBuf,
        #[arg(long)]
        stem: PathBuf,
        #[arg(long)]
        evidence: String,
        #[arg(short, long)]
        output: PathBuf,
    },
}

#[derive(Subcommand)]
enum VocalCommand {
    /// Attach saved generation checks without replacing vocal edits or approving notes.
    AttachReview {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        report: PathBuf,
        #[arg(long)]
        expected_revision: u64,
    },
    /// Generate a retained vocal proposal for an existing editor project.
    GenerateProject {
        project: PathBuf,
        #[arg(long)]
        recognize: bool,
        #[arg(long, value_enum, default_value = "auto")]
        language: chart_engine::lyric_language::Language,
        /// Use the pinned local pitch model to corroborate uncertain vocal pitches.
        #[arg(long)]
        neural_pitch: bool,
        #[arg(long)]
        lookup_lyrics: bool,
        #[arg(long)]
        download_model: bool,
        #[arg(long)]
        model_dir: Option<PathBuf>,
    },
    /// Estimate sung pitches and phrases from an aligned vocal stem. Models
    /// are loaded only for explicit --recognize or --neural-pitch assistance.
    Generate {
        #[arg(long)]
        audio: PathBuf,
        #[arg(long)]
        stem_manifest: PathBuf,
        #[arg(long, conflicts_with = "bpm")]
        chart: Option<PathBuf>,
        #[arg(long, required_unless_present = "chart")]
        bpm: Option<f64>,
        #[arg(long, conflicts_with = "recognize")]
        words: Option<PathBuf>,
        /// Supplied plain lyric text, retained separately when not audio-aligned.
        #[arg(long)]
        lyrics: Option<PathBuf>,
        /// Lyric-source JSON retaining provider provenance and optional timed LRC lines.
        #[arg(long, conflicts_with = "lyrics")]
        lyric_source: Option<PathBuf>,
        /// Optional local acoustic lyric alignment / word recognition.
        #[arg(long)]
        recognize: bool,
        #[arg(long, value_enum, default_value = "auto")]
        language: chart_engine::lyric_language::Language,
        /// Optional independent local pitch-model evidence; also works with supplied words.
        #[arg(long)]
        neural_pitch: bool,
        #[arg(long)]
        model_dir: Option<PathBuf>,
        #[arg(long)]
        download_model: bool,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Validate an editable vocal document; musical approval is separate.
    Validate { document: PathBuf },
    /// Extract the supported PART VOCALS subset into editable JSON.
    Import {
        midi: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Export vocals into a new MIDI or append to a copy of an existing MIDI.
    Export {
        document: PathBuf,
        /// Existing MIDI to preserve, including its tempo map and resolution.
        #[arg(long, conflicts_with = "bpm")]
        midi: Option<PathBuf>,
        /// Explicit tempo for a new vocal-only MIDI.
        #[arg(long, required_unless_present = "midi")]
        bpm: Option<f64>,
        #[arg(short, long)]
        output: PathBuf,
    },
}

#[derive(Subcommand)]
enum ModelCommand {
    Fetch {
        #[arg(long, default_value = "models")]
        directory: PathBuf,
        /// Also download the optional higher-quality Piano separator.
        #[arg(long)]
        high_quality_piano: bool,
        /// Also download the optional multilingual vocal-recognition model.
        #[arg(long)]
        multilingual_lyrics: bool,
    },
    Verify {
        #[arg(long, default_value = "models")]
        directory: PathBuf,
    },
}

#[derive(Args)]
struct OutputOptions {
    #[arg(short, long)]
    output: PathBuf,
    #[arg(long)]
    title: Option<String>,
    #[arg(long)]
    artist: Option<String>,
    /// Emit standard drums without Pro cymbal markers.
    #[arg(long)]
    standard_drums: bool,
    /// Look up exact artist/title/duration lyrics in LRCLIB.
    #[arg(long)]
    lyrics: bool,
}

#[derive(Args)]
struct InferenceOptions {
    /// Sound arranged into five-fret Guitar; use other for a synth/melodic stem.
    #[arg(long, value_enum, default_value = "guitar")]
    guitar_source: chart_engine::mapping::GuitarSource,
    #[arg(long, default_value = "models")]
    model_dir: PathBuf,
    /// Stem folder containing stem-manifest.json; bare legacy folders require import-stems.
    #[arg(long, conflicts_with = "stem_manifest")]
    stems: Option<PathBuf>,
    /// Source/hash-bound manifest shared with the instrument-review mixer.
    #[arg(long)]
    stem_manifest: Option<PathBuf>,
    /// Persistent automatic separation cache; defaults to model-dir/stem-cache.
    #[arg(long)]
    stem_cache: Option<PathBuf>,
    /// Transcribe drums only, using a bound drum stem by default.
    #[arg(long)]
    drums_only: bool,
    /// Instrument drafts to generate (comma-separated); piano=Keys, pro_piano=Pro Keys.
    #[arg(long, value_delimiter = ',', value_parser = ["guitar", "bass", "drums", "piano", "pro_piano"], conflicts_with = "drums_only")]
    instruments: Vec<String>,
    /// Keyboard sound: piano uses Kong; experimental synth uses MuScriptor on the full mix.
    #[arg(long, default_value = "piano", value_parser = ["piano", "synth"])]
    keyboard_sound: String,
    /// Piano source quality; high-quality uses the optional slower native separator.
    #[arg(long, default_value = "standard", value_parser = ["standard", "high-quality"])]
    piano_quality: String,
    /// Optional separate, source-bound keyboard analysis manifest for mixed-instrument jobs.
    #[arg(long)]
    keyboard_analysis: Option<PathBuf>,
    /// Explicit fallback: run drums-only ADTOF on the original mix without isolation.
    #[arg(long, requires = "drums_only", conflicts_with_all = ["stems", "stem_manifest"])]
    full_mix_drums: bool,
}

impl InferenceOptions {
    fn requested(&self) -> Vec<String> {
        if self.drums_only {
            vec!["drums".into()]
        } else if !self.instruments.is_empty() {
            self.instruments.clone()
        } else {
            vec!["drums".into(), "guitar".into(), "bass".into()]
        }
    }
    fn isolation(&self) -> pipeline::IsolationOptions<'_> {
        pipeline::IsolationOptions {
            guitar_source: self.guitar_source,
            stems: self.stems.as_deref(),
            manifest: self.stem_manifest.as_deref(),
            cache_dir: self.stem_cache.as_deref(),
            full_mix_drums: self.full_mix_drums,
            keyboard_sound: if self.keyboard_sound == "synth" {
                chart_engine::keyboard_detection::KeyboardSound::Synth
            } else {
                chart_engine::keyboard_detection::KeyboardSound::Piano
            },
            piano_quality: if self.piano_quality == "high-quality" {
                chart_engine::keyboard_detection::PianoQuality::HighQuality
            } else {
                chart_engine::keyboard_detection::PianoQuality::Standard
            },
            keyboard_analysis: self.keyboard_analysis.as_deref(),
        }
    }
}

fn print(value: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn write_new(output: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.persist_noclobber(output)?;
    Ok(())
}

fn vocal_document(path: &Path) -> Result<chart_engine::vocals::VocalChart> {
    let chart: chart_engine::vocals::VocalChart = serde_json::from_slice(&fs::read(path)?)?;
    chart.validate()?;
    Ok(chart)
}

fn build(
    audio: &Path,
    existing: Option<&Path>,
    timing: Option<TempoMap>,
    options: OutputOptions,
    transcription: &Transcription,
) -> Result<()> {
    let mut meta = pipeline::metadata(audio)?;
    if let Some(folder) = existing {
        for (key, value) in pipeline::ini_fields(&pipeline::song_ini(folder)?) {
            match key.as_str() {
                "name" => meta.name = value,
                "artist" => meta.artist = value,
                "album" => meta.album = value,
                "year" => meta.year = value,
                "genre" => meta.genre = value,
                _ => {}
            }
        }
    }
    if let Some(title) = options.title {
        meta.name = title;
    }
    if let Some(artist) = options.artist {
        meta.artist = artist;
    }
    let report = pipeline::build(Build {
        audio,
        output: &options.output,
        existing,
        timing,
        transcription,
        meta,
        pro: !options.standard_drums,
        lookup_lyrics: options.lyrics,
    })?;
    print(&report)
}

fn main() -> Result<()> {
    let Some(command) = Cli::parse().command else {
        #[cfg(all(feature = "editor", not(feature = "browser")))]
        return chart_engine::editor_ui::launch();
        #[cfg(feature = "browser")]
        return chart_engine::browser::serve(Path::new("output/browser-workspace"), 3030);
        #[cfg(not(any(feature = "editor", feature = "browser")))]
        anyhow::bail!(
            "Choose a command with --help, or build --features editor for the native app"
        );
    };
    match command {
        #[cfg(feature = "browser")]
        Commands::Serve { workspace, port } => chart_engine::browser::serve(&workspace, port)?,
        Commands::Ai { action } => match action {
            AiCommand::Review { options, limit } => ai_review(options, limit)?,
            AiCommand::Next { options } => ai_review(options, 1)?,
            AiCommand::Apply {
                project,
                packet,
                request,
            } => {
                let packet = bounded_json(&packet, 512 * 1024)?;
                // Whole vocal replacements may legitimately exceed a context
                // packet, but still use the existing document validator.
                let request = bounded_json(&request, 16 * 1024 * 1024)?;
                print(&chart_engine::ai_harness::apply(
                    &chart_engine::editor::Store::new(&project),
                    &packet,
                    request,
                )?)?;
            }
        },
        Commands::Parts { action } => {
            if let PartsCommand::SplitKit {
                project,
                models,
                output,
            } = &action
            {
                let store = chart_engine::editor::Store::new(project);
                let doc = store.load()?;
                let result =
                    chart_engine::part_analysis::prepare_project_drum_parts(&store, &doc, models)?;
                if let Some(output) = output {
                    write_new(output, &serde_json::to_vec_pretty(&result)?)?;
                }
                print(&result)?;
                return Ok(());
            }
            if let PartsCommand::Clip {
                project,
                kit,
                part,
                start,
                end,
                output,
            } = &action
            {
                ensure!(!output.exists(), "Clip output already exists");
                let store = chart_engine::editor::Store::new(project);
                let doc = store.load()?;
                let set = serde_json::from_slice(&fs::read(kit)?)?;
                let clip = chart_engine::part_analysis::audition_clip(
                    &store, &doc, &set, part, *start, *end,
                )?;
                write_new(output, &clip.wav)?;
                print(
                    &serde_json::json!({"output":output,"part":clip.part,"start":clip.start,"end":clip.end,"source_audio_sha256":set.original_audio_sha256,"drum_audio_sha256":set.drum_audio_sha256,"musical_acceptance":"unverified"}),
                )?;
                return Ok(());
            }
            if let PartsCommand::GuitarAttacks { audio, output }
            | PartsCommand::DrumBands { audio, output } = &action
            {
                let source_hash = models::hash_file(audio)?;
                let samples = models::decode(audio, 44100, 1)?;
                let evidence = match &action {
                    PartsCommand::GuitarAttacks { .. } => {
                        serde_json::json!({"algorithm":chart_engine::guitar_attacks::ALGORITHM,"attacks":chart_engine::guitar_attacks::detect(&samples,44100)?})
                    }
                    _ => {
                        serde_json::json!({"algorithm":chart_engine::drum_parts::ALGORITHM,"bands":chart_engine::drum_parts::analyse(&samples,44100)?})
                    }
                };
                ensure!(
                    models::hash_file(audio)? == source_hash,
                    "Audio changed during analysis"
                );
                let report = serde_json::json!({"source_audio_sha256":source_hash,"sample_rate":44100,"frames":samples.len(),"evidence":evidence,"interpretation":"Uncalibrated source-clock signal features; no physical instrument identity, chart note or musical approval is inferred."});
                if let Some(output) = output {
                    write_new(output, &serde_json::to_vec_pretty(&report)?)?;
                }
                print(&report)?;
                return Ok(());
            }
            let (report, output) = match action {
                PartsCommand::Analyze { project, output } => {
                    let store = chart_engine::editor::Store::new(&project);
                    let doc = store.load()?;
                    (
                        chart_engine::part_analysis::analyze_project(&store, &doc)?,
                        output,
                    )
                }
                PartsCommand::Files {
                    audio,
                    chart,
                    manifest,
                    events,
                    output,
                } => (
                    chart_engine::part_analysis::analyze_files(
                        &audio,
                        &chart,
                        manifest.as_deref(),
                        events.as_deref(),
                    )?,
                    output,
                ),
                PartsCommand::GuitarAttacks { .. }
                | PartsCommand::DrumBands { .. }
                | PartsCommand::SplitKit { .. }
                | PartsCommand::Clip { .. } => unreachable!(),
            };
            if let Some(output) = output {
                write_new(&output, &serde_json::to_vec_pretty(&report)?)?;
            }
            print(&report)?;
        }
        Commands::Benchmark { action } => match action {
            BenchmarkCommand::VocalPitch {
                audio,
                model,
                output,
            } => {
                ensure!(!output.exists(), "Benchmark output already exists");
                let sha = models::hash_file(&audio)?;
                let alignment = chart_engine::clean::StemAlignment {
                    audio_sha256: sha.clone(), stem_sha256: sha,
                    evidence: "Oracle vocal stem is the analysis recording; exact file identity and source clock, no reference note or timing supplied".into(),
                };
                let evidence = chart_engine::vocal_pitch::analyze(
                    &audio,
                    &audio,
                    &alignment,
                    &model,
                    &|| false,
                )?;
                write_new(&output, &serde_json::to_vec_pretty(&evidence)?)?;
                print(
                    &serde_json::json!({"output":output,"frames":evidence.frames.len(),"model_sha256":evidence.model_sha256}),
                )?;
            }
            BenchmarkCommand::Inspect {
                midi,
                instrument,
                delay_ms,
                lyrics_output,
                output,
            } => {
                ensure!(
                    fs::metadata(&midi)?.len() <= 64 * 1024 * 1024,
                    "Benchmark MIDI exceeds 64 MiB"
                );
                let bytes = fs::read(&midi)?;
                let track = chart_engine::benchmark::parse(&bytes, instrument, delay_ms)?;
                let targets = serde_json::json!({"version":1,"notes_sha256":models::hash_file(&midi)?,"instrument":instrument,"delay_ms":delay_ms,"attacks_seconds":chart_engine::benchmark::attack_times(&track),"track":track});
                let lyrics = lyrics_output
                    .as_ref()
                    .map(|_| chart_engine::benchmark::lyric_source(&bytes))
                    .transpose()?;
                ensure!(
                    !output.exists()
                        && lyrics_output
                            .as_ref()
                            .is_none_or(|path| !path.exists() && path != &output),
                    "Benchmark outputs already exist or overlap"
                );
                write_new(&output, &serde_json::to_vec_pretty(&targets)?)?;
                if let Some((path, lyrics)) = lyrics_output.zip(lyrics) {
                    write_new(&path, &serde_json::to_vec_pretty(&lyrics)?)?;
                }
                print(
                    &serde_json::json!({"output":output,"status":"reference_targets_only_not_inference_timestamps"}),
                )?;
            }
            BenchmarkCommand::Compare {
                reference_midi,
                generated_midi,
                reference_audio,
                generated_audio,
                reference_delay_ms,
                generated_delay_ms,
                instrument,
                output,
            } => {
                let report = chart_engine::benchmark::compare(chart_engine::benchmark::Request {
                    reference_midi: &reference_midi,
                    generated_midi: &generated_midi,
                    reference_audio: &reference_audio,
                    generated_audio: &generated_audio,
                    reference_delay_ms,
                    generated_delay_ms,
                    instrument,
                })?;
                write_new(&output, &serde_json::to_vec_pretty(&report)?)?;
                print(&report)?;
            }
        },
        Commands::Generation { action } => match action {
            GenerationCommand::Create { request, output } => {
                let request = serde_json::from_slice(&fs::read(request)?)?;
                let root = chart_engine::generation::create(request, &output)?;
                print(
                    &serde_json::json!({"job": root, "status":chart_engine::generation::status(&root)?}),
                )?;
            }
            GenerationCommand::Run { job } => {
                chart_engine::generation::run(&job)?;
                print(&chart_engine::generation::status(&job)?)?;
            }
            GenerationCommand::Show { job } => print(&chart_engine::generation::status(&job)?)?,
        },
        Commands::Clean { action } => match action {
            CleanCommand::Detect {
                audio,
                policy,
                mode,
                model_dir,
                no_download,
                vocal_stem,
                alignment,
                output,
            } => {
                chart_engine::pipeline::destination(&output, None)?;
                let policy = match policy {
                    Some(path) => serde_json::from_slice(&fs::read(path)?)?,
                    None => chart_engine::clean::CleanPolicy::default(),
                };
                let mode = match mode {
                    CleanModeArg::FullMix => chart_engine::clean::CleanMode::FullMix,
                    CleanModeArg::VocalOnly => chart_engine::clean::CleanMode::VocalOnly,
                };
                let alignment: Option<chart_engine::clean::StemAlignment> = alignment
                    .map(|path| -> Result<_> { Ok(serde_json::from_slice(&fs::read(path)?)?) })
                    .transpose()?;
                let model_dir =
                    model_dir.unwrap_or_else(chart_engine::generation::default_model_dir);
                let detection = chart_engine::clean_recognition::detect(
                    chart_engine::clean_recognition::Request {
                        audio: &audio,
                        model_dir: &model_dir,
                        download_model: !no_download,
                        vocal_stem: vocal_stem.as_deref().zip(alignment.as_ref()),
                    },
                    &policy,
                    mode,
                    std::sync::Arc::new(chart_engine::clean_recognition::Control::default()),
                )?;
                chart_engine::clean_recognition::save(&output, &detection)?;
                print(
                    &serde_json::json!({"output":output,"recognized_words":detection.recognition.words.words.len(),"matches":detection.plan.matches.len(),"timed_ranges":detection.plan.ranges.len(),"untimed_words":detection.plan.untimed_words,"status":"automatic_candidate_needs_listening_review","warnings":detection.recognition.warnings}),
                )?;
            }
            CleanCommand::Plan {
                words,
                policy,
                mode,
                output,
            } => {
                let words = serde_json::from_slice(&fs::read(words)?)?;
                let policy = match policy {
                    Some(path) => serde_json::from_slice(&fs::read(path)?)?,
                    None => chart_engine::clean::CleanPolicy::default(),
                };
                let mode = match mode {
                    CleanModeArg::FullMix => chart_engine::clean::CleanMode::FullMix,
                    CleanModeArg::VocalOnly => chart_engine::clean::CleanMode::VocalOnly,
                };
                let plan = chart_engine::clean::plan(&words, &policy, mode)?;
                write_new(&output, &serde_json::to_vec_pretty(&plan)?)?;
                print(&plan)?;
            }
            CleanCommand::Render {
                audio,
                plan,
                vocal_stem,
                alignment,
                output,
            } => {
                let plan = serde_json::from_slice(&fs::read(plan)?)?;
                let alignment = alignment
                    .map(|p| -> Result<chart_engine::clean::StemAlignment> {
                        Ok(serde_json::from_slice(&fs::read(p)?)?)
                    })
                    .transpose()?;
                print(&chart_engine::clean::render(
                    chart_engine::clean::RenderRequest {
                        audio: &audio,
                        plan: &plan,
                        vocal_stem: vocal_stem.as_deref(),
                        stem_alignment: alignment.as_ref(),
                        output: &output,
                    },
                )?)?;
            }
            CleanCommand::BindStem {
                audio,
                stem,
                evidence,
                output,
            } => {
                ensure!(
                    !evidence.trim().is_empty(),
                    "Alignment evidence is required; hashes alone do not prove alignment"
                );
                let alignment = chart_engine::clean::StemAlignment {
                    audio_sha256: models::hash_file(&audio)?,
                    stem_sha256: models::hash_file(&stem)?,
                    evidence,
                };
                write_new(&output, &serde_json::to_vec_pretty(&alignment)?)?;
                print(&alignment)?;
            }
        },
        Commands::Editor { action } => match action {
            EditorCommand::Create {
                song,
                markers,
                output,
            } => {
                let store =
                    chart_engine::editor::Store::create(&song, markers.as_deref(), &output)?;
                print(&store.inspect()?)?;
            }
            EditorCommand::Show { project } => {
                print(&chart_engine::editor::Store::new(&project).inspect()?)?
            }
            EditorCommand::Apply { project, request } => {
                let request = serde_json::from_slice(&fs::read(request)?)?;
                print(&chart_engine::editor::Store::new(&project).apply(request)?)?;
            }
            EditorCommand::Open { project } => {
                #[cfg(feature = "studio")]
                chart_engine::studio::open(&project)?;
                #[cfg(not(feature = "studio"))]
                anyhow::bail!(
                    "Native editor unavailable for {}; build with --features editor",
                    project.display()
                );
            }
        },
        Commands::Vocals { action } => match action {
            VocalCommand::AttachReview {
                project,
                report,
                expected_revision,
            } => {
                ensure!(
                    fs::metadata(&report)?.len() <= 64 * 1024 * 1024,
                    "Vocal review report exceeds 64 MiB"
                );
                let report: chart_engine::vocal_generation::Generation =
                    serde_json::from_slice(&fs::read(report)?)?;
                let review = chart_engine::vocal_review::VocalReviewMap::from_generation(&report)?;
                print(&chart_engine::editor::Store::new(&project).apply(
                    chart_engine::editor::Request {
                        expected_revision,
                        actor: chart_engine::editor::Actor {
                            kind: chart_engine::editor::ActorKind::Ai,
                            name: "CLI vocal review import".into(),
                        },
                        action: chart_engine::editor::Action::SetVocalReview { review },
                    },
                )?)?;
            }
            VocalCommand::Validate { document } => {
                let chart = vocal_document(&document)?;
                print(
                    &serde_json::json!({"status":"structure_valid_needs_musical_review",
                    "version":chart.version,"resolution":chart.resolution,
                    "notes":chart.notes.len(),"phrases":chart.phrases.len()}),
                )?;
            }
            VocalCommand::GenerateProject {
                project,
                recognize,
                language,
                neural_pitch,
                lookup_lyrics,
                download_model,
                model_dir,
            } => {
                let model_dir =
                    model_dir.unwrap_or_else(chart_engine::generation::default_model_dir);
                print(&chart_engine::vocal_workflow::generate_project(
                    chart_engine::vocal_workflow::ProjectRequest {
                        project: &project,
                        model_dir: &model_dir,
                        recognize,
                        language,
                        neural_pitch,
                        lookup_lyrics,
                        download_model,
                        actor: chart_engine::editor::Actor {
                            kind: chart_engine::editor::ActorKind::Ai,
                            name: "CLI vocal generation".into(),
                        },
                    },
                    std::sync::Arc::new(chart_engine::clean_recognition::Control::default()),
                )?)?;
            }
            VocalCommand::Generate {
                audio,
                stem_manifest,
                chart,
                bpm,
                words,
                lyrics,
                lyric_source,
                recognize,
                language,
                neural_pitch,
                model_dir,
                download_model,
                output,
            } => {
                let timing = if let Some(path) = chart {
                    chart_engine::chart::Chart::parse(fs::read_to_string(path)?)?.timing
                } else {
                    TempoMap::constant(bpm.context("Tempo is required")?)?
                };
                let words = words
                    .map(|path| -> Result<chart_engine::clean::WordTimings> {
                        Ok(serde_json::from_slice(&fs::read(path)?)?)
                    })
                    .transpose()?;
                let plain_lyrics = lyrics
                    .map(
                        |path| -> Result<chart_engine::vocal_generation::LyricSource> {
                            Ok(chart_engine::vocal_generation::LyricSource {
                                text: fs::read_to_string(&path)?,
                                synced_lyrics: None,
                                source: path.display().to_string(),
                            })
                        },
                    )
                    .transpose()?;
                let lyrics = if let Some(path) = lyric_source {
                    Some(serde_json::from_slice::<
                        chart_engine::vocal_generation::LyricSource,
                    >(&fs::read(path)?)?)
                } else {
                    plain_lyrics
                };
                let models = model_dir.unwrap_or_else(chart_engine::generation::default_model_dir);
                let result = chart_engine::vocal_workflow::generate(
                    chart_engine::vocal_workflow::Request {
                        audio: &audio,
                        manifest: &stem_manifest,
                        timing: &timing,
                        words: words.as_ref(),
                        lyrics: lyrics.as_ref(),
                        recognize,
                        language,
                        neural_pitch,
                        model_dir: &models,
                        download_model,
                    },
                    &output,
                    std::sync::Arc::new(chart_engine::clean_recognition::Control::default()),
                )?;
                print(
                    &serde_json::json!({"output":output,"status":result.status,"notes":result.chart.as_ref().map_or(0,|chart|chart.notes.len()),"phrases":result.chart.as_ref().map_or(0,|chart|chart.phrases.len()),"review_issues":result.issues.len()}),
                )?;
            }
            VocalCommand::Import { midi, output } => {
                let chart = chart_engine::vocals::import_midi(&fs::read(midi)?)?;
                write_new(&output, &serde_json::to_vec_pretty(&chart)?)?;
                print(
                    &serde_json::json!({"status":"editable_draft","output":output,
                    "notes":chart.notes.len(),"phrases":chart.phrases.len()}),
                )?;
            }
            VocalCommand::Export {
                document,
                midi,
                bpm,
                output,
            } => {
                if output
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|name| {
                        name.eq_ignore_ascii_case("notes.mid")
                            || name.eq_ignore_ascii_case("notes.midi")
                    })
                {
                    let parent = output
                        .parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .unwrap_or_else(|| Path::new("."));
                    ensure!(
                        !fs::read_dir(parent)?
                            .collect::<std::io::Result<Vec<_>>>()?
                            .iter()
                            .any(|entry| entry
                                .file_name()
                                .to_string_lossy()
                                .eq_ignore_ascii_case("notes.chart")),
                        "Existing notes.chart is authoritative; do not add a competing notes.mid. Export to a separate working directory."
                    );
                }
                let chart = vocal_document(&document)?;
                let original = if let Some(path) = midi {
                    fs::read(path)?
                } else {
                    let map = TempoMap::new(
                        chart.resolution,
                        0.0,
                        vec![chart_engine::timing::Tempo {
                            tick: 0,
                            bpm: bpm.context("New MIDI requires BPM")?,
                        }],
                    )?;
                    chart_engine::midi::new_midi(&map, &chart_engine::chart::Tracks::new())?
                };
                let bytes = chart_engine::vocals::append_midi(&original, &chart)?;
                write_new(&output, &bytes)?;
                print(
                    &serde_json::json!({"status":"exported_draft_needs_game_and_musical_review",
                    "output":output,"notes":chart.notes.len(),"phrases":chart.phrases.len()}),
                )?;
            }
        },
        Commands::Refine {
            events,
            bass_audio,
            output,
        } => {
            let mut events: Transcription = serde_json::from_slice(&fs::read(events)?)?;
            events.validate()?;
            chart_engine::bass_analysis::analyze(&mut events, &bass_audio)?;
            write_new(&output, &serde_json::to_vec_pretty(&events)?)?;
        }
        Commands::Review {
            folder,
            drum_audio,
            bass_audio,
            output,
        } => print(&chart_engine::review::run(
            &folder,
            &drum_audio,
            bass_audio.as_deref(),
            &output,
        )?)?,
        Commands::Models { action } => match action {
            ModelCommand::Fetch {
                directory,
                high_quality_piano,
                multilingual_lyrics,
            } => {
                models::fetch(&directory)?;
                if multilingual_lyrics {
                    chart_engine::clean_recognition::prepare_model(
                        &directory,
                        true,
                        chart_engine::lyric_language::Language::Auto,
                        &Default::default(),
                    )?;
                }
                if high_quality_piano {
                    chart_engine::keyboard_separation::fetch(&directory)?;
                }
            }
            ModelCommand::Verify { directory } => {
                for a in models::assets() {
                    let path = models::verified(&directory, &a.id)?;
                    eprintln!("{}: {} [{}]", a.id, path.display(), a.license);
                }
                if chart_engine::keyboard_separation::resolved_model_path(&directory)?
                    .try_exists()?
                {
                    let path = chart_engine::keyboard_separation::verified(&directory)?;
                    eprintln!(
                        "{}: {} [{}]",
                        chart_engine::keyboard_separation::MODEL_ID,
                        path.display(),
                        chart_engine::keyboard_separation::WEIGHT_LICENSE_NOTICE
                    );
                }
            }
        },
        Commands::Inspect { folder } => print(&pipeline::inspect(&folder)?)?,
        Commands::Scan { folder } => print(&pipeline::scan(&folder)?)?,
        Commands::Separate {
            audio,
            output,
            model_dir,
            stem_cache,
        } => {
            pipeline::destination(&output, None)?;
            let default_cache = model_dir.join("stem-cache");
            let manifest = pipeline::cached_separation(
                &audio,
                &model_dir,
                stem_cache.as_deref().unwrap_or(&default_cache),
            )?;
            pipeline::export_isolation(&audio, &manifest, &output)?;
            print(
                &serde_json::json!({"output":output,"manifest":output.join(pipeline::STEM_MANIFEST),"generation":manifest.generation,"stems":["drums","bass","other","vocals","guitar","piano"],"musical_acceptance":"unverified"}),
            )?;
        }
        Commands::ImportStems {
            audio,
            stems,
            output,
            alignment_method,
            alignment_details,
        } => {
            let manifest = pipeline::import_isolation(
                &audio,
                &stems,
                &output,
                chart_engine::stem_review::AlignmentEvidence {
                    method: alignment_method,
                    details: alignment_details,
                },
            )?;
            print(
                &serde_json::json!({"manifest":output,"source_audio_sha256":manifest.source_audio_sha256,"alignment":manifest.alignment,"generation":manifest.generation,"musical_acceptance":"unverified"}),
            )?;
        }
        Commands::Generate {
            audio,
            bpm,
            options,
            inference,
        } => {
            pipeline::destination(&options.output, None)?;
            TempoMap::constant(bpm)?;
            let wanted = inference.requested();
            let transcription = pipeline::transcribe_with_isolation(
                &audio,
                &inference.model_dir,
                inference.isolation(),
                &wanted,
            )?;
            build(
                &audio,
                None,
                Some(TempoMap::constant(bpm)?),
                options,
                &transcription,
            )?;
        }
        Commands::Enhance {
            folder,
            audio,
            options,
            inference,
        } => {
            pipeline::destination(&options.output, Some(&folder))?;
            pipeline::validate_existing(&folder)?;
            let mut wanted = if inference.instruments.is_empty() {
                pipeline::inspect(&folder)?.missing
            } else {
                inference.requested()
            };
            if inference.drums_only {
                wanted.retain(|n| n == "drums");
            }
            ensure!(
                !wanted.is_empty(),
                "Requested instruments are already charted"
            );
            let audio = audio
                .map(Ok)
                .unwrap_or_else(|| pipeline::audio_file(&folder))?;
            let transcription = pipeline::transcribe_with_isolation(
                &audio,
                &inference.model_dir,
                inference.isolation(),
                &wanted,
            )?;
            build(&audio, Some(&folder), None, options, &transcription)?;
        }
        Commands::Transcribe {
            audio,
            output,
            inference,
        } => {
            ensure!(
                !output.exists(),
                "Event output exists; refusing to overwrite"
            );
            let wanted = inference.requested();
            let transcription = pipeline::transcribe_with_isolation(
                &audio,
                &inference.model_dir,
                inference.isolation(),
                &wanted,
            )?;
            let parent = output
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let mut stage = tempfile::NamedTempFile::new_in(parent)?;
            serde_json::to_writer_pretty(&mut stage, &transcription)?;
            stage.persist_noclobber(&output).map_err(|e| e.error)?;
            print(
                &serde_json::json!({"output":output,"drums":transcription.drums.len(),"guitar":transcription.guitar.len(),"bass":transcription.bass.len(),"piano":transcription.piano.len(),"pro_piano":transcription.pro_piano.len()}),
            )?;
        }
        Commands::Build {
            events,
            audio,
            existing,
            bpm,
            options,
        } => {
            pipeline::destination(&options.output, existing.as_deref())?;
            let bytes = fs::read(&events).with_context(|| format!("Read {}", events.display()))?;
            let performance = events.extension().is_some_and(|e| e == "mid");
            let transcription = if performance {
                chart_engine::midi::import(&bytes)?
            } else {
                serde_json::from_slice(&bytes)?
            };
            let timing = if let Some(bpm) = bpm {
                Some(TempoMap::constant(bpm)?)
            } else if performance {
                Some(chart_engine::midi::timing(&bytes)?)
            } else {
                None
            };
            build(&audio, existing.as_deref(), timing, options, &transcription)?;
        }
    }
    Ok(())
}
