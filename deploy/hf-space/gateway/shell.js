const el = id => document.getElementById(id);
let state, busy = false, checks = [], toolLoaded = false, profiles = [], publicKey = null, lastLoginResult = null, toasted, lastRoute;
function toast(text) { el('toast').textContent=text;el('toast').hidden=false;clearTimeout(toasted);toasted=setTimeout(()=>el('toast').hidden=true,7000); }
function age(at) { if(!at)return 'Not yet verified';const seconds=Math.max(0,Math.floor(Date.now()/1000-at));return seconds<60?`${seconds}s ago`:`${Math.floor(seconds/60)} min ago`; }
async function json(path, options) { const response=await fetch(path,{cache:'no-store',...options});const value=await response.json();if(!response.ok)throw new Error(value.error||'The service is reconnecting. Try again shortly.');return value; }
async function action(name, body={}) {
 if(busy)return;busy=true;document.querySelectorAll('button').forEach(b=>b.disabled=true);
 try {const value=await json(`/_space/action/${name}`,{method:'POST',headers:{'Content-Type':'application/json','X-Space-CSRF':state.csrf},body:JSON.stringify(body)});if(value.message)toast(value.message);await refresh();return value;}
 catch(error){toast(error.message);}
 finally{busy=false;document.querySelectorAll('button').forEach(b=>b.disabled=false);}
}
function route(value){try{const url=new URL(value,location.origin);return url.origin===location.origin&&!url.pathname.startsWith('/_space')?url.pathname+url.search+url.hash:'/agents';}catch{return '/agents';}}
function renderTools(){
 const node=el('tools');node.replaceChildren();
 for(const [id,name] of [['codex','Codex'],['claude','Claude Code'],['opencode','OpenCode'],['github','GitHub']]) {
  const check=checks.find(c=>c.id===id&&!c.profile);const available=profiles.some(p=>p.harness===id);const connected=check?.signed_in===true||checks.some(c=>c.id===id&&c.profile&&c.signed_in===true);
  const row=document.createElement('div');row.className='tool';
  const text=document.createElement('div'),title=document.createElement('strong'),detail=document.createElement('p');title.textContent=name;
  detail.textContent=connected?`${check.detail||'Connected'} · ${state.backup?.credentials_at&&!state.backup.credentials_pending?'Credential backup '+age(state.backup.credentials_at):'Backup pending'}`:available?'Profile exists · Check or reconnect if authorization expired':check?.cli_found===false?'Not installed in this image':'Ready to connect';
  text.append(title,detail);const button=document.createElement('button');button.textContent=available?'Open Launchpad':connected&&id!=='github'?'Create profile':connected?'Reconnect':'Connect';
  button.onclick=()=>available?el('controls').close():action(connected&&id!=='github'?'profile':'login',{tool:id}).then(()=>connected?loadTools():null);
  const actions=document.createElement('div');actions.className='actions';actions.append(button);row.append(text,actions);if(available){const reconnect=document.createElement('button');reconnect.textContent='Reconnect';reconnect.onclick=()=>action('login',{tool:id});actions.append(reconnect);}node.append(row);
 }
}
async function loadTools(){try{const value=await json('/_space/tools');checks=value.checks;profiles=value.profiles;el('hf-state').textContent=value.hf?.check?.signed_in===true?`Connected as ${value.hf.account||'your HF account'}`:'HF agent CLI is not signed in.';toolLoaded=true;renderTools();}catch(error){toast(error.message);}}
async function refresh(){
 try{
  state=await json('/_space/status');el('settings').href=el('hub').href=state.settings;
  el('startup-message').textContent=state.message;el('create-key').hidden=!state.needs_key;el('manual-key').hidden=!state.needs_key;el('create-bucket').hidden=!state.needs_bucket;
  el('title').textContent=state.needs_key?'Keep your credentials safe.':'Your Space is getting ready.';
  el('bar-status').textContent=state.maintenance?'Maintenance · reconnecting':state.ready?(state.backup?.error?'Backup delayed':'Recovery '+age(state.backup?.checkpoint_at)):'Startup paused';
  document.body.classList.toggle('ready',state.ready);
  if(state.ready&&!el('workspace').getAttribute('src')){lastRoute=route(new URLSearchParams(location.search).get('route')||'/agents');el('workspace').src=lastRoute;}
  const runtime=state.policy?.runtime||{};el('hosting').textContent=`${runtime.hardware?.current||'Hardware reported by Hub'} · Sleep ${runtime.gcTimeout===-1?'off':runtime.gcTimeout?runtime.gcTimeout+'s':'setting in Hub'} · ${Math.round((state.disk?.free||0)/1073741824)} GiB free · Startup verified ${age(state.policy?.checked_at)}`;
  el('backup').textContent=state.backup?`${state.backup.error||'Automatic recovery is running.'} Checkpoint: ${age(state.backup.checkpoint_at)}. Credentials: ${age(state.backup.credentials_at)}.`:'Recovery starts after the backup key is configured.';
  el('reconnect-panel').hidden=!state.backup?.reconnect;el('reconnect-reason').textContent=state.backup?.reconnect_reason||'Credentials need attention.';
  el('version').textContent=`Launchpad ${(state.release.launchpad||'unknown').slice(0,12)} · Connector ${state.release.adapter||'unknown'}`;
  const update=state.update;el('update').hidden=!update?.candidate;el('rollback').hidden=!update?.rollback;
  el('update-status').textContent=update?.error||`Last release check: ${age(update?.checked_at)}`;
  if(update?.candidate){el('candidate').textContent=`Available: Launchpad ${update.candidate.launchpad.slice(0,12)} · Connector ${update.candidate.adapter}`;el('tool-changes').textContent=Object.entries(update.candidate.tools).map(([k,v])=>`${k}: ${state.release.tools?.[k]||'—'} → ${v}`).join('\n');const notes=update.candidate.notes;el('release-notes').href=typeof notes==='string'&&notes.startsWith('https://github.com/Wauplin/launchpad/')?notes:'https://github.com/Wauplin/launchpad/releases';}
  const login=state.login;if(login?.result==='completed'&&lastLoginResult!=='completed'&&el('controls').open)void loadTools();lastLoginResult=login?.result;el('login-panel').hidden=!login?.tool;
  if(login?.tool){el('login-title').textContent=`Connect ${login.tool}${login.result?' · '+login.result:''}`;el('login-output').textContent=login.output;el('login-links').replaceChildren();for(const match of login.output.matchAll(/https:\/\/[^\s<>\x1b]+/g)){try{const url=new URL(match[0]);const a=document.createElement('a');a.href=url.href;a.target='_blank';a.rel='noreferrer';a.className='button';a.textContent='Open authorization page ↗';el('login-links').append(a);}catch{}}}
  if(toolLoaded)renderTools();
 }catch(error){toast(error.message);}
}
el('open-controls').onclick=()=>{el('controls').showModal();if(state?.ready)loadTools();};el('close-controls').onclick=()=>el('controls').close();
el('create-bucket').onclick=()=>action('create-bucket');el('retry').onclick=refresh;el('create-key').onclick=()=>action('bootstrap-key');el('check-tools').onclick=loadTools;
document.querySelectorAll('[data-action]').forEach(button=>button.onclick=()=>available?el('controls').close():action(button.dataset.action));
el('hf-form').onsubmit=event=>{event.preventDefault();const token=el('hf-token').value;el('hf-token').value='';action('hf-token',{token}).then(()=>loadTools());};
el('login-form').onsubmit=event=>{event.preventDefault();const value=el('login-input').value;el('login-input').value='';action('login-input',{value});};
el('ssh').onclick=async()=>{const value=await action('ssh-key');if(value?.public_key){el('public-key').hidden=false;el('public-key').textContent=(value.fingerprint?value.fingerprint+'\n':'')+value.public_key;publicKey=value.public_key;el('copy-public').hidden=false;}};
el('confirm-rollback').onclick=()=>action('rollback',{confirmed:true});
el('workspace').addEventListener('load',()=>{try{lastRoute=route(el('workspace').contentWindow.location.href);}catch{}});
window.addEventListener('message',event=>{if(event.origin!==location.origin||event.source!==el('workspace').contentWindow||event.data?.type!=='launchpad-route')return;lastRoute=route(event.data.route);history.replaceState({},'',`/_space/shell?route=${encodeURIComponent(lastRoute)}`);});
setInterval(()=>{if(document.visibilityState!=='visible')return;if(el('controls').open||!state?.ready)refresh();try{if(state?.ready){const next=route(el('workspace').contentWindow.location.href);if(next!==lastRoute){lastRoute=next;history.replaceState({},'',`/_space/shell?route=${encodeURIComponent(next)}`);}}}catch{}},2000);
refresh().then(()=>{if(new URLSearchParams(location.search).has('setup')&&state?.ready){el('controls').showModal();loadTools();}});

