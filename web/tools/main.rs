//! Build static WebAssembly assets or serve those bytes for local acceptance.
use std::{env,fs,io::{Read,Write},net::{TcpListener,TcpStream},path::{Component,Path,PathBuf},process::Command};
type Result<T>=std::result::Result<T,Box<dyn std::error::Error>>;
#[cfg(test)]
#[path = "../src/audio_decode.rs"]
mod audio_decode;
#[cfg(test)]
mod tests {
    use super::audio_decode::{self, Pcm};
    fn ready<T>(future: impl std::future::Future<Output = T>) -> T {
        struct Wake;
        impl std::task::Wake for Wake { fn wake(self: std::sync::Arc<Self>) {} }
        let waker = std::task::Waker::from(std::sync::Arc::new(Wake));
        let mut context = std::task::Context::from_waker(&waker);
        match std::pin::pin!(future).as_mut().poll(&mut context) {
            std::task::Poll::Ready(value) => value,
            std::task::Poll::Pending => panic!("native audio decoding should complete synchronously"),
        }
    }
    #[test]
    fn playback_wave_keeps_source_clock_stereo_and_sample_polarity() {
        let stereo = (0..48_000).flat_map(|n| {
            let sample = (n as f32 * 0.04).sin() * 0.8;
            [sample, -sample]
        }).collect();
        let pcm = Pcm { stereo, rate: 48_000 };
        let decoded = ready(audio_decode::decode(&pcm.wave().unwrap())).unwrap();
        assert_eq!(decoded.rate, pcm.rate);
        assert_eq!(decoded.stereo.len(), pcm.stereo.len());
        assert_eq!(decoded.duration(), 1.);
        assert!(decoded.stereo.iter().zip(&pcm.stereo).all(|(a,b)| (a-b).abs() < 0.00005));
    }
    #[test]
    fn corrupt_audio_is_rejected_before_replacing_the_open_project() {
        assert!(ready(audio_decode::decode(b"not an audio recording")).is_err());
    }
}
fn run(command:&mut Command)->Result<()>{let status=command.status()?;if !status.success(){return Err(format!("Build command failed: {command:?}").into())}Ok(())}
fn build(root:&Path)->Result<()>{
 let target=env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(||root.join("target/web-static"));
 run(Command::new("cargo").current_dir(root).env("CARGO_TARGET_DIR",&target).args(["build","--locked","--release","--manifest-path","web/Cargo.toml","--target","wasm32-unknown-unknown"]))?;
 let dist=root.join("web/dist");fs::create_dir_all(&dist)?;
 run(Command::new("wasm-bindgen").arg(target.join("wasm32-unknown-unknown/release/yace_web.wasm")).args(["--target","web","--no-typescript","--out-dir"]).arg(&dist))?;
 fs::copy(root.join("web/index.html"),dist.join("index.html"))?;
 println!("Static browser app: {}",dist.display());Ok(())
}
fn response(stream:&mut TcpStream,status:&str,mime:&str,body:&[u8],head:bool)->Result<()>{write!(stream,"HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",body.len())?;if !head{stream.write_all(body)?;}Ok(())}
fn serve_file(mut stream:TcpStream,root:&Path)->Result<()>{
 let mut buffer=[0u8;8192];let count=stream.read(&mut buffer)?;let request=std::str::from_utf8(&buffer[..count])?;let mut words=request.lines().next().unwrap_or_default().split_whitespace();let method=words.next().unwrap_or_default();let uri=words.next().unwrap_or_default().split('?').next().unwrap_or_default();if !matches!(method,"GET"|"HEAD"){return response(&mut stream,"405 Method Not Allowed","text/plain",b"Static files only",false)}
 let relative=if uri=="/"{"index.html"}else{uri.trim_start_matches('/')};let path=Path::new(relative);if !path.components().all(|part|matches!(part,Component::Normal(_))){return response(&mut stream,"400 Bad Request","text/plain",b"Invalid path",method=="HEAD")}
 let path=root.join(path);let Ok(path)=path.canonicalize()else{return response(&mut stream,"404 Not Found","text/plain",b"No file",method=="HEAD")};if !path.starts_with(root)||!path.is_file(){return response(&mut stream,"404 Not Found","text/plain",b"No file",method=="HEAD")}
 let mime=match path.extension().and_then(|s|s.to_str()){Some("html")=>"text/html; charset=utf-8",Some("js")=>"text/javascript; charset=utf-8",Some("wasm")=>"application/wasm",Some("css")=>"text/css; charset=utf-8",Some("json")=>"application/json",_=>"application/octet-stream"};let bytes=fs::read(&path)?;println!("{method} {uri}");response(&mut stream,"200 OK",mime,&bytes,method=="HEAD")
}
fn main()->Result<()>{let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap().to_owned();let args:Vec<_>=env::args().skip(1).collect();match args.first().map(String::as_str){Some("build")=>build(&root),Some("serve")=>{let port=args.get(1).map(|s|s.parse()).transpose()?.unwrap_or(3032u16);let dist=root.join("web/dist").canonicalize()?;let listener=TcpListener::bind((std::net::Ipv4Addr::LOCALHOST,port))?;println!("Static YACE: http://127.0.0.1:{port}/");for stream in listener.incoming(){let stream=stream?;let dist=dist.clone();std::thread::spawn(move||{if let Err(error)=serve_file(stream,&dist){eprintln!("Static response: {error}");}});}Ok(())},_=>Err("Use: yace-static-web build | serve [port]".into())}}
