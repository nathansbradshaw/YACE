use crate::{
    Chart, Document, Format, Note, Timing,
    generation::{self, GenerationRequest, Profile},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Foot {
    Left,
    Right,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Content {
    pub document: Document,
    pub profile: Profile,
    pub foot_overrides: BTreeMap<String, Foot>,
    pub reviews: BTreeMap<String, Review>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Review {
    pub content_sha256: String,
    pub comment: String,
    pub actor: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub version: u32,
    pub revision: u64,
    pub source_sha256: String,
    #[serde(default)]
    pub assets_sha256: BTreeMap<String, String>,
    pub content: Content,
    pub next_id: u64,
    pub undo: Vec<Content>,
    pub redo: Vec<Content>,
    pub audit: Vec<Audit>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Audit {
    pub revision: u64,
    pub actor: String,
    pub action: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    EditNotes {
        chart_id: String,
        note_ids: Vec<String>,
        edit: SelectionEdit,
    },
    PasteNotes {
        chart_id: String,
        notes: Vec<Note>,
        row: i64,
    },
    SetSong {
        title: String,
        artist: String,
        timing: Timing,
    },
    AddNote {
        chart_id: String,
        row: i64,
        lane: u8,
        kind: crate::NoteKind,
        end_row: Option<i64>,
    },
    UpdateNote {
        chart_id: String,
        note: Note,
    },
    DeleteNote {
        chart_id: String,
        note_id: String,
    },
    SetTiming {
        chart_id: Option<String>,
        timing: Option<Timing>,
    },
    SetMetadata {
        chart_id: Option<String>,
        key: String,
        value: String,
    },
    SetChart {
        chart_id: String,
        difficulty: String,
        meter: u32,
        description: String,
    },
    SetFoot {
        note_id: String,
        foot: Option<Foot>,
    },
    SetProfile {
        profile: Profile,
    },
    Review {
        chart_id: String,
        comment: String,
    },
    AddChart {
        difficulty: crate::Difficulty,
        meter: u32,
        description: String,
    },
    DeleteChart {
        chart_id: String,
    },
    Undo,
    Redo,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SelectionEdit {
    Delete,
    Move { rows: i64 },
    MoveLanes { rows: i64, lanes: i8 },
    Mirror,
    Quantize { rows: i64 },
    QuantizeDivisions { divisions: u32 },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub expected_revision: u64,
    pub actor: String,
    pub action: Action,
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
impl Project {
    pub fn import(raw: String, format: Format, profile: Profile) -> Result<Self> {
        let source_sha256 = hash(raw.as_bytes());
        let document = Document::parse(raw, format)?;
        Ok(Self {
            version: 2,
            revision: 0,
            source_sha256,
            assets_sha256: BTreeMap::new(),
            content: Content {
                document,
                profile,
                foot_overrides: BTreeMap::new(),
                reviews: BTreeMap::new(),
            },
            next_id: 1,
            undo: Vec::new(),
            redo: Vec::new(),
            audit: Vec::new(),
        })
    }
    pub fn blank(
        title: String,
        artist: String,
        music: String,
        bpm: f64,
        offset: f64,
        profile: Profile,
    ) -> Result<Self> {
        let raw = format!(
            "#VERSION:0.83;\n#TITLE:{};\n#ARTIST:{};\n#MUSIC:{};\n#OFFSET:{offset};\n#BPMS:0={bpm};\n",
            crate::codec::escape(&title),
            crate::codec::escape(&artist),
            crate::codec::escape(&music)
        );
        Self::import(raw, Format::Ssc, profile)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 2, "Unsupported StepMania project version");
        ensure!(
            self.source_sha256 == hash(self.content.document.raw.as_bytes()),
            "Source snapshot hash differs"
        );
        ensure!(
            self.audit.len() <= 100_000 && self.undo.len() <= 100 && self.redo.len() <= 100,
            "Project history exceeds limits"
        );
        self.content.document.validate()?;
        for content in self.undo.iter().chain(&self.redo) {
            content.document.validate()?;
            ensure!(
                hash(content.document.raw.as_bytes()) == self.source_sha256,
                "History source differs"
            );
        }
        for id in self.content.foot_overrides.keys() {
            ensure!(
                self.content
                    .document
                    .charts
                    .iter()
                    .any(|c| c.notes.iter().any(|n| &n.id == id)),
                "Foot override refers to a missing note"
            );
        }
        Ok(())
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= 128 * 1024 * 1024, "Project exceeds 128 MiB");
        let project: Self = serde_json::from_slice(bytes)?;
        project.validate()?;
        Ok(project)
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)?;
        ensure!(
            bytes.len() <= 128 * 1024 * 1024,
            "Project exceeds 128 MiB; export or split this chart before adding more history"
        );
        Ok(bytes)
    }
    pub fn chart_hash(&self, id: &str) -> Result<String> {
        let c = self
            .content
            .document
            .charts
            .iter()
            .find(|c| c.id == id)
            .context("Chart missing")?;
        Ok(hash(&serde_json::to_vec(&(
            c,
            self.content.document.effective_timing(id)?,
            c.profile.unwrap_or(self.content.profile),
            c.notes
                .iter()
                .filter_map(|n| {
                    self.content
                        .foot_overrides
                        .get(&n.id)
                        .map(|foot| (&n.id, foot))
                })
                .collect::<BTreeMap<_, _>>(),
        ))?))
    }
    pub fn reviewed(&self, id: &str) -> bool {
        self.chart_hash(id).ok().is_some_and(|hash| {
            self.content
                .reviews
                .get(id)
                .is_some_and(|r| r.content_sha256 == hash)
        })
    }
    fn chart_mut(&mut self, id: &str) -> Result<&mut Chart> {
        let c = self
            .content
            .document
            .charts
            .iter_mut()
            .find(|c| c.id == id)
            .context("Chart missing")?;
        ensure!(
            c.read_only_reason.is_none(),
            "Unsupported chart is preserved read-only"
        );
        Ok(c)
    }
    fn checkpoint(&mut self, old: Content, actor: String, action: String) -> Result<()> {
        self.content.document.validate()?;
        self.undo.push(old);
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.revision = self.revision.checked_add(1).context("Revision overflow")?;
        self.audit.push(Audit {
            revision: self.revision,
            actor,
            action,
        });
        Ok(())
    }
    pub fn apply(&mut self, request: Request) -> Result<()> {
        ensure!(
            request.expected_revision == self.revision,
            "Stale project revision"
        );
        ensure!(
            !request.actor.trim().is_empty() && request.actor.len() <= 256,
            "Actor is required"
        );
        let mut candidate = self.clone();
        let old = candidate.content.clone();
        let action_name = serde_json::to_value(&request.action)?["type"]
            .as_str()
            .unwrap_or("edit")
            .to_owned();
        match request.action {
            Action::EditNotes {
                chart_id,
                note_ids,
                edit,
            } => {
                let ids: std::collections::BTreeSet<_> = note_ids.into_iter().collect();
                ensure!(!ids.is_empty(), "Select notes first");
                let chart = candidate.chart_mut(&chart_id)?;
                ensure!(
                    ids.iter().all(|id| chart.notes.iter().any(|n| &n.id == id)),
                    "Selection contains a missing note"
                );
                if matches!(edit, SelectionEdit::Delete) {
                    chart.notes.retain(|n| !ids.contains(&n.id));
                    candidate
                        .content
                        .foot_overrides
                        .retain(|id, _| !ids.contains(id));
                } else {
                    for n in chart.notes.iter_mut().filter(|n| ids.contains(&n.id)) {
                        match edit {
                            SelectionEdit::Move { rows }
                            | SelectionEdit::MoveLanes { rows, .. } => {
                                n.row = n.row.checked_add(rows).context("Row overflow")?;
                                n.end_row = n
                                    .end_row
                                    .map(|end| end.checked_add(rows).context("Tail overflow"))
                                    .transpose()?;
                                if let SelectionEdit::MoveLanes { lanes, .. } = edit {
                                    let lane = i16::from(n.lane) + i16::from(lanes);
                                    ensure!(
                                        (0..4).contains(&lane),
                                        "Selection would leave the four lanes"
                                    );
                                    n.lane = lane as u8;
                                }
                            }
                            SelectionEdit::Mirror => n.lane = 3 - n.lane,
                            SelectionEdit::Quantize { rows } => {
                                ensure!((1..=192).contains(&rows), "Invalid grid");
                                n.row = ((n.row as f64 / rows as f64).round() as i64) * rows;
                                n.end_row = n
                                    .end_row
                                    .map(|end| ((end as f64 / rows as f64).round() as i64) * rows);
                            }
                            SelectionEdit::QuantizeDivisions { divisions } => {
                                ensure!(
                                    (1..=1024).contains(&divisions),
                                    "Use 1–1024 steps per beat"
                                );
                                let snap = |row: i64| -> Result<i64> {
                                    ensure!(row >= 0, "Negative note row");
                                    let index = (row as u128 * divisions as u128 + 24) / 48;
                                    Ok(i64::try_from(
                                        (index * 48 + divisions as u128 / 2) / divisions as u128,
                                    )?)
                                };
                                n.row = snap(n.row)?;
                                n.end_row = n.end_row.map(snap).transpose()?;
                            }
                            SelectionEdit::Delete => unreachable!(),
                        }
                    }
                }
            }
            Action::PasteNotes {
                chart_id,
                mut notes,
                row,
            } => {
                ensure!(
                    !notes.is_empty() && notes.len() <= 100_000,
                    "Clipboard has no notes or exceeds limits"
                );
                ensure!(
                    (0..=crate::MAX_ROW).contains(&row),
                    "Paste position outside chart"
                );
                let first = notes.iter().map(|n| n.row).min().unwrap();
                for n in &mut notes {
                    n.row = n
                        .row
                        .checked_sub(first)
                        .and_then(|r| r.checked_add(row))
                        .context("Paste row overflow")?;
                    n.end_row = n
                        .end_row
                        .map(|end| {
                            end.checked_sub(first)
                                .and_then(|r| r.checked_add(row))
                                .context("Paste tail overflow")
                        })
                        .transpose()?;
                    n.id = format!("paste-{}", candidate.next_id);
                    candidate.next_id += 1;
                }
                candidate.chart_mut(&chart_id)?.notes.extend(notes);
            }
            Action::SetSong {
                title,
                artist,
                timing,
            } => {
                candidate
                    .content
                    .document
                    .metadata
                    .insert("TITLE".into(), title);
                candidate
                    .content
                    .document
                    .metadata
                    .insert("ARTIST".into(), artist);
                candidate.content.document.timing = timing;
            }
            Action::Undo => {
                let prior = candidate.undo.pop().context("Nothing to undo")?;
                candidate.redo.push(old);
                candidate.content = prior;
                candidate.revision += 1;
                candidate.audit.push(Audit {
                    revision: candidate.revision,
                    actor: request.actor,
                    action: action_name,
                });
                candidate.encode()?;
                *self = candidate;
                return Ok(());
            }
            Action::Redo => {
                let next = candidate.redo.pop().context("Nothing to redo")?;
                candidate.undo.push(old);
                candidate.content = next;
                candidate.revision += 1;
                candidate.audit.push(Audit {
                    revision: candidate.revision,
                    actor: request.actor,
                    action: action_name,
                });
                candidate.encode()?;
                *self = candidate;
                return Ok(());
            }
            Action::AddNote {
                chart_id,
                row,
                lane,
                kind,
                end_row,
            } => {
                let id = format!("edit-{}", candidate.next_id);
                candidate.next_id += 1;
                candidate.chart_mut(&chart_id)?.notes.push(Note {
                    id,
                    row,
                    lane,
                    kind,
                    end_row,
                });
            }
            Action::UpdateNote { chart_id, note } => {
                let chart = candidate.chart_mut(&chart_id)?;
                let old = chart
                    .notes
                    .iter_mut()
                    .find(|n| n.id == note.id)
                    .context("Note missing")?;
                *old = note;
            }
            Action::DeleteNote { chart_id, note_id } => {
                let chart = candidate.chart_mut(&chart_id)?;
                ensure!(chart.notes.iter().any(|n| n.id == note_id), "Note missing");
                chart.notes.retain(|n| n.id != note_id);
                candidate.content.foot_overrides.remove(&note_id);
            }
            Action::SetTiming { chart_id, timing } => {
                if let Some(id) = chart_id {
                    candidate.chart_mut(&id)?.timing = timing;
                } else {
                    candidate.content.document.timing =
                        timing.context("Song timing cannot inherit")?;
                }
            }
            Action::SetMetadata {
                chart_id,
                key,
                value,
            } => {
                ensure!(
                    !key.is_empty()
                        && key
                            .bytes()
                            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()),
                    "Invalid metadata key"
                );
                ensure!(
                    TimingKindCheck::metadata(&key),
                    "Use the typed timing, note or chart command"
                );
                let values = if let Some(id) = chart_id {
                    &mut candidate.chart_mut(&id)?.metadata
                } else {
                    &mut candidate.content.document.metadata
                };
                values.insert(key, value);
            }
            Action::SetChart {
                chart_id,
                difficulty,
                meter,
                description,
            } => {
                let c = candidate.chart_mut(&chart_id)?;
                c.difficulty = difficulty;
                c.meter = meter;
                c.description = description;
            }
            Action::SetFoot { note_id, foot } => {
                ensure!(
                    candidate
                        .content
                        .document
                        .charts
                        .iter()
                        .any(|c| c.notes.iter().any(|n| n.id == note_id)),
                    "Note missing"
                );
                if let Some(foot) = foot {
                    candidate.content.foot_overrides.insert(note_id, foot);
                } else {
                    candidate.content.foot_overrides.remove(&note_id);
                }
            }
            Action::SetProfile { profile } => {
                let previous = candidate.content.profile;
                for chart in &mut candidate.content.document.charts {
                    chart.profile.get_or_insert(previous);
                }
                candidate.content.profile = profile;
            }
            Action::Review { chart_id, comment } => {
                let content_sha256 = candidate.chart_hash(&chart_id)?;
                candidate.content.reviews.insert(
                    chart_id,
                    Review {
                        content_sha256,
                        comment,
                        actor: request.actor.clone(),
                    },
                );
            }
            Action::AddChart {
                difficulty,
                meter,
                description,
            } => {
                let id = format!("added-{}", candidate.next_id);
                candidate.next_id += 1;
                candidate.content.document.charts.push(Chart {
                    profile: Some(candidate.content.profile),
                    id,
                    stepstype: "dance-single".into(),
                    difficulty: difficulty.name().into(),
                    meter,
                    description,
                    notes: Vec::new(),
                    timing: None,
                    metadata: BTreeMap::new(),
                    read_only_reason: None,
                    original_notes: String::new(),
                });
            }
            Action::DeleteChart { chart_id } => {
                let _ = candidate.chart_mut(&chart_id)?;
                let notes: Vec<_> = candidate
                    .chart_mut(&chart_id)?
                    .notes
                    .iter()
                    .map(|n| n.id.clone())
                    .collect();
                candidate
                    .content
                    .document
                    .charts
                    .retain(|c| c.id != chart_id);
                candidate
                    .content
                    .foot_overrides
                    .retain(|id, _| !notes.contains(id));
                candidate.content.reviews.remove(&chart_id);
            }
        }
        candidate.checkpoint(old, request.actor, action_name)?;
        candidate.encode()?;
        *self = candidate;
        Ok(())
    }
    /// The callback is polled during analysis and arrangement. Cancellation
    /// returns before publishing charts or adding an undo checkpoint.
    pub fn generate(
        &mut self,
        expected_revision: u64,
        actor: String,
        request: &GenerationRequest,
        samples: &[f32],
        rate: u32,
        cancel: impl Fn() -> bool,
    ) -> Result<Vec<String>> {
        ensure!(!actor.trim().is_empty(), "Actor is required");
        ensure!(
            self.revision == expected_revision,
            "Stale generation revision"
        );
        let mut charts = generation::generate(
            samples,
            rate,
            &self.content.document.timing,
            request,
            cancel,
        )?;
        let mut candidate = self.clone();
        let old = candidate.content.clone();
        let mut ids = Vec::new();
        for chart in &mut charts {
            chart.id = format!("generated-{}", candidate.next_id);
            candidate.next_id += 1;
            for (i, n) in chart.notes.iter_mut().enumerate() {
                n.id = format!("{}-n{i}", chart.id);
            }
            ids.push(chart.id.clone());
        }
        candidate.content.document.charts.extend(charts);
        candidate.checkpoint(old, actor, "generate".into())?;
        candidate.encode()?;
        *self = candidate;
        Ok(ids)
    }
}
struct TimingKindCheck;
impl TimingKindCheck {
    fn metadata(key: &str) -> bool {
        crate::TimingKind::parse(key).is_none()
            && !matches!(
                key,
                "OFFSET"
                    | "NOTES"
                    | "NOTEDATA"
                    | "STEPSTYPE"
                    | "DIFFICULTY"
                    | "METER"
                    | "DESCRIPTION"
            )
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Footwork {
    pub assignments: BTreeMap<String, Foot>,
    pub warnings: Vec<Warning>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Warning {
    pub row: i64,
    pub message: String,
}
/// A bounded two-foot solver. It honors holds and overrides; predictions and
/// difficulty warnings are advice, never evidence of pad acceptance.
pub fn footwork(chart: &Chart, timing: &Timing, overrides: &BTreeMap<String, Foot>) -> Footwork {
    let mut result = Footwork {
        assignments: BTreeMap::new(),
        warnings: Vec::new(),
    };
    let mut rows: BTreeMap<i64, Vec<&Note>> = BTreeMap::new();
    for n in &chart.notes {
        if !matches!(n.kind, crate::NoteKind::Mine | crate::NoteKind::Fake)
            && timing.judgable(n.row as f64 / 48.)
        {
            rows.entry(n.row).or_default().push(n);
        }
    }
    let mut positions = [0u8, 3u8];
    let mut held = [-1i64; 2];
    let mut last = [-1000i64; 2];
    let mut previous = None;
    for (row, notes) in rows {
        let mut best = None;
        for mask in 0..(1usize << notes.len().min(4)) {
            let mut used = [false; 2];
            let mut cost = 0.;
            let mut assignments = Vec::new();
            for (i, n) in notes.iter().enumerate() {
                let foot = (mask >> i) & 1;
                let label = if foot == 0 { Foot::Left } else { Foot::Right };
                if used[foot]
                    || held[foot] > row && positions[foot] != n.lane
                    || overrides.get(&n.id).is_some_and(|f| *f != label)
                {
                    cost = f64::INFINITY;
                    break;
                }
                used[foot] = true;
                let delta = timing.seconds(row as f64 / 48., crate::timing::Boundary::Note)
                    - timing.seconds(last[foot] as f64 / 48., crate::timing::Boundary::Note);
                cost += if positions[foot] != n.lane {
                    1. / delta.max(0.05)
                } else {
                    0.2
                };
                if previous == Some(foot) {
                    cost += 2.;
                }
                if foot == 0 && n.lane == 3 || foot == 1 && n.lane == 0 {
                    cost += 5.;
                }
                assignments.push(foot);
            }
            if best.as_ref().is_none_or(|(c, _)| cost < *c) {
                best = Some((cost, assignments));
            }
        }
        let Some((cost, feet)) = best.filter(|(c, _)| c.is_finite()) else {
            result.warnings.push(Warning{row,message:"Needs more than two feet, conflicts with a hold, or cannot honor foot overrides".into()});
            continue;
        };
        if cost > 15. {
            result.warnings.push(Warning {
                row,
                message: "Abrupt foot transition; listen and playtest this passage".into(),
            });
        }
        for (n, foot) in notes.iter().zip(feet) {
            result.assignments.insert(
                n.id.clone(),
                if foot == 0 { Foot::Left } else { Foot::Right },
            );
            if positions[foot] == 1 && n.lane == 2 || positions[foot] == 2 && n.lane == 1 {
                result.warnings.push(Warning {
                    row,
                    message: "Vertical foot movement across the center (candle)".into(),
                });
            }
            positions[foot] = n.lane;
            held[foot] = n.end_row.unwrap_or(row);
            last[foot] = row;
            previous = Some(foot);
        }
    }
    result
}
