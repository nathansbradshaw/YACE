//! Read tensor data without executing checkpoint objects or upstream scripts.
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
mod checkpoint;
mod infer;
mod network;
mod onnx;
#[path = "../../../examples/support/onnx_proto.rs"]
mod proto;
#[path = "../../../src/vocal_note_recipe.rs"]
mod recipe;
#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    RawOnnx {
        path: PathBuf,
        name: String,
        output: PathBuf,
    },
    InspectOnnx {
        path: PathBuf,
    },
    Infer {
        stem: PathBuf,
        report: PathBuf,
        model: PathBuf,
        word_model: PathBuf,
        output: PathBuf,
        #[arg(long)]
        seconds: Option<f64>,
    },
    Export {
        checkpoint: PathBuf,
        output: PathBuf,
        #[arg(long)]
        rwbd: bool,
    },
    Inspect {
        checkpoint: PathBuf,
        #[arg(long)]
        key: Option<String>,
    },
}
fn main() -> Result<()> {
    match Args::parse().command {
        Command::RawOnnx { path, name, output } => {
            anyhow::ensure!(!output.exists(), "Output exists");
            let bytes = std::fs::read(path)?;
            std::fs::write(output, proto::output(&bytes, &name)?)?;
        }
        Command::InspectOnnx { path } => {
            let bytes = std::fs::read(&path)?;
            for g in proto::fields(&bytes)?.into_iter().filter(|f| f.id == 7) {
                for n in proto::fields(g.data)?.into_iter().filter(|n| n.id == 1) {
                    let f = proto::fields(n.data)?;
                    if f.iter().any(|f| {
                        f.id == 4
                            && matches!(f.data, b"Sigmoid" | b"Div" | b"ArgMax" | b"ReduceSum")
                    }) {
                        println!(
                            "{:?}",
                            f.iter()
                                .filter(|f| matches!(f.id, 1..=4))
                                .map(|f| (f.id, String::from_utf8_lossy(f.data)))
                                .collect::<Vec<_>>()
                        )
                    }
                }
            }
            let s = ort::session::Session::builder()?
                .with_intra_threads(2)?
                .commit_from_file(path)?;
            println!("inputs={:?}\noutputs={:?}", s.inputs, s.outputs);
        }
        Command::Infer {
            stem,
            report,
            model,
            word_model,
            output,
            seconds,
        } => infer::run(&stem, &report, &model, &word_model, &output, seconds)?,
        Command::Export {
            checkpoint,
            output,
            rwbd,
        } => {
            anyhow::ensure!(!output.exists(), "Use a new output directory");
            let ck = checkpoint::Checkpoint::read(&checkpoint, Some("state_dict"))?;
            anyhow::ensure!(
                (ck.sha256 == checkpoint::RWBD_SHA) == rwbd,
                "Checkpoint kind differs from requested graph"
            );
            let mut g = onnx::Graph::new(&ck);
            network::encoder(&mut g, !rwbd)?;
            std::fs::create_dir_all(&output)?;
            std::fs::write(output.join("encoder.onnx"), g.finish())?;
            if !rwbd {
                let mut g = onnx::Graph::new(&ck);
                network::decoder(&mut g)?;
                std::fs::write(output.join("decoder.onnx"), g.finish())?;
            }
        }
        Command::Inspect { checkpoint, key } => {
            let tensors = checkpoint::Checkpoint::read(&checkpoint, key.as_deref())?;
            let mut names: Vec<_> = tensors.tensors.iter().collect();
            names.sort_by_key(|(name, _)| *name);
            for (name, info) in names {
                println!("{name}: {:?} {:?}", info.layout.dims(), info.dtype);
            }
        }
    }
    Ok(())
}
