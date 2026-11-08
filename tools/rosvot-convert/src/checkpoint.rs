//! Restricted legacy torch.save tensor reader; metadata is never executed.
use anyhow::{Context, Result, bail, ensure};
use candle_core::{
    DType,
    pickle::{Object, Stack, TensorInfo},
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Cursor, Read, Seek, SeekFrom},
    path::Path,
};
pub const ROSVOT_SHA: &str = "7501fb5f913d971c2f51bcb3063b930027b03206581820a4d2bfdc394c9c3fcb";
pub const RWBD_SHA: &str = "0bc2d42a6d4b7a05436deb937e2deda1c12de49e5687cfda0bdf6a430120dcd2";
const MAGIC: &[u8] = &[
    0x80, 2, 0x8a, 10, 0x6c, 0xfc, 0x9c, 0x46, 0xf9, 0x20, 0x6a, 0xa8, 0x50, 0x19, 0x2e,
];
fn object(reader: &mut Cursor<&[u8]>) -> Result<Object> {
    let mut stack = Stack::empty();
    stack.read_loop(reader)?;
    Ok(stack.finalize()?)
}
fn collect(object: Object, prefix: &str, out: &mut BTreeMap<String, TensorInfo>) -> Result<()> {
    if let Some(info) = object
        .clone()
        .into_tensor_info(Object::Unicode(prefix.into()), Path::new(""))?
    {
        ensure!(
            out.insert(prefix.into(), info).is_none(),
            "Duplicate tensor"
        );
        return Ok(());
    }
    match object {
        Object::Build { callable, args } => {
            collect(*callable, prefix, out)?;
            collect(*args, &format!("{prefix}._metadata"), out)?;
        }
        Object::Dict(values) => {
            for (key, value) in values {
                let key = match key {
                    Object::Unicode(s) => Some(s),
                    Object::Int(n) => Some(n.to_string()),
                    Object::Long(n) => Some(n.to_string()),
                    _ => None,
                };
                if let Some(key) = key {
                    let path = if prefix.is_empty() {
                        key
                    } else {
                        format!("{prefix}.{key}")
                    };
                    collect(value, &path, out)?;
                }
            }
        }
        Object::Tuple(values) | Object::List(values) => {
            for (i, value) in values.into_iter().enumerate() {
                collect(value, &format!("{prefix}.{i}"), out)?;
            }
        }
        _ => {}
    }
    Ok(())
}
pub struct Checkpoint {
    bytes: Vec<u8>,
    pub sha256: String,
    pub tensors: BTreeMap<String, TensorInfo>,
    offsets: BTreeMap<String, u64>,
}
pub struct Data {
    pub dims: Vec<i64>,
    pub raw: Vec<u8>,
    pub dtype: DType,
}
impl Checkpoint {
    pub fn read(path: &Path, key: Option<&str>) -> Result<Self> {
        let mut bytes = Vec::new();
        File::open(path)?
            .take(160_000_001)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= 160_000_000,
            "Checkpoint exceeds pinned budget"
        );
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        ensure!(
            sha256 == ROSVOT_SHA || sha256 == RWBD_SHA,
            "Unrecognized checkpoint; only the pinned official research weights are supported"
        );
        let mut reader = Cursor::new(bytes.as_slice());
        let mut magic = vec![0; MAGIC.len()];
        reader.read_exact(&mut magic)?;
        ensure!(
            magic == MAGIC,
            "Expected pinned legacy torch tensor checkpoint"
        );
        ensure!(
            object(&mut reader)? == Object::Int(1001),
            "Unsupported torch storage protocol"
        );
        let system = object(&mut reader)?;
        ensure!(
            matches!(&system,Object::Dict(d) if d.iter().any(|(k,v)|*k==Object::Unicode("little_endian".into())&&*v==Object::Bool(true))),
            "Checkpoint must be little endian"
        );
        let root = object(&mut reader)?;
        let mut all = BTreeMap::new();
        collect(root.clone(), "", &mut all)?;
        ensure!(
            !all.is_empty() && all.len() <= 10000,
            "Missing or oversized tensor dictionary"
        );
        let keys = match object(&mut reader)? {
            Object::List(v) => v,
            other => bail!("Invalid storage keys: {other:?}"),
        };
        let mut storage = BTreeMap::new();
        for info in all.values() {
            let id = info.path.trim_start_matches('/').to_owned();
            if let Some((dtype, size)) = storage.insert(id, (info.dtype, info.storage_size)) {
                ensure!(
                    dtype == info.dtype && size == info.storage_size,
                    "Storage aliases disagree"
                );
            }
        }
        let mut offsets = BTreeMap::new();
        let file_size = bytes.len() as u64;
        for key in keys {
            let Object::Unicode(key) = key else {
                bail!("Non-string storage key")
            };
            let (dtype, elements) = storage.get(&key).with_context(|| {
                format!(
                    "Unreferenced tensor storage {key}; {} referenced storages: {:?}",
                    storage.len(),
                    storage.keys().take(8).collect::<Vec<_>>()
                )
            })?;
            let mut bytes = [0; 8];
            reader.read_exact(&mut bytes)?;
            ensure!(
                u64::from_le_bytes(bytes) == *elements as u64,
                "Storage element count differs"
            );
            let offset = reader.stream_position()?;
            let size = elements
                .checked_mul(dtype.size_in_bytes())
                .context("Storage overflow")? as u64;
            ensure!(
                size <= 512 * 1024 * 1024
                    && offset.checked_add(size).is_some_and(|end| end <= file_size),
                "Invalid storage bounds"
            );
            ensure!(
                offsets.insert(key, offset).is_none(),
                "Duplicate storage key"
            );
            reader.seek(SeekFrom::Start(offset + size))?;
        }
        ensure!(
            reader.stream_position()? == file_size,
            "Unexpected trailing checkpoint data"
        );
        let tensors = if let Some(key) = key {
            let mut selected = root;
            for part in key.split('/') {
                let Object::Dict(values) = selected else {
                    bail!("Checkpoint key is not a dictionary")
                };
                selected = values
                    .into_iter()
                    .find(|(k, _)| *k == Object::Unicode(part.into()))
                    .map(|(_, v)| v)
                    .context("Missing checkpoint dictionary key")?;
            }
            let mut selected_tensors = BTreeMap::new();
            collect(selected, "", &mut selected_tensors)?;
            selected_tensors
        } else {
            all
        };
        ensure!(!tensors.is_empty(), "No selected tensor data");
        Ok(Self {
            bytes,
            sha256,
            tensors,
            offsets,
        })
    }
    pub fn get(&self, name: &str) -> Result<Data> {
        let info = self
            .tensors
            .get(name)
            .with_context(|| format!("Missing tensor {name}"))?;
        ensure!(info.layout.is_contiguous(), "Noncontiguous tensor {name}");
        let count = info
            .layout
            .dims()
            .iter()
            .try_fold(1usize, |n, d| n.checked_mul(*d))
            .context("Tensor size overflow")?;
        let size = count
            .checked_mul(info.dtype.size_in_bytes())
            .context("Tensor byte overflow")?;
        ensure!(
            info.layout
                .start_offset()
                .checked_add(size)
                .is_some_and(|n| n <= info.storage_size * info.dtype.size_in_bytes()),
            "Tensor outside storage"
        );
        let id = info.path.trim_start_matches('/');
        let offset =
            self.offsets.get(id).context("Storage offset")? + info.layout.start_offset() as u64;
        let mut file = Cursor::new(self.bytes.as_slice());
        file.seek(SeekFrom::Start(offset))?;
        let mut raw = vec![0; size];
        file.read_exact(&mut raw)?;
        Ok(Data {
            dims: info.layout.dims().iter().map(|d| *d as i64).collect(),
            raw,
            dtype: info.dtype,
        })
    }
}
