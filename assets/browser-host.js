// Browser API glue only. Commands and persistence remain in the Rust editor.
document.addEventListener('keydown',event=>{
  const target=event.target;
  if(!(target instanceof Element) || !target.closest('.studio-app') || target.closest('input,textarea,select,[contenteditable=true],button,a'))return;
  if(event.key===' ' || event.key==='Backspace' || event.key==='Delete' || ((event.metaKey||event.ctrlKey)&&['a','c','x','v','z'].includes(event.key.toLowerCase())))event.preventDefault();
},true);
window.ipc.ws.addEventListener('close',()=>{
  window.__yaceAudio?.dispose();
  const panel=document.createElement('div');panel.setAttribute('role','alert');
  panel.style.cssText='position:fixed;inset:0;z-index:40000;background:#111315ed;color:#e7ecef;display:grid;place-content:center;padding:32px;font:16px system-ui;gap:16px';
  const message=document.createElement('p');message.textContent='Connection lost. Editing has stopped. Reopen to load the last saved revision.';
  const link=document.createElement('a');link.href='/';link.textContent='Reopen editor';link.style.color='#41c6b8';panel.append(message,link);document.body.append(panel);
},{once:true});
