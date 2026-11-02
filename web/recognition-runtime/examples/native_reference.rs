//! Compare native CPU and browser SIMD using exactly the same wrapper policy.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err("Use native_reference MODEL MONO_16KHZ_F32LE OUTPUT_JSON".into());
    }
    let model = std::fs::read(&args[0])?;
    let pcm = std::fs::read(&args[1])?;
    if pcm.len() % 4 != 0 {
        return Err("Incomplete float PCM sample".into());
    }
    let samples: Vec<_> = pcm
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    let output = yace_recognition_runtime::recognize_native(&model, &samples, "en")?;
    std::fs::write(&args[2], output)?;
    Ok(())
}
