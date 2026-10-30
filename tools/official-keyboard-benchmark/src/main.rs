//! Local read-only audit of installed SNG/YARGSONG containers.
//! Container decoding follows the published YARG.Core IO format; no game files
//! are modified. MIDI is parsed independently with midly.
use anyhow::{Context, Result, ensure};
use midly::{MetaMessage, MidiMessage, Smf, TrackEventKind};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::Path,
};

struct Archive {
    file: File,
    base: u64,
    cipher: Option<[i32; 3]>,
    keys: Vec<u8>,
}
impl Archive {
    fn read(&mut self, pos: u64, len: usize) -> Result<Vec<u8>> {
        ensure!(len <= 64 * 1024 * 1024, "unexpected member/header size");
        ensure!(
            pos.checked_add(len as u64).context("offset overflow")? + self.base
                <= self.file.metadata()?.len(),
            "read exceeds container"
        );
        self.file.seek(SeekFrom::Start(pos + self.base))?;
        let mut b = vec![0; len];
        self.file.read_exact(&mut b)?;
        if let Some([a, c_b, c]) = self.cipher {
            for (i, v) in b.iter_mut().enumerate() {
                *v = ((*v as i32)
                    .wrapping_sub((pos as i32).wrapping_add(i as i32).wrapping_mul(c))
                    .wrapping_sub(c_b)
                    ^ a) as u8;
            }
        }
        Ok(b)
    }
    fn open(path: &Path) -> Result<Self> {
        let mut file = File::open(path)?;
        let mut h = [0u8; 24];
        file.read_exact(&mut h)?;
        let mut a = Self {
            file,
            base: 0,
            cipher: None,
            keys: Vec::new(),
        };
        if &h[..8] == b"YARGSONG" {
            let z = h[8] as i32 + 1679;
            let w = (z ^ 4) - z * 2;
            let n = 25 * w - 5;
            let mut x = (w + (z << 1)) ^ 4;
            let l = (n + 73)
                .wrapping_mul(n + 23)
                .wrapping_sub(n.wrapping_mul(n).wrapping_add(96 * n));
            x = (-l + n + x - w * 25 + 5) % 255;
            ensure!(x >= 0, "negative cipher stride");
            let mut v = [0i32; 3];
            for i in 0..24i32 {
                let j = i * x;
                v[0] += (h[9 + (j % 15) as usize] as i32 + i * 3298 + 88903) as u8 as i32;
                v[1] -= h[9 + ((j + 7001) % 15) as usize] as i32;
                v[2] += h[9 + (j % 15) as usize] as i32;
            }
            a.base = 24;
            a.cipher = Some(v);
        }
        // YARG.Core skips the inner signature for YARGSONG wrappers.
        if a.cipher.is_none() {
            ensure!(&a.read(0, 6)? == b"SNGPKG", "invalid SNG signature");
        }
        let version = u32::from_le_bytes(a.read(6, 4)?.try_into().unwrap());
        ensure!(version == 1, "unsupported SNG version {version}");
        a.keys = a.read(10, 16)?;
        Ok(a)
    }
    fn member(&mut self, pos: u64, len: usize) -> Result<Vec<u8>> {
        let mut b = self.read(pos, len)?;
        for (i, v) in b.iter_mut().enumerate() {
            *v ^= self.keys[i % 16] ^ i as u8;
        }
        Ok(b)
    }
}
fn u64_at(b: &[u8], p: &mut usize) -> Result<u64> {
    let end = p.checked_add(8).context("overflow")?;
    let x = u64::from_le_bytes(b.get(*p..end).context("truncated u64")?.try_into()?);
    *p = end;
    Ok(x)
}
fn string(b: &[u8], p: &mut usize) -> Result<String> {
    let e = *p + 4;
    let n = u32::from_le_bytes(b.get(*p..e).context("truncated length")?.try_into()?) as usize;
    *p = e;
    let e = p.checked_add(n).context("overflow")?;
    let s = String::from_utf8(b.get(*p..e).context("truncated string")?.to_vec())?;
    *p = e;
    Ok(s)
}
fn scan(path: &Path, extract: Option<&Path>) -> Result<Value> {
    let mut a = Archive::open(path)?;
    let meta_len = u64::from_le_bytes(a.read(26, 8)?.try_into().unwrap());
    let meta = a.read(34, usize::try_from(meta_len)?)?;
    let mut p = 0;
    let count = u64_at(&meta, &mut p)?;
    let mut metadata = BTreeMap::new();
    for _ in 0..count {
        let k = string(&meta, &mut p)?;
        let v = string(&meta, &mut p)?;
        metadata.insert(k, v);
    }
    ensure!(p == meta.len(), "metadata trailing bytes");
    let pos = 34 + meta_len;
    let len = u64::from_le_bytes(a.read(pos, 8)?.try_into().unwrap());
    let listing = a.read(pos + 8, usize::try_from(len)?)?;
    let mut p = 0;
    let count = u64_at(&listing, &mut p)?;
    let mut members = Vec::new();
    let mut tracks = Vec::new();
    let mut chart_hashes = BTreeMap::new();
    for _ in 0..count {
        let n = *listing.get(p).context("missing filename length")? as usize;
        p += 1;
        let name = String::from_utf8(listing.get(p..p + n).context("truncated name")?.to_vec())?;
        p += n;
        let size = u64_at(&listing, &mut p)?;
        let at = u64_at(&listing, &mut p)?;
        members.push(json!({"name":name,"bytes":size}));
        if let Some(dest) = extract {
            ensure!(
                Path::new(&name).file_name().and_then(|n| n.to_str()) == Some(&name),
                "unsafe member path"
            );
            if name.ends_with(".opus") || name.ends_with(".mid") || name.ends_with(".chart") {
                fs::write(dest.join(&name), a.member(at, usize::try_from(size)?)?)?;
            }
        }
        if name.ends_with(".mid") || name.ends_with(".chart") {
            let bytes = a.member(at, usize::try_from(size)?)?;
            chart_hashes.insert(name.clone(), format!("{:x}", Sha256::digest(&bytes)));
            if name.ends_with(".mid") {
                let smf = Smf::parse(&bytes).context("MIDI decode")?;
                for t in smf.tracks {
                    let mut name = String::new();
                    let mut notes = BTreeMap::<u8, usize>::new();
                    for e in t {
                        match e.kind {
                            TrackEventKind::Meta(MetaMessage::TrackName(n)) => {
                                name = String::from_utf8_lossy(n).into_owned()
                            }
                            TrackEventKind::Midi {
                                message: MidiMessage::NoteOn { key, vel },
                                ..
                            } if vel.as_int() > 0 => *notes.entry(key.as_int()).or_default() += 1,
                            _ => {}
                        }
                    }
                    let playable = if name.starts_with("PART REAL_KEYS_") {
                        notes
                            .iter()
                            .filter(|(k, _)| (48u8..=72).contains(*k))
                            .map(|(_, n)| n)
                            .sum::<usize>()
                    } else if name == "PART KEYS" {
                        notes
                            .iter()
                            .filter(|(k, _)| {
                                [60, 72, 84, 96].iter().any(|b| **k >= *b && **k <= *b + 4)
                            })
                            .map(|(_, n)| n)
                            .sum()
                    } else {
                        0
                    };
                    tracks.push(json!({"name":name,"note_on_counts":notes,"keyboard_playable_notes":playable}));
                }
            } else {
                let text = String::from_utf8(bytes)?;
                let mut name = String::new();
                let mut count = 0;
                for line in text.lines().chain(std::iter::once("[END]")) {
                    let l = line.trim();
                    if l.starts_with('[') && l.ends_with(']') {
                        if !name.is_empty() {
                            tracks.push(json!({"name":name,"note_events":count,"keyboard_playable_notes":if name.contains("Keyboard"){count}else{0}}));
                        }
                        name = l.trim_matches(['[', ']']).to_owned();
                        count = 0;
                    } else if l.split_whitespace().nth(2) == Some("N") {
                        count += 1;
                    }
                }
            }
        }
    }
    ensure!(p == listing.len(), "listing trailing bytes");
    ensure!(!chart_hashes.is_empty(), "no chart found");
    Ok(
        json!({"path":path,"metadata":metadata,"members":members,"chart_sha256":chart_hashes,"tracks":tracks}),
    )
}
fn main() -> Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.get(1).is_some_and(|s| s == "extract") {
        let archive = Path::new(args.get(2).context("archive")?);
        let dest = Path::new(args.get(3).context("output")?);
        ensure!(!dest.exists(), "extraction destination exists");
        fs::create_dir_all(dest)?;
        let inventory = scan(archive, Some(dest))?;
        fs::write(
            dest.join("archive-inventory.json"),
            serde_json::to_vec_pretty(&inventory)?,
        )?;
        return Ok(());
    }
    let dir = args
        .get(1)
        .context("usage: official-keys-audit SETLIST_DIR")?;
    let mut paths = fs::read_dir(dir)?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.retain(|p| p.extension().is_some_and(|e| e == "yargsong" || e == "sng"));
    paths.sort();
    let mut songs = Vec::new();
    let mut errors = Vec::new();
    for path in paths {
        match scan(&path, None) {
            Ok(s) => songs.push(s),
            Err(e) => errors.push(json!({"path":path,"error":format!("{e:#}")})),
        }
    }
    let keys = songs
        .iter()
        .filter(|s| {
            s["tracks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["keyboard_playable_notes"].as_u64().unwrap_or(0) > 0)
        })
        .count();
    let pro = songs
        .iter()
        .filter(|s| {
            s["tracks"].as_array().unwrap().iter().any(|t| {
                t["name"]
                    .as_str()
                    .unwrap_or("")
                    .starts_with("PART REAL_KEYS_")
                    && t["keyboard_playable_notes"].as_u64().unwrap_or(0) > 0
            })
        })
        .count();
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"songs_scanned":songs.len(),"songs_with_keyboard_notes":keys,"songs_with_pro_keys_notes":pro,"errors":errors,"songs":songs})
        )?
    );
    ensure!(errors.is_empty(), "some containers failed audit");
    Ok(())
}
