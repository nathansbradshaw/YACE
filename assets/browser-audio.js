// Browser-only Web Audio bindings. Rust owns transport, mixing policy and edits.
const config = await dioxus.recv();
window.__yaceAudioPreparing=config.id;
window.__yaceAudio?.pause();
const context = new AudioContext();
const output = context.createGain();
output.connect(context.destination);
let paused = true, offset = 0, anchor = 0, ended = false, disposed = false;
let nodes = [], gains = [], buffers = [];
function position() {
  return Math.min(buffers[0]?.duration || 0, offset + (paused ? 0 : context.currentTime-anchor));
}
function stop() {
  for (const node of nodes) { try {node.stop();} catch (_) {} node.disconnect(); }
  nodes=[];
}
function pause() { offset=position(); paused=true; stop(); }
function weights(values) {
  for(let i=0;i<gains.length;i++) gains[i].gain.setValueAtTime(values[i] || 0,context.currentTime);
}
function play() {
  if (!paused || disposed) return;
  context.resume();
  stop(); ended=false;
  const when=context.currentTime+0.02;
  nodes=buffers.map((buffer,i)=>{
    const node=context.createBufferSource();node.buffer=buffer;
    node.connect(gains[i]);node.start(when,Math.min(offset,buffer.duration));return node;
  });
  anchor=when;paused=false;
  nodes[0].onended=()=>{if(!paused && position()>=buffers[0].duration-0.03){offset=buffers[0].duration;paused=true;ended=true;stop();}};
}
function seek(seconds) {const playing=!paused;pause();offset=Math.max(0,Math.min(seconds,buffers[0].duration));ended=false;if(playing)play();}
function dispose(){if(disposed)return;pause();disposed=true;clearInterval(timer);for(const gain of gains)gain.disconnect();output.disconnect();buffers=[];gains=[];context.close();window.removeEventListener('beforeunload',dispose);window.removeEventListener('pointerdown',unlock,true);window.removeEventListener('keydown',unlock,true);window.ipc.ws.removeEventListener('close',dispose);try{dioxus.send({disposed:true});}catch(_){} }
function unlock(){if(context.state==='suspended')context.resume();}
const timer=setInterval(()=>{
  if(disposed)return;
  dioxus.send({position:Math.max(0,position()),paused,ended,context:context.state});
},33);
const controls={id:config.id,play,pause,seek,weights,volume:v=>output.gain.setValueAtTime(v,context.currentTime),dispose};
window.addEventListener('pointerdown',unlock,true);
window.addEventListener('keydown',unlock,true);
window.addEventListener('beforeunload',dispose);
window.ipc.ws.addEventListener('close',dispose,{once:true});
try {
  buffers=await Promise.all(config.urls.map(async url=>{
    const response=await fetch(url);if(!response.ok)throw Error('Audio request failed');
    return context.decodeAudioData(await response.arrayBuffer());
  }));
  gains=buffers.map(()=>{const gain=context.createGain();gain.connect(output);return gain;});
  weights(config.weights);
  if(window.__yaceAudioPreparing!==config.id || disposed)throw Error('Audio preparation was superseded');
  window.__yaceAudio?.dispose();
  window.__yaceAudio=controls;
  dioxus.send({ready:true,sample_rate:context.sampleRate});
} catch(error) {dioxus.send({ready:false,error:String(error)});dispose();}
await new Promise(()=>{});
