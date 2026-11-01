//! Rebuild the synthetic codec fixture; takes a new output WAV path.
use std::{fs,io::Write};
fn main(){
    let path=std::env::args_os().nth(1).expect("output WAV path");
    let mut out=fs::OpenOptions::new().write(true).create_new(true).open(path).unwrap();
    let frames=4410u32;let size=frames*8;
    out.write_all(b"RIFF").unwrap();out.write_all(&(size+36).to_le_bytes()).unwrap();out.write_all(b"WAVEfmt ").unwrap();
    for bytes in [16u32.to_le_bytes().to_vec(),3u16.to_le_bytes().to_vec(),2u16.to_le_bytes().to_vec(),44100u32.to_le_bytes().to_vec(),(44100u32*8).to_le_bytes().to_vec(),8u16.to_le_bytes().to_vec(),32u16.to_le_bytes().to_vec()]{out.write_all(&bytes).unwrap();}
    out.write_all(b"data").unwrap();out.write_all(&size.to_le_bytes()).unwrap();
    for i in 0..frames {for (hz,gain) in [(440.,1.3),(880.,0.2)]{let sample=(i as f32*std::f32::consts::TAU*hz/44100.).sin()*gain;out.write_all(&sample.to_le_bytes()).unwrap();}}
}
