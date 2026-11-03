//! Sample-aligned, bounded listening comparisons; never part of project history.
use crate::{
    editor::{Project, Store, TimeRange},
    timing::TempoMap,
    vocals::{VocalChart, VocalTone},
};
use anyhow::{Context, Result, ensure};
pub const RATE: u32 = 22050;
pub fn binding(project: &Project, timing: &TempoMap, range: TimeRange) -> Result<String> {
    use sha2::{Digest, Sha256};
    let chart = project.content.vocals.as_ref().context("No vocal notes")?;
    let stem = project
        .content
        .stems
        .as_ref()
        .and_then(|m| {
            m.stems
                .iter()
                .find(|s| s.id == crate::stem_review::StemId::Vocals)
        })
        .context("No vocal stem")?;
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(
            &project.audio_sha256,
            &stem.sha256,
            chart,
            timing,
            range
        ))?)
    ))
}
pub fn prepare(
    project: &Project,
    store: &Store,
    chart: &VocalChart,
    range: TimeRange,
) -> Result<String> {
    use base64::Engine;
    ensure!(
        range.start.is_finite()
            && range.end.is_finite()
            && range.start >= 0.
            && range.start < range.end
            && range.end <= project.duration
            && range.end - range.start <= 30.,
        "Choose a comparison interval of at most 30 seconds"
    );
    let timing = project.timing(store)?;
    ensure!(
        crate::models::hash_file(&store.audio_path(project)?)? == project.audio_sha256,
        "Recording changed before audition"
    );
    let manifest = project.content.stems.as_ref().context("No vocal stem")?;
    manifest.validate_binding(&project.audio_sha256, project.duration)?;
    let stem = manifest
        .stems
        .iter()
        .find(|s| s.id == crate::stem_review::StemId::Vocals)
        .context("No vocal stem")?;
    ensure!(
        crate::models::hash_file(&stem.path)? == stem.sha256,
        "Vocal stem changed before audition"
    );
    let pcm = crate::models::decode(&stem.path, RATE, 1)?;
    ensure!(
        (pcm.len() as f64 / RATE as f64 - project.duration).abs() <= 2. / RATE as f64,
        "Vocal audition source clock differs"
    );
    let bytes = wave(&pcm, chart, &timing, range)?;
    ensure!(
        crate::models::hash_file(&store.audio_path(project)?)? == project.audio_sha256,
        "Recording changed during audition"
    );
    ensure!(
        crate::models::hash_file(&stem.path)? == stem.sha256,
        "Vocal stem changed during audition"
    );
    Ok(format!(
        "data:audio/wav;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}
pub fn wave(
    pcm: &[f32],
    chart: &VocalChart,
    timing: &TempoMap,
    range: TimeRange,
) -> Result<Vec<u8>> {
    chart.validate()?;
    ensure!(
        chart.resolution == timing.resolution
            && range.start.is_finite()
            && range.end.is_finite()
            && range.start >= 0.
            && range.start < range.end
            && range.end - range.start <= 30.,
        "Invalid comparison clock or range"
    );
    let start = (range.start * RATE as f64).round() as usize;
    let end = (range.end * RATE as f64).round() as usize;
    ensure!(
        start < end && end <= pcm.len() && pcm[start..end].iter().all(|s| s.is_finite()),
        "Invalid comparison samples"
    );
    let data = ((end - start) * 4) as u32;
    let mut bytes = Vec::with_capacity(data as usize + 44);
    bytes.extend(b"RIFF");
    bytes.extend((36 + data).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16_u32.to_le_bytes());
    bytes.extend(1_u16.to_le_bytes());
    bytes.extend(2_u16.to_le_bytes());
    bytes.extend(RATE.to_le_bytes());
    bytes.extend((RATE * 4).to_le_bytes());
    bytes.extend(4_u16.to_le_bytes());
    bytes.extend(16_u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(data.to_le_bytes());
    let notes: Vec<_> = chart
        .notes
        .iter()
        .filter_map(|n| match n.tone {
            VocalTone::Pitched { midi } => {
                Some((timing.seconds(n.start), timing.seconds(n.end), midi))
            }
            _ => None,
        })
        .collect();
    let mut cursor = notes.partition_point(|(_, end, _)| *end <= range.start);
    for (i, sample) in pcm[start..end].iter().enumerate() {
        let t = (start + i) as f64 / RATE as f64;
        while cursor < notes.len() && notes[cursor].1 <= t {
            cursor += 1;
        }
        let tone = notes
            .get(cursor)
            .filter(|(a, b, _)| *a <= t && t < *b)
            .map_or(0., |(a, b, p)| {
                let fade = ((t - a) / 0.008).min((b - t) / 0.008).clamp(0., 1.);
                0.16 * fade
                    * (std::f64::consts::TAU * 440. * 2_f64.powf((*p as f64 - 69.) / 12.) * (t - a))
                        .sin()
            });
        bytes.extend(((sample.clamp(-1., 1.) * 32767.) as i16).to_le_bytes());
        bytes.extend(((tone * 32767.) as i16).to_le_bytes());
    }
    Ok(bytes)
}