el('api-tool').onchange=()=>{if(el('api-tool').value!=='opencode')el('api-provider').value=el('api-tool').value==='claude'?'anthropic':'openai';};
el('api-form').onsubmit=event=>{event.preventDefault();const key=el('api-key').value;el('api-key').value='';action('api-key',{tool:el('api-tool').value,provider:el('api-provider').value,key,paid:el('api-paid').checked}).then(()=>loadTools());};
el('ssh-form').onsubmit=event=>{event.preventDefault();const private_key=el('ssh-private').value;el('ssh-private').value='';action('ssh-import',{private_key}).then(value=>{if(value?.public_key){el('public-key').hidden=false;el('public-key').textContent=(value.fingerprint?value.fingerprint+'\n':'')+value.public_key;publicKey=value.public_key;el('copy-public').hidden=false;}});};

el('rotate-key').onclick=()=>action('rotate-key',{confirmed:true});

el('generate-key').onclick=async()=>{const value=await action('generate-key');if(value?.backup_key){el('backup-key').value=value.backup_key;el('generated-key').hidden=false;}};
el('copy-key').onclick=()=>navigator.clipboard.writeText(el('backup-key').value).then(()=>toast('Key copied. Save it as LP_SPACE_BACKUP_KEY in Hub Secrets.')).catch(()=>toast('Clipboard unavailable. Select the generated value and copy it.'));

el('copy-public').onclick=()=>navigator.clipboard.writeText(publicKey||'').then(()=>toast('Public key copied')).catch(()=>toast('Select the public key above and copy it.'));
