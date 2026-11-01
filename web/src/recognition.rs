//! Rust host for the desktop recognition engine compiled as a WASI reactor.
//! No filesystem, environment, network or additional worker capability is given
//! to the engine. Its only inputs are verified model bytes and local PCM.
use anyhow::{Context, Result, ensure};
use js_sys::{
    Array, Function, Object, Reflect, Uint8Array,
    WebAssembly::{Instance, Memory, Module},
};
use serde::Deserialize;
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_futures::JsFuture;

#[derive(Deserialize)]
pub struct Output {
    pub language: String,
    pub segments: Vec<crate::clean_transcript::RecognizedSegment>,
}
fn error(value: JsValue) -> anyhow::Error {
    anyhow::anyhow!("Local recognition runtime: {value:?}")
}
fn field(object: &JsValue, name: &str) -> Result<JsValue> {
    Reflect::get(object, &name.into()).map_err(error)
}
fn set(object: &Object, name: &str, value: &JsValue) -> Result<()> {
    Reflect::set(object, &name.into(), value).map_err(error)?;
    Ok(())
}
type SharedMemory = Rc<RefCell<Option<Memory>>>;
fn write(memory: &SharedMemory, address: u32, bytes: &[u8]) -> bool {
    let borrow = memory.borrow();
    let Some(memory) = borrow.as_ref() else {
        return false;
    };
    let view = Uint8Array::new(&memory.buffer());
    let Some(end) = address.checked_add(bytes.len() as u32) else {
        return false;
    };
    if end > view.length() {
        return false;
    }
    view.subarray(address, end).copy_from(bytes);
    true
}
fn put32(memory: &SharedMemory, address: u32, value: u32) -> u32 {
    if write(memory, address, &value.to_le_bytes()) {
        0
    } else {
        21
    }
}
fn read32(memory: &SharedMemory, address: u32) -> Option<u32> {
    let borrow = memory.borrow();
    let mem = borrow.as_ref()?;
    let view = Uint8Array::new(&mem.buffer());
    if address.checked_add(4)? > view.length() {
        return None;
    }
    let mut bytes = [0; 4];
    view.subarray(address, address + 4).copy_to(&mut bytes);
    Some(u32::from_le_bytes(bytes))
}
fn imports(memory: SharedMemory) -> Result<Object> {
    let wasi = Object::new();
    for name in ["environ_get", "environ_sizes_get"] {
        let mem = memory.clone();
        let closure = Closure::<dyn FnMut(u32, u32) -> u32>::new(move |a, b| {
            if put32(&mem, a, 0) != 0 || put32(&mem, b, 0) != 0 {
                21
            } else {
                0
            }
        });
        set(&wasi, name, &closure.into_js_value())?;
    }
    let mem = memory.clone();
    let clock = Closure::<dyn FnMut(u32, JsValue, u32) -> u32>::new(move |id, _, address| {
        if id > 3 {
            return 28;
        }
        let now = (js_sys::Date::now() * 1_000_000.) as u64;
        if write(&mem, address, &now.to_le_bytes()) {
            0
        } else {
            21
        }
    });
    set(&wasi, "clock_time_get", &clock.into_js_value())?;
    let mem = memory.clone();
    let fdwrite = Closure::<dyn FnMut(u32, u32, u32, u32) -> u32>::new(
        move |fd: u32, rows: u32, count: u32, written: u32| {
            if ![1, 2].contains(&fd) {
                return 8;
            }
            if count > 4096 {
                return 28;
            }
            let mut size = 0u32;
            for i in 0..count {
                let Some(row) = rows.checked_add(i * 8) else {
                    return 21;
                };
                let Some(address) = read32(&mem, row) else {
                    return 21;
                };
                let Some(length) = read32(&mem, row + 4) else {
                    return 21;
                };
                let borrow = mem.borrow();
                let Some(memory) = borrow.as_ref() else {
                    return 21;
                };
                if address
                    .checked_add(length)
                    .is_none_or(|end| end > Uint8Array::new(&memory.buffer()).length())
                {
                    return 21;
                }
                let Some(next) = size.checked_add(length) else {
                    return 28;
                };
                size = next;
            }
            // Native diagnostic text is not a capability to publish local content.
            put32(&mem, written, size)
        },
    );
    set(&wasi, "fd_write", &fdwrite.into_js_value())?;
    let mem = memory.clone();
    let stat = Closure::<dyn FnMut(u32, u32) -> u32>::new(move |fd, address| {
        if fd > 2 {
            return 8;
        }
        let mut bytes = [0u8; 24];
        bytes[0] = 2; // character device, read/write only
        bytes[8..16].copy_from_slice(&(if fd == 0 { 2u64 } else { 64u64 }).to_le_bytes());
        if write(&mem, address, &bytes) { 0 } else { 21 }
    });
    set(&wasi, "fd_fdstat_get", &stat.into_js_value())?;
    let close = Closure::<dyn FnMut(u32) -> u32>::new(|_| 8);
    set(&wasi, "fd_close", &close.into_js_value())?;
    for name in ["fd_fdstat_set_flags", "fd_prestat_get"] {
        let denied = Closure::<dyn FnMut(u32, u32) -> u32>::new(|_, _| 8);
        set(&wasi, name, &denied.into_js_value())?;
    }
    let denied = Closure::<dyn FnMut(u32, u32, u32) -> u32>::new(|_, _, _| 8);
    set(&wasi, "fd_prestat_dir_name", &denied.into_js_value())?;
    let mem = memory.clone();
    let read = Closure::<dyn FnMut(u32, u32, u32, u32) -> u32>::new(move |fd, _, _, count| {
        if fd == 0 { put32(&mem, count, 0) } else { 8 }
    });
    set(&wasi, "fd_read", &read.into_js_value())?;
    let denied = Closure::<dyn FnMut(u32, JsValue, u32, u32) -> u32>::new(|_, _, _, _| 8);
    set(&wasi, "fd_seek", &denied.into_js_value())?;
    let denied = Closure::<dyn FnMut() -> u32>::new(|| 8);
    set(&wasi, "path_open", &denied.into_js_value())?;
    let exit = Closure::<dyn FnMut(u32)>::new(|code| {
        wasm_bindgen::throw_str(&format!("Recognition engine exited ({code})"))
    });
    set(&wasi, "proc_exit", &exit.into_js_value())?;
    let progress = Closure::<dyn FnMut(i32)>::new({
        let mut last = 0.;
        let mut latest = -1;
        move |percent| {
            if percent >= 0 {
                latest = percent;
            }
            let now = js_sys::Date::now();
            if now - last > 1000. {
                last = now;
                crate::worker::progress(if latest >= 0 {
                    format!("Recognizing lyrics locally · {latest}%")
                } else {
                    "Recognizing lyrics locally…".into()
                });
            }
        }
    });
    let app = Object::new();
    set(&app, "progress", &progress.into_js_value())?;
    let imports = Object::new();
    set(&imports, "wasi_snapshot_preview1", &wasi)?;
    set(&imports, "yace", &app)?;
    Ok(imports)
}
fn call(exports: &Object, name: &str, args: &[JsValue]) -> Result<JsValue> {
    let function = field(exports, name)?
        .dyn_into::<Function>()
        .map_err(error)?;
    let array = Array::new();
    for arg in args {
        array.push(arg);
    }
    function.apply(&JsValue::UNDEFINED, &array).map_err(error)
}
pub async fn recognize(samples: &[f32], model: &[u8], language: &str) -> Result<Output> {
    ensure!(
        !samples.is_empty() && samples.len() <= 16000 * 3600,
        "Invalid recognition source length"
    );
    let bytes = Uint8Array::from(include_bytes!("../runtime/yace-recognition.wasm").as_slice());
    let module = Module::new(&bytes).map_err(error)?;
    let memory = Rc::new(RefCell::new(None));
    let imports = imports(memory.clone())?;
    for item in Module::imports(&module).iter() {
        let module_name = field(&item, "module")?
            .as_string()
            .context("No import module")?;
        let name = field(&item, "name")?
            .as_string()
            .context("No import name")?;
        let group = field(&imports, &module_name)?;
        ensure!(
            !group.is_undefined() && field(&group, &name)?.is_function(),
            "Unsupported recognition capability: {module_name}.{name}"
        );
    }
    let instance = JsFuture::from(js_sys::WebAssembly::instantiate_module(&module, &imports))
        .await
        .map_err(error)?
        .dyn_into::<Instance>()
        .map_err(error)?;
    let exports = instance.exports();
    *memory.borrow_mut() = Some(
        field(&exports, "memory")?
            .dyn_into::<Memory>()
            .map_err(error)?,
    );
    call(&exports, "__wasm_call_ctors", &[])?;
    let allocate = |bytes: usize| -> Result<u32> {
        let address = call(&exports, "yace_alloc", &[(bytes as u32).into()])?
            .as_f64()
            .context("No input allocation")? as u32;
        ensure!(address != 0, "Recognition input allocation failed");
        Ok(address)
    };
    let model_address = allocate(model.len())?;
    ensure!(
        write(&memory, model_address, model),
        "Cannot copy lyric model"
    );
    let sample_address = allocate(std::mem::size_of_val(samples))?;
    // WASM is little endian; avoid a second complete Rust PCM allocation.
    let sample_bytes = unsafe {
        std::slice::from_raw_parts(
            samples.as_ptr().cast::<u8>(),
            std::mem::size_of_val(samples),
        )
    };
    ensure!(
        write(&memory, sample_address, sample_bytes),
        "Cannot copy recognition audio"
    );
    let language = std::ffi::CString::new(language)?;
    let language_address = allocate(language.as_bytes_with_nul().len())?;
    ensure!(
        write(&memory, language_address, language.as_bytes_with_nul()),
        "Cannot copy language"
    );
    let code = call(
        &exports,
        "yace_recognize",
        &[
            model_address.into(),
            (model.len() as u32).into(),
            sample_address.into(),
            (samples.len() as u32).into(),
            language_address.into(),
        ],
    )?
    .as_f64()
    .context("No recognition result")?;
    let address = call(&exports, "yace_result_ptr", &[])?
        .as_f64()
        .context("No result address")? as u32;
    let length = call(&exports, "yace_result_len", &[])?
        .as_f64()
        .context("No result length")? as u32;
    ensure!(
        length <= 16 * 1024 * 1024,
        "Recognition result exceeds limit"
    );
    let borrow = memory.borrow();
    let mem = borrow.as_ref().unwrap();
    let view = Uint8Array::new(&mem.buffer());
    let end = address
        .checked_add(length)
        .context("Result address overflow")?;
    ensure!(end <= view.length(), "Recognition result outside memory");
    let bytes = view.subarray(address, end).to_vec();
    ensure!(code == 0., "{}", String::from_utf8_lossy(&bytes));
    Ok(serde_json::from_slice(&bytes)?)
}
