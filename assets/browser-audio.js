// LiveView's browser API bindings. Rust owns transport, routing and drift policy.
const config = await dioxus.recv();
window.__yaceAudioPreparing = config.id;
window.__yaceAudio?.pause();
const context = new AudioContext();
const output = context.createGain();
output.connect(context.destination);
let disposed = false, intent = 0, error = null, baseRate = config.rate;
const channels = [], gains = [], nodes = [];
const position = () => channels[0]?.currentTime || 0;
function pause() { intent++; for (const element of channels) element.pause(); }
async function play() {
  if (disposed || !channels[0]?.paused) return;
  const ticket = ++intent;
  try {
    await context.resume();
    if (disposed || ticket !== intent) return;
    await Promise.all(channels.map(element => element.play()));
    if (disposed || ticket !== intent) for (const element of channels) element.pause();
  } catch (failure) { if (ticket === intent && !disposed) { pause(); error = String(failure); } }
}
function seek(seconds) {
  const target = Math.max(0, Math.min(seconds, channels[0]?.duration || 0));
  for (const element of channels) { element.currentTime = target; element.playbackRate = baseRate; }
}
function rate(value) { baseRate = value; for (const element of channels) element.playbackRate = value; }
function weights(values) {
  gains.forEach((gain, index) => gain.gain.setValueAtTime(values[index] || 0, context.currentTime));
}
function synchronize(commands) {
  for (const command of commands) {
    const element = channels[command.index];
    if (!element) continue;
    element.playbackRate = command.rate;
    if (command.seek !== undefined) element.currentTime = command.seek;
  }
}
function unlock() { if (context.state === 'suspended') context.resume(); }
function dispose() {
  if (disposed) return;
  pause(); disposed = true; clearInterval(timer);
  for (const element of channels) { element.removeAttribute('src'); element.load(); element.remove(); }
  for (const node of nodes) node.disconnect();
  for (const gain of gains) gain.disconnect();
  output.disconnect(); context.close();
  window.removeEventListener('beforeunload', dispose);
  window.removeEventListener('pointerdown', unlock, true);
  window.removeEventListener('keydown', unlock, true);
  window.ipc.ws.removeEventListener('close', dispose);
  try { dioxus.send({disposed:true}); } catch (_) {}
}
const timer = setInterval(() => {
  if (disposed) return;
  dioxus.send({position:position(), paused:channels[0]?.paused ?? true,
    ended:channels[0]?.ended ?? false, context:context.state, error,
    channels:channels.map(element => ({position:element.currentTime,
      ready:!element.paused && !element.seeking && element.readyState >= 3}))});
  error = null;
}, 33);
const controls = {id:config.id, play, pause, seek, rate, weights, synchronize,
  volume:value => output.gain.setValueAtTime(value, context.currentTime), dispose};
window.addEventListener('pointerdown', unlock, true);
window.addEventListener('keydown', unlock, true);
window.addEventListener('beforeunload', dispose);
window.ipc.ws.addEventListener('close', dispose, {once:true});
try {
  await Promise.all(config.urls.map(url => new Promise((resolve, reject) => {
    const element = new Audio();
    element.preload = 'auto'; element.hidden = true; element.preservesPitch = true;
    element.mozPreservesPitch = true; element.webkitPreservesPitch = true;
    element.playbackRate = baseRate;
    const gain = context.createGain(); gain.connect(output);
    const node = context.createMediaElementSource(element); node.connect(gain);
    channels.push(element); gains.push(gain); nodes.push(node);
    element.addEventListener('loadedmetadata', resolve, {once:true});
    element.addEventListener('error', () => { error = 'Recording playback failed'; reject(Error(error)); }, {once:true});
    document.body.append(element); element.src = url;
  })));
  weights(config.weights);
  if (window.__yaceAudioPreparing !== config.id || disposed) throw Error('Audio preparation was superseded');
  window.__yaceAudio?.dispose(); window.__yaceAudio = controls;
  dioxus.send({ready:true, sample_rate:context.sampleRate});
} catch (failure) { dioxus.send({ready:false, error:String(failure)}); dispose(); }
await new Promise(() => {});
