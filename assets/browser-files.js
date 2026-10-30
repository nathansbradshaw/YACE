// Native file/folder pickers must open in the original browser user gesture,
// before the Rust event crosses the UI connection. Only file transport is JS.
window.__yacePendingPick=null;
function beginPick(mode,accept='') {
  const prior=window.__yacePendingPick;
  if(prior && !prior.claimed)return prior;
  const input=document.createElement('input');
  input.type='file';input.accept=accept;input.hidden=true;
  if(mode==='folder'){input.multiple=true;input.setAttribute('webkitdirectory','');}
  let resolve;
  const promise=new Promise(done=>{resolve=done;});
  const pending={mode,promise,claimed:false};
  window.__yacePendingPick=pending;
  const finish=value=>{input.remove();resolve(value);};
  input.addEventListener('input',event=>event.stopPropagation());
  input.addEventListener('cancel',()=>finish(null),{once:true});
  input.addEventListener('change',async event=>{
    event.stopPropagation();
    try {
      if(!input.files.length){finish(null);return;}
      const form=new FormData();
      for(const file of input.files){
        const name=file.webkitRelativePath?file.webkitRelativePath.split('/').slice(1).join('/'):file.name;
        form.append('file',file,name);
      }
      const response=await fetch('/upload',{method:'POST',body:form});
      if(!response.ok)throw Error(await response.text());
      finish(await response.json());
    } catch(error){finish({error:String(error)});}
  },{once:true});
  const dialogs=document.querySelectorAll('dialog[open]');
  (dialogs[dialogs.length-1]||document.body).append(input);
  try {if(input.showPicker)input.showPicker();else input.click();}
  catch(error){finish({error:String(error)});}
  return pending;
}
document.addEventListener('click',event=>{
  const target=event.target instanceof Element?event.target.closest('[data-browser-pick]'):null;
  if(target && !target.disabled)beginPick(target.getAttribute('data-browser-pick'),target.getAttribute('data-browser-accept')||'');
},true);
window.__yaceTakePick=async(mode,accept)=>{
  const pending=window.__yacePendingPick || beginPick(mode,accept);
  pending.claimed=true;
  const result=await pending.promise;
  if(window.__yacePendingPick===pending)window.__yacePendingPick=null;
  return result;
};
