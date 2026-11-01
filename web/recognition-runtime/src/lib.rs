//! The desktop whisper.cpp engine, isolated in a single-threaded WASI reactor.
//! Only bounded input/output buffers cross the browser host boundary.
use serde::Serialize;
use std::{
    cell::RefCell,
    ffi::{CStr, c_char, c_void},
    ptr,
};
#[allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code
)]
#[cfg(target_arch = "wasm32")]
mod ffi {
    include!(concat!(env!("OUT_DIR"), "/whisper.rs"));
}
#[cfg(not(target_arch = "wasm32"))]
mod ffi {
    pub use whisper_rs_sys::*;
}

#[link(wasm_import_module = "yace")]
#[cfg(target_arch = "wasm32")]
unsafe extern "C" {
    fn progress(percent: i32);
}
#[cfg(not(target_arch = "wasm32"))]
unsafe fn progress(percent: i32) {
    if percent >= 0 {
        eprintln!("Recognition {percent}%");
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub fn recognize_native(model: &[u8], samples: &[f32], language: &str) -> Result<Vec<u8>, String> {
    let language = std::ffi::CString::new(language).map_err(|e| e.to_string())?;
    serde_json::to_vec_pretty(&recognize(model, samples, &language)?).map_err(|e| e.to_string())
}
thread_local! { static RESULT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) }; }
#[derive(Serialize)]
struct Segment {
    text: String,
    start_seconds: f64,
    end_seconds: f64,
    mean_token_probability: f32,
    no_speech_probability: f32,
}
#[derive(Serialize)]
struct Recognition {
    language: String,
    segments: Vec<Segment>,
}
struct Context(*mut ffi::whisper_context);
impl Drop for Context {
    fn drop(&mut self) {
        unsafe { ffi::whisper_free(self.0) }
    }
}
unsafe extern "C" fn report(
    _: *mut ffi::whisper_context,
    _: *mut ffi::whisper_state,
    percent: i32,
    _: *mut c_void,
) {
    unsafe { progress(percent) }
}
unsafe extern "C" fn keep_alive(_: *mut c_void) -> bool {
    unsafe { progress(-1) };
    false
}
unsafe fn text(value: *const c_char) -> Result<String, String> {
    if value.is_null() {
        return Err("Recognition returned no text".into());
    }
    unsafe { CStr::from_ptr(value) }
        .to_str()
        .map(str::to_owned)
        .map_err(|e| e.to_string())
}
fn recognize(model: &[u8], samples: &[f32], language: &CStr) -> Result<Recognition, String> {
    if model.len() < 1024
        || samples.len() < 1600
        || samples.len() > 16000 * 3600
        || samples.iter().any(|s| !s.is_finite())
    {
        return Err("Invalid recognition input".into());
    }
    unsafe {
        let context_params = ffi::whisper_context_default_params_by_ref();
        if context_params.is_null() {
            return Err("No context parameters".into());
        }
        (*context_params).use_gpu = false;
        let context = ffi::whisper_init_from_buffer_with_params(
            model.as_ptr().cast_mut().cast(),
            model.len(),
            ptr::read(context_params),
        );
        ffi::whisper_free_context_params(context_params);
        if context.is_null() {
            return Err("Cannot load the local lyric model".into());
        }
        let context = Context(context);
        let params = ffi::whisper_full_default_params_by_ref(
            ffi::whisper_sampling_strategy_WHISPER_SAMPLING_BEAM_SEARCH,
        );
        if params.is_null() {
            return Err("No recognition parameters".into());
        }
        (*params).n_threads = 1;
        (*params).language = language.as_ptr();
        (*params).translate = false;
        (*params).no_context = true;
        (*params).token_timestamps = true;
        (*params).max_len = 1;
        (*params).split_on_word = true;
        (*params).suppress_blank = true;
        (*params).suppress_nst = true;
        (*params).temperature = 0.;
        (*params).temperature_inc = 0.;
        (*params).print_special = false;
        (*params).print_progress = false;
        (*params).print_realtime = false;
        (*params).print_timestamps = false;
        (*params).beam_search.beam_size = 5;
        (*params).beam_search.patience = -1.;
        (*params).progress_callback = Some(report);
        (*params).abort_callback = Some(keep_alive);
        let result = ffi::whisper_full(
            context.0,
            ptr::read(params),
            samples.as_ptr(),
            samples.len() as i32,
        );
        ffi::whisper_free_params(params);
        if result != 0 {
            return Err(format!("Local lyric recognition failed ({result})"));
        }
        let mut segments = Vec::new();
        let eot = ffi::whisper_token_eot(context.0);
        for i in 0..ffi::whisper_full_n_segments(context.0) {
            let mut probabilities = Vec::new();
            let mut start = i64::MAX;
            let mut end = -1;
            for j in 0..ffi::whisper_full_n_tokens(context.0, i) {
                let token = ffi::whisper_full_get_token_data(context.0, i, j);
                if token.id < eot {
                    probabilities.push(token.p);
                    let token_text = ffi::whisper_full_get_token_text(context.0, i, j);
                    if !token_text.is_null()
                        && CStr::from_ptr(token_text)
                            .to_bytes()
                            .iter()
                            .any(u8::is_ascii_alphanumeric)
                        && token.t0 >= 0
                        && token.t1 > token.t0
                    {
                        start = start.min(token.t0);
                        end = end.max(token.t1);
                    }
                }
            }
            segments.push(Segment {
                text: text(ffi::whisper_full_get_segment_text(context.0, i))?,
                start_seconds: if end > start {
                    start as f64 / 100.
                } else {
                    -1.
                },
                end_seconds: if end > start { end as f64 / 100. } else { -1. },
                mean_token_probability: if probabilities.is_empty() {
                    0.
                } else {
                    probabilities.iter().sum::<f32>() / probabilities.len() as f32
                },
                no_speech_probability: ffi::whisper_full_get_segment_no_speech_prob(context.0, i),
            });
        }
        Ok(Recognition {
            language: text(ffi::whisper_lang_str(ffi::whisper_full_lang_id(context.0)))?,
            segments,
        })
    }
}
// Buffers are allocated in this module's memory and owned by its host instance.
// No raw address from another process or module is accepted by the host adapter.
#[unsafe(no_mangle)]
pub extern "C" fn yace_alloc(bytes: usize) -> *mut u8 {
    if bytes == 0 || bytes > 1024 * 1024 * 1024 {
        return ptr::null_mut();
    }
    // The host also puts f32 samples in these buffers; keep their alignment
    // explicit rather than relying on a byte Vec allocator's implementation.
    let layout = std::alloc::Layout::from_size_align(bytes, 16).unwrap();
    unsafe { std::alloc::alloc_zeroed(layout) }
}
#[unsafe(no_mangle)]
/// # Safety
/// `value` must be a still-live allocation returned by `yace_alloc`, with its
/// original byte length. No reference to that allocation may remain in use.
pub unsafe extern "C" fn yace_free(value: *mut u8, bytes: usize) {
    if !value.is_null() {
        unsafe {
            std::alloc::dealloc(
                value,
                std::alloc::Layout::from_size_align(bytes, 16).unwrap(),
            );
        }
    }
}
#[unsafe(no_mangle)]
/// # Safety
/// All pointers must refer to live allocations in this module's memory for the
/// declared lengths. Samples must be aligned f32 values; language must be a
/// terminated C string. The host retains these buffers until the call returns.
pub unsafe extern "C" fn yace_recognize(
    model: *const u8,
    model_len: usize,
    samples: *const f32,
    sample_count: usize,
    language: *const c_char,
) -> i32 {
    let result = unsafe {
        recognize(
            std::slice::from_raw_parts(model, model_len),
            std::slice::from_raw_parts(samples, sample_count),
            CStr::from_ptr(language),
        )
    };
    let (code, bytes) = match result {
        Ok(value) => (0, serde_json::to_vec(&value).unwrap()),
        Err(error) => (
            1,
            serde_json::to_vec(&serde_json::json!({"error":error})).unwrap(),
        ),
    };
    RESULT.with(|cell| *cell.borrow_mut() = bytes);
    code
}
#[unsafe(no_mangle)]
pub extern "C" fn yace_result_ptr() -> *const u8 {
    RESULT.with(|v| v.borrow().as_ptr())
}
#[unsafe(no_mangle)]
pub extern "C" fn yace_result_len() -> usize {
    RESULT.with(|v| v.borrow().len())
}
