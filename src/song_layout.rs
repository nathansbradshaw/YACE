//! Standard song files stay visible; YACE state and evidence live under `.yace`.
use std::path::{Path, PathBuf};

pub const DIRECTORY: &str = ".yace";

pub fn relative(name: &str) -> PathBuf {
    let category = match name {
        "chart-engine-review.json" | "editor-review.json" | "chart-engine-part-origins.json" => {
            "review"
        }
        "chart-engine-isolation.json"
        | "chart-engine-keyboard-isolation.json"
        | "chart-engine-melody-source.json" => "audio",
        "game-export.json" | "yace-audio.json" => "export",
        "chart-engine-working" => return Path::new(DIRECTORY).join("working"),
        "chart-engine-history" => return Path::new(DIRECTORY).join("history/generation"),
        "chart-engine-vocals.json" => "working",
        "import-information.json" => "analysis",
        value if value.starts_with("chart-engine-") && value.ends_with(".json") => "analysis",
        _ => return PathBuf::from(name),
    };
    Path::new(DIRECTORY).join(category).join(name)
}

/// Canonical destination for newly written song sidecars.
pub fn path(song: &Path, name: &str) -> PathBuf {
    song.join(relative(name))
}

/// Read new folders first, while retaining compatibility with earlier projects.
pub fn resolve(song: &Path, name: &str) -> PathBuf {
    let canonical = path(song, name);
    if crate::storage::exists(&canonical) {
        canonical
    } else {
        song.join(name)
    }
}

pub fn prepare(song: &Path) -> anyhow::Result<()> {
    let private = song.join(DIRECTORY);
    if !crate::storage::exists(&private) {
        crate::storage::create_dir(&private)?;
    }
    for category in [
        "analysis", "audio", "review", "export", "working", "history",
    ] {
        let path = private.join(category);
        if !crate::storage::exists(&path) {
            crate::storage::create_dir(path)?;
        }
    }
    Ok(())
}

/// Extra state for an existing editing project, including legacy standalone ones.
pub fn project_data(project: &Path) -> PathBuf {
    if project.file_name().is_some_and(|name| name == "project")
        && project
            .parent()
            .is_some_and(|parent| parent.file_name().is_some_and(|name| name == DIRECTORY))
    {
        project.parent().unwrap().to_owned()
    } else {
        project.join(DIRECTORY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standard_song_assets_keep_their_names_and_yace_files_are_grouped() {
        for name in [
            "notes.chart",
            "notes.mid",
            "song.ini",
            "song.ogg",
            "guitar.wav",
            "album.png",
            "video.mp4",
        ] {
            assert_eq!(relative(name), Path::new(name));
        }
        assert_eq!(
            relative("chart-engine-transcription.json"),
            Path::new(".yace/analysis/chart-engine-transcription.json")
        );
        assert_eq!(
            relative("chart-engine-isolation.json"),
            Path::new(".yace/audio/chart-engine-isolation.json")
        );
        assert_eq!(
            relative("game-export.json"),
            Path::new(".yace/export/game-export.json")
        );
    }
}
