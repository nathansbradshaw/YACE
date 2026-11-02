use anyhow::{Context, Result, ensure};
use chart_engine::stepmania::{
    self as sm, Difficulty, Format, GenerationRequest, Profile, Project,
    bundle::{self, Files},
};
use clap::Subcommand;
use std::{
    fs,
    path::{Path, PathBuf},
};
#[derive(Subcommand)]
pub enum Command {
    /// Snapshot one song folder as an editable StepMania project.
    Import {
        source: PathBuf,
        #[arg(long)]
        simfile: Option<String>,
        #[arg(long, default_value = "pad")]
        profile: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Start dance charts from a local recording.
    New {
        #[arg(long)]
        audio: PathBuf,
        #[arg(long)]
        bpm: f64,
        #[arg(long, default_value_t = 0.)]
        first_beat: f64,
        #[arg(long)]
        title: String,
        #[arg(long)]
        artist: String,
        #[arg(long, default_value = "pad")]
        profile: String,
        #[arg(long)]
        output: PathBuf,
    },
    Show {
        project: PathBuf,
    },
    /// Apply the same revision-checked action used by the editor.
    Apply {
        project: PathBuf,
        #[arg(long)]
        request: PathBuf,
    },
    Generate {
        project: PathBuf,
        #[arg(long, default_value_t = 1)]
        seed: u64,
        #[arg(long, default_value_t = 1.)]
        density: f64,
        #[arg(
            long,
            value_delimiter = ',',
            default_value = "Beginner,Easy,Medium,Hard,Challenge"
        )]
        difficulties: Vec<String>,
    },
    Export {
        project: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value = "ssc")]
        format: String,
    },
    Footwork {
        project: PathBuf,
        #[arg(long)]
        chart: String,
    },
}
fn profile(s: &str) -> Result<Profile> {
    match s {
        "pad" => Ok(Profile::Pad),
        "keyboard" => Ok(Profile::Keyboard),
        _ => anyhow::bail!("Profile must be pad or keyboard"),
    }
}
fn load(root: &Path) -> Result<(Project, Files)> {
    bundle::restore(chart_engine::stepmania_files::read(root)?)
}
pub fn run(command: Command) -> Result<()> {
    match command {
        Command::Import {
            source,
            simfile,
            profile: style,
            output,
        } => {
            let files = chart_engine::stepmania_files::read(&source)?;
            let choices = bundle::candidates(&files);
            let selected = simfile
                .or_else(|| {
                    source
                        .is_file()
                        .then(|| source.file_name().unwrap().to_string_lossy().into_owned())
                })
                .or_else(|| (choices.len() == 1).then(|| choices[0].clone()))
                .context("Choose --simfile when a folder contains several simfiles")?;
            let (project, assets) = bundle::import(files, &selected, profile(&style)?)?;
            chart_engine::stepmania_files::write_new(&output, &bundle::backup(&project, &assets)?)?;
        }
        Command::New {
            audio,
            bpm,
            first_beat,
            title,
            artist,
            profile: style,
            output,
        } => {
            ensure!(first_beat.is_finite(), "First beat must be finite");
            let name = audio
                .file_name()
                .context("Invalid recording name")?
                .to_string_lossy()
                .into_owned();
            let project = Project::blank(
                title,
                artist,
                name.clone(),
                bpm,
                -first_beat,
                profile(&style)?,
            )?;
            let assets = Files::from([(name, fs::read(&audio)?)]);
            chart_engine::stepmania_files::write_new(&output, &bundle::backup(&project, &assets)?)?;
        }
        Command::Show { project } => {
            let (project, _) = load(&project)?;
            println!("{}", serde_json::to_string_pretty(&project)?);
        }
        Command::Apply { project, request } => {
            ensure!(
                fs::metadata(&request)?.len() <= 1024 * 1024,
                "Request exceeds 1 MiB"
            );
            let (mut document, _) = load(&project)?;
            let revision = document.revision;
            document.apply(serde_json::from_slice(&fs::read(request)?)?)?;
            chart_engine::stepmania_files::save_document(&project, &document, Some(revision))?;
            println!("Revision {}", document.revision);
        }
        Command::Generate {
            project,
            seed,
            density,
            difficulties,
        } => {
            let (mut document, assets) = load(&project)?;
            let revision = document.revision;
            let (name, _) =
                bundle::check_music(&document, &assets)?.context("Project has no recording")?;
            let samples =
                chart_engine::models::decode(&project.join("source").join(name), 11025, 1)?;
            let request = GenerationRequest {
                profile: document.content.profile,
                difficulties: difficulties
                    .iter()
                    .map(|s| {
                        Difficulty::parse(s).with_context(|| format!("Unknown difficulty {s}"))
                    })
                    .collect::<Result<_>>()?,
                seed,
                density,
            };
            let ids =
                document.generate(revision, "cli".into(), &request, &samples, 11025, || false)?;
            chart_engine::stepmania_files::save_document(&project, &document, Some(revision))?;
            println!(
                "Added {} drafts at revision {}",
                ids.len(),
                document.revision
            );
        }
        Command::Export {
            project,
            output,
            format,
        } => {
            let (project, assets) = load(&project)?;
            let format = match format.as_str() {
                "sm" => Format::Sm,
                "ssc" => Format::Ssc,
                _ => anyhow::bail!("Format must be sm or ssc"),
            };
            chart_engine::stepmania_files::write_new(
                &output,
                &bundle::export(&project, &assets, format)?,
            )?;
        }
        Command::Footwork { project, chart } => {
            let (project, _) = load(&project)?;
            let chart = project
                .content
                .document
                .charts
                .iter()
                .find(|c| c.id == chart)
                .context("Chart is missing")?;
            let analysis = sm::project::footwork(
                chart,
                project.content.document.effective_timing(&chart.id)?,
                &project.content.foot_overrides,
            );
            println!("{}", serde_json::to_string_pretty(&analysis)?);
        }
    }

    Ok(())
}
