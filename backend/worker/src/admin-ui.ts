/** Bolo Office. Split sign-in, app shell hidden until the session is complete. No external assets. */
export const ADMIN_HTML = String.raw`<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="robots" content="noindex, nofollow">
<title>Bolo Office</title>
<script>try{document.documentElement.dataset.theme=localStorage.getItem("bolo_office_theme")||"dark"}catch(e){}</script>
<style>
:root,[data-theme="dark"]{--bg:#090807;--panel:#12110e;--elev:#1a1814;--line:#2c281f;--gold:#e4c27a;--gold2:#f3d48a;--text:#f6f1e6;--mute:#b7ab96;--bad:#f0a394;--ok:#8fceab;--ink:#1a1408;--shadow:0 16px 40px rgba(0,0,0,.35)}
[data-theme="light"]{--bg:#f4f0e8;--panel:#fffcf7;--elev:#fff;--line:#e3d9c6;--gold:#8a6414;--gold2:#6e5010;--text:#1c1812;--mute:#5c5346;--bad:#9d2c22;--ok:#1d6b3a;--ink:#1a1408;--shadow:0 16px 40px rgba(40,30,10,.12)}
*{box-sizing:border-box}html,body{margin:0;min-height:100%;background:var(--bg);color:var(--text);font:16px/1.5 "Segoe UI",system-ui,sans-serif}
[hidden]{display:none !important}
button,input,select{font:inherit;color:inherit}
:focus-visible{outline:2px solid var(--gold2);outline-offset:2px}
#gate{min-height:100vh;display:flex}
.brand{flex:1.08;display:flex;flex-direction:column;justify-content:center;padding:72px 64px;position:relative;overflow:hidden;background:
radial-gradient(900px 480px at 12% -10%,rgba(224,184,90,.22),transparent 55%),linear-gradient(165deg,#221b10 0%,#0c0b09 62%)}
.mark{font-weight:800;font-size:30px;color:var(--gold);letter-spacing:.2px}.mark span{color:var(--text);font-weight:560;margin-left:8px;font-size:18px}
.brand h2{font-size:40px;line-height:1.12;font-weight:650;margin:22px 0 12px;max-width:14ch}
.lede,.facts{color:var(--mute);max-width:42ch}
.facts{margin:18px 0 0;padding:0;list-style:none}
.facts li{margin:0 0 10px;padding-left:16px;position:relative}
.facts li:before{content:"";position:absolute;left:0;top:.65em;width:6px;height:6px;border-radius:99px;background:var(--gold)}
.wave{position:absolute;left:0;right:0;bottom:18px;width:100%;color:rgba(224,184,90,.4)}
.panel{flex:.92;display:flex;align-items:center;justify-content:center;padding:48px 32px;background:#0e0d0b}
#panel{width:100%;max-width:420px}
h1{font-size:28px;line-height:1.2;margin:0 0 8px;font-weight:650}
.k{display:block;color:var(--mute);font-size:12px;font-weight:700;letter-spacing:.06em;text-transform:uppercase;margin:16px 0 6px}
input,select{width:100%;height:48px;background:#0c0b09;border:1px solid var(--line);border-radius:12px;padding:0 14px}
input:focus,select:focus{outline:none;border-color:var(--gold);box-shadow:0 0 0 3px rgba(224,184,90,.28)}
.pw{position:relative}.pw input{padding-right:76px}
.textbtn{position:absolute;right:8px;top:8px;height:32px;border:0;background:transparent;color:var(--gold2);cursor:pointer;padding:0 8px}
.links .textbtn{position:static;height:auto;padding:8px 0}
.btn{width:100%;height:48px;margin-top:18px;border:0;border-radius:12px;background:linear-gradient(180deg,#f3d48a,#d4af37);color:#1a1408;font-weight:750;cursor:pointer}
.btn[disabled]{opacity:.55;cursor:not-allowed}
.btn.ghost{background:transparent;color:var(--gold2);border:1px solid #5c4c22;width:auto;height:40px;margin-top:0;padding:0 14px}
.btn.bad{background:#e07a68;color:#1a100e}
.err{color:var(--bad);margin:12px 0 0}
.hint{color:var(--mute);font-size:13px;margin:8px 0 0}
.codes{display:grid;grid-template-columns:repeat(6,1fr);gap:8px}
.codes input{text-align:center;font-size:22px;font-weight:700;padding:0}
.links{display:flex;justify-content:space-between;gap:12px;margin-top:8px}
#shell{display:grid;grid-template-columns:232px 1fr;min-height:100vh}
#shell.collapsed{grid-template-columns:72px 1fr}
.side{background:var(--panel);border-right:1px solid var(--line);padding:14px 10px;display:flex;flex-direction:column;gap:8px;position:sticky;top:0;height:100vh}
.side .logo{padding:6px 8px 12px}
#nav{display:flex;flex-direction:column;gap:2px;overflow:auto}
#nav button{display:flex;align-items:center;gap:10px;width:100%;text-align:left;background:transparent;border:0;color:var(--mute);border-radius:10px;padding:8px 10px;cursor:pointer;transition:background .18s ease,color .18s ease}
#nav button .i{width:16px;height:16px;flex:none}
#nav button.on{color:var(--text);background:var(--elev);box-shadow:inset 2px 0 0 var(--gold)}
#nav button:hover{color:var(--text);background:var(--elev)}
#shell.collapsed #nav button span,#shell.collapsed .logo span{display:none}
.workspace{min-width:0;display:flex;flex-direction:column}
header{display:flex;align-items:center;gap:10px;padding:12px 18px;border-bottom:1px solid var(--line);background:color-mix(in srgb,var(--bg) 86%,transparent);position:sticky;top:0;z-index:5;backdrop-filter:blur(10px)}
#find{flex:1;max-width:520px;height:38px;border-radius:10px;border:1px solid var(--line);background:var(--panel);color:var(--mute);text-align:left;padding:0 12px;cursor:pointer}
#find kbd{float:right;border:1px solid var(--line);border-radius:6px;padding:1px 6px;font-size:11px}
.logo{font-weight:750;color:var(--gold);font-size:18px}.logo span{color:var(--text);font-weight:500;margin-left:6px;font-size:13px}
.acct{margin-left:auto;position:relative}
#who{display:flex;align-items:center;gap:8px;background:var(--panel);border:1px solid var(--line);color:var(--text);border-radius:999px;padding:4px 10px 4px 4px;cursor:pointer;max-width:280px}
.ava{width:26px;height:26px;border-radius:99px;display:grid;place-items:center;background:var(--gold);color:var(--ink);font-size:12px;font-weight:750}
.badge{font-size:11px;border:1px solid var(--line);border-radius:99px;padding:1px 6px;color:var(--gold2)}
.menu{position:absolute;right:0;top:calc(100% + 8px);background:var(--elev);border:1px solid var(--line);border-radius:12px;min-width:220px;padding:6px;box-shadow:var(--shadow)}
.menu button{display:block;width:100%;text-align:left;background:transparent;border:0;color:var(--text);padding:10px 12px;border-radius:8px;cursor:pointer}
.menu button:hover{background:var(--panel)}
main{max-width:1120px;margin:0 auto;padding:22px 22px 72px;width:100%}
#palette{position:fixed;inset:0;background:rgba(0,0,0,.45);z-index:40;display:grid;align-items:start;justify-content:center;padding-top:12vh}
#palette .box{width:min(560px,92vw);background:var(--elev);border:1px solid var(--line);border-radius:16px;box-shadow:var(--shadow);overflow:hidden}
#palette input{border:0;border-bottom:1px solid var(--line);border-radius:0;height:52px;background:transparent}
#hits{max-height:320px;overflow:auto;padding:6px}
#hits button{display:block;width:100%;text-align:left;background:transparent;border:0;color:var(--text);padding:10px 12px;border-radius:8px;cursor:pointer}
#hits button:hover,#hits button.on{background:var(--panel)}
#drawer{position:fixed;top:0;right:0;height:100%;width:min(460px,100%);background:var(--panel);border-left:1px solid var(--line);z-index:25;transform:translateX(104%);transition:transform .2s ease;overflow:auto;padding:18px 18px 40px;box-shadow:var(--shadow)}
#drawer.open{transform:none}
.tabs{display:flex;gap:4px;margin:12px 0}
.tabs button{background:transparent;border:1px solid transparent;color:var(--mute);border-radius:8px;padding:6px 10px;cursor:pointer}
.tabs button.on{color:var(--text);border-color:var(--line);background:var(--elev)}
.kebabwrap{position:relative}
.kebab{width:32px;height:32px;border-radius:8px;border:1px solid var(--line);background:transparent;color:var(--text);cursor:pointer}
.pop{position:absolute;right:0;top:36px;z-index:6;background:var(--elev);border:1px solid var(--line);border-radius:12px;padding:6px;min-width:180px;box-shadow:var(--shadow)}
.pop .btn{display:block;width:100%;text-align:left;margin:2px 0}
.skel{height:14px;border-radius:8px;background:linear-gradient(90deg,var(--line),var(--elev),var(--line));background-size:200% 100%;animation:bolo-spin 1.2s linear infinite}
.spark{width:100%;height:36px;color:var(--gold);margin-top:8px}
@media (prefers-reduced-motion:reduce){*{animation:none !important;transition:none !important}}
.grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(180px,1fr));gap:14px}
.card{background:var(--panel);border:1px solid var(--line);border-radius:16px;padding:16px 18px}
.stat .k{margin:0}.stat .v{font-size:28px;font-weight:700;margin-top:4px}
h2{font-size:20px;margin:28px 0 12px;font-weight:650}
table{width:100%;border-collapse:collapse}th,td{text-align:left;padding:10px 8px;border-bottom:1px solid var(--line);vertical-align:top}
th{color:var(--mute);font-size:12px;letter-spacing:.05em;text-transform:uppercase}
tr.click{cursor:pointer}tr:hover td{background:color-mix(in srgb,var(--gold) 8%,transparent)}
th{position:sticky;top:0;background:var(--panel);z-index:1}
.tag{display:inline-block;padding:1px 8px;border-radius:99px;font-size:12px;border:1px solid var(--line)}
.tag.paid,.tag.active{color:var(--gold2);border-color:#5c4c22}.tag.revoked,.tag.suspended,.tag.disabled{color:var(--bad)}
.row{display:flex;gap:8px;flex-wrap:wrap;align-items:center}
.empty{color:var(--mute);padding:8px 2px}
.bar{height:8px;background:#2a2418;border-radius:99px;overflow:hidden;margin-top:8px}.bar>i{display:block;height:100%;background:linear-gradient(90deg,var(--gold),var(--gold2))}
.pill{border:1px solid var(--line);border-radius:999px;padding:2px 10px;font-size:12px;color:var(--mute)}
.example{background:#2b230c;color:var(--gold2);border-color:#5a4a17}
dialog{background:var(--panel);color:var(--text);border:1px solid #5c4c22;border-radius:16px;max-width:440px;width:92%;padding:18px}
dialog::backdrop{background:rgba(0,0,0,.62)}
.toast{position:fixed;bottom:18px;right:18px;background:#1c1812;border:1px solid var(--gold);padding:12px 14px;border-radius:12px;display:none;max-width:320px}
.qr{background:#fff;border-radius:12px;padding:8px;display:inline-block;margin:8px 0}
code{font-family:Consolas,ui-monospace,monospace;color:var(--gold2);font-size:13px}
.mute{color:var(--mute)}
.spin{display:inline-block;width:12px;height:12px;border:2px solid var(--line);border-top-color:var(--gold);border-radius:50%;animation:bolo-spin .7s linear infinite;vertical-align:-2px;margin-right:6px}
@keyframes bolo-spin{to{transform:rotate(360deg)}}
@media(max-width:860px){
#gate{flex-direction:column}.panel{order:1;padding:28px 18px 8px}.brand{order:2;padding:8px 22px 28px}
.brand h2,.facts,.wave{display:none}.mark{font-size:22px}
#shell{grid-template-columns:1fr}
.side{position:fixed;z-index:20;transform:translateX(-105%);transition:transform .2s ease;width:240px}
#shell.navopen .side{transform:none}
#who .who-name{display:none}
}
</style></head><body>
<div id="gate">
  <section class="brand">
    <div class="mark">Bolo <span>Office</span></div>
    <h2>Speak anywhere in Windows.</h2>
    <p class="lede">Push-to-talk dictation for Windows, by Kingdom Vision.</p>
    <ul class="facts">
      <li>Hold a hotkey, speak English, Urdu, Roman Urdu, or a mix, and the text lands at the caret.</li>
      <li>Windows 10 and 11. Bolo does not store your audio, and it has no telemetry.</li>
    </ul>
    <svg class="wave" viewBox="0 0 640 120" aria-hidden="true">
      <path d="M0 78c30 0 30-36 70-36s40 36 80 36 40-52 80-52 40 64 80 64 36-44 76-44 40 52 80 52 40-48 80-48 34 28 74 28" fill="none" stroke="currentColor" stroke-width="2"/>
      <path d="M0 96c40 0 40-18 80-18s40 18 80 18 40-28 80-28 40 36 80 36 40-22 80-22 40 26 80 26 40-16 80-16" fill="none" stroke="currentColor" stroke-width="1.25" opacity=".55"/>
    </svg>
  </section>
  <section class="panel"><div id="panel"></div></section>
</div>
<div id="shell" hidden>
<aside class="side">
  <div class="logo">Bolo<span>Office</span></div>
  <nav id="nav" aria-label="Office">
    <button type="button" data-v="dash" class="on"><svg class="i" viewBox="0 0 24 24" aria-hidden="true"><path d="M4 10.5 12 4l8 6.5V20h-6v-5H10v5H4z" fill="none" stroke="currentColor" stroke-width="1.6"/></svg><span>Dashboard</span></button>
    <button type="button" data-v="users"><svg class="i" viewBox="0 0 24 24" aria-hidden="true"><path d="M8 11a3 3 0 1 0 0-6 3 3 0 0 0 0 6Zm8 1a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5ZM3 19c.4-2.4 2.4-4 5-4s4.6 1.6 5 4m1-4c2.2 0 4 1.3 4.5 3.5" fill="none" stroke="currentColor" stroke-width="1.6"/></svg><span>Users</span></button>
    <button type="button" data-v="team" id="navTeam" hidden><svg class="i" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 12a3 3 0 1 0 0-6 3 3 0 0 0 0 6ZM5 19c.5-3 3-5 7-5s6.5 2 7 5" fill="none" stroke="currentColor" stroke-width="1.6"/></svg><span>Team</span></button>
    <button type="button" data-v="devices"><svg class="i" viewBox="0 0 24 24" aria-hidden="true"><rect x="4" y="3" width="16" height="18" rx="2" fill="none" stroke="currentColor" stroke-width="1.6"/></svg><span>Devices</span></button>
    <button type="button" data-v="keys" data-perm="view_keys" hidden><svg class="i" viewBox="0 0 24 24" aria-hidden="true"><path d="M14 7a4 4 0 1 0-3.5 6.6L14 17h2v2h2v-2l1-1-4.4-4.4A4 4 0 0 0 14 7Z" fill="none" stroke="currentColor" stroke-width="1.6"/></svg><span>Keys</span></button>
    <button type="button" data-v="health"><svg class="i" viewBox="0 0 24 24" aria-hidden="true"><path d="M3 12h4l2-5 4 10 2-5h6" fill="none" stroke="currentColor" stroke-width="1.6"/></svg><span>Health</span></button>
    <button type="button" data-v="limits"><svg class="i" viewBox="0 0 24 24" aria-hidden="true"><path d="M4 19V5M4 19h16M8 15v-4m4 4V8m4 7v-2" fill="none" stroke="currentColor" stroke-width="1.6"/></svg><span>Limits</span></button>
    <button type="button" data-v="releases"><svg class="i" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3 4 7v10l8 4 8-4V7l-8-4Zm0 8 8-4M12 11v10M12 11 4 7" fill="none" stroke="currentColor" stroke-width="1.6"/></svg><span>Releases</span></button>
    <button type="button" data-v="feedback" data-perm="manage_feedback" hidden><svg class="i" viewBox="0 0 24 24" aria-hidden="true"><path d="M5 6h14v9H8l-3 3V6Z" fill="none" stroke="currentColor" stroke-width="1.6"/></svg><span>Feedback</span></button>
    <button type="button" data-v="errors" data-perm="manage_feedback" hidden><svg class="i" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 8v5m0 3h.01M12 3 2 20h20L12 3Z" fill="none" stroke="currentColor" stroke-width="1.6"/></svg><span>Errors</span></button>
    <button type="button" data-v="audit"><svg class="i" viewBox="0 0 24 24" aria-hidden="true"><path d="M8 6h11M8 12h11M8 18h11M5 6h.01M5 12h.01M5 18h.01" fill="none" stroke="currentColor" stroke-width="1.6"/></svg><span>Audit log</span></button>
    <button type="button" data-v="settings" data-perm="manage_settings" hidden><svg class="i" viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="3" fill="none" stroke="currentColor" stroke-width="1.6"/><path d="M12 3v2m0 14v2M3 12h2m14 0h2" fill="none" stroke="currentColor" stroke-width="1.6"/></svg><span>Settings</span></button>
  </nav>
  <button type="button" id="collapse" class="btn ghost" style="margin-top:auto">Collapse</button>
</aside>
<div class="workspace">
<header>
  <button type="button" id="find" aria-haspopup="dialog">Search or jump to a page <kbd>Ctrl K</kbd></button>
  <span class="pill example" id="ex" hidden>Example data</span>
  <button type="button" id="theme" class="btn ghost" aria-label="Toggle color theme">Theme</button>
  <div class="acct">
    <button type="button" id="who" aria-haspopup="menu" aria-expanded="false" aria-controls="menu"></button>
    <div id="menu" class="menu" role="menu" hidden>
      <button type="button" id="m2fa" role="menuitem">Email codes</button>
      <button type="button" id="mtotp" role="menuitem">Authenticator</button>
      <button type="button" id="mout" role="menuitem">Sign out</button>
    </div>
  </div>
</header>
<main id="app"></main>
</div>
<div id="palette" hidden role="dialog" aria-label="Command palette"><div class="box"><input id="qpal" placeholder="Jump to a page or find a user" aria-label="Command palette"><div id="hits"></div></div></div>
<aside id="drawer" hidden aria-label="User detail"><button type="button" id="drawerx" class="btn ghost">Close</button><div class="tabs" role="tablist"><button type="button" data-tab="ov" class="on">Overview</button><button type="button" data-tab="dev">Devices</button><button type="button" data-tab="use">Usage</button><button type="button" data-tab="act">Activity</button></div><div id="drawerbody"></div></aside>
<dialog id="dlg"><form method="dialog" id="dform">
  <h3 id="dtitle" style="margin-top:0"></h3>
  <p id="dtext" class="mute"></p>
  <div id="dextra"></div>
  <label class="k" for="dreason">Reason (required, saved to the audit log)</label>
  <input id="dreason" minlength="5" required>
  <label class="k" id="dconfirmwrap" for="dconfirm" hidden>Type DELETE to confirm</label>
  <input id="dconfirm" hidden autocomplete="off">
  <div class="row" style="margin-top:14px;justify-content:flex-end">
    <button class="btn ghost" value="cancel" formnovalidate type="submit">Cancel</button>
    <button class="btn" id="dok" value="ok" type="submit" style="width:auto">Confirm</button>
  </div>
</form></dialog>
<dialog id="authdlg"><form method="dialog" id="authform">
  <h3 id="authtitle" style="margin-top:0">Confirm it's you</h3>
  <p id="authtext" class="mute"></p>
  <label class="k" for="authval" id="authlab">Code</label>
  <input id="authval" inputmode="numeric" autocomplete="one-time-code">
  <div class="row" style="margin-top:14px;justify-content:flex-end">
    <button class="btn ghost" value="cancel" type="submit" style="width:auto">Cancel</button>
    <button class="btn" value="ok" type="submit" style="width:auto">Confirm</button>
  </div>
</form></dialog>
<div class="toast" id="toast" role="status"></div>
</div>
<script>
const $= (s)=>document.querySelector(s);
let T=sessionStorage.getItem("bolo_admin")||"";
let ME=null;
const CFG=window.__BOLO_OFFICE__||{mock:false,apiBase:"/admin/api",home:"/admin"};
const API=()=>CFG.apiBase||"/admin/api";
function h(tag,attrs,...kids){const e=document.createElement(tag);for(const[k,v]of Object.entries(attrs||{})){if(k==="on")for(const[ev,f]of Object.entries(v))e.addEventListener(ev,f);else if(v!==false&&v!=null)e.setAttribute(k,v===true?"":v)}for(const c of kids.flat())if(c!=null&&c!==false)e.append(c instanceof Node?c:document.createTextNode(String(c)));return e}
function toast(m){const t=$("#toast");t.textContent=m;t.style.display="block";setTimeout(()=>t.style.display="none",3500)}
function showGate(){$("#shell").hidden=true;$("#gate").hidden=false}
function showApp(){$("#gate").hidden=true;$("#shell").hidden=false}
async function api(p,o={}){const r=await fetch(API()+p,{...o,headers:{"content-type":"application/json",...(T?{authorization:"Bearer "+T}:{})}});const j=await r.json().catch(()=>({}));if(r.status===401&&j.code==="admin_unauthenticated"){T="";sessionStorage.removeItem("bolo_admin");showGate();login();throw new Error("signed out")}if(!r.ok)throw Object.assign(new Error(j.error||"Sign-in failed."),{code:j.code});return j}
const allowed=(a)=>ME&&ME.permissions&&ME.permissions.includes(a);
function codeBoxes(){
  const wrap=h("div",{class:"codes",role:"group","aria-label":"6-digit email code"});
  for(let i=0;i<6;i++){
    const inp=h("input",{inputmode:"numeric",maxlength:"1","aria-label":"Digit "+(i+1),autocomplete:i===0?"one-time-code":"off"});
    inp.addEventListener("input",()=>{inp.value=(inp.value||"").replace(/\D/g,"").slice(-1);if(inp.value&&inp.nextElementSibling)inp.nextElementSibling.focus()});
    inp.addEventListener("keydown",(e)=>{if(e.key==="Backspace"&&!inp.value&&inp.previousElementSibling){inp.previousElementSibling.focus();inp.previousElementSibling.value=""}});
    inp.addEventListener("paste",(e)=>{const t=(e.clipboardData.getData("text")||"").replace(/\D/g,"").slice(0,6);if(!t)return;e.preventDefault();[...wrap.children].forEach((el,idx)=>{el.value=t[idx]||""});(wrap.children[Math.min(t.length,5)]).focus();wrap.dispatchEvent(new Event("input",{bubbles:true}))});
    wrap.append(inp);
  }
  return wrap;
}
function codeValue(wrap){return [...wrap.querySelectorAll("input")].map(i=>i.value).join("")}
function login(){
  showGate();
  const err=h("p",{class:"err",role:"alert",hidden:true});
  const email=h("input",{id:"em",type:"email",autocomplete:"username",required:true,placeholder:"name@company.com"});
  const pw=h("input",{id:"pw",type:"password",autocomplete:"current-password",required:true});
  const eye=h("button",{type:"button",class:"textbtn","aria-label":"Show password","aria-pressed":"false"},"Show");
  eye.addEventListener("click",()=>{const show=pw.type==="password";pw.type=show?"text":"password";eye.textContent=show?"Hide":"Show";eye.setAttribute("aria-label",show?"Hide password":"Show password");eye.setAttribute("aria-pressed",show?"true":"false")});
  const btn=h("button",{class:"btn",type:"submit"},"Continue");
  const form=h("form",{novalidate:true},
    h("h1",{},"Office sign-in"),
    h("p",{class:"mute"},CFG.mock?"Example sign-in for this local preview.":"Use your Office email and password."),
    h("label",{class:"k",for:"em"},"Email"),email,
    h("label",{class:"k",for:"pw"},"Password"),
    h("div",{class:"pw"},pw,eye)
  );
  if(CFG.mock){
    form.append(h("label",{class:"k",for:"cd"},"Authenticator code"),h("input",{id:"cd",inputmode:"numeric",autocomplete:"one-time-code",placeholder:"6-digit code"}));
  }
  form.append(err,btn);
  form.addEventListener("submit",async(e)=>{
    e.preventDefault(); err.hidden=true; btn.disabled=true; btn.textContent="Checking…";
    try{
      const body={email:email.value.trim(),password:pw.value};
      if(CFG.mock)body.code=($("#cd")&&$("#cd").value)||"";
      const j=await api("/login",{method:"POST",body:JSON.stringify(body)});
      if(j.needsEmailCode)return step2(j,email.value.trim());
      if(!j.token)throw new Error("Sign-in failed.");
      T=j.token; sessionStorage.setItem("bolo_admin",T);
      if(j.needsEnrollment)return enrollView(j.enrollment);
      boot();
    }catch(ex){err.textContent=ex.message||"Sign-in failed.";err.hidden=false;btn.disabled=false;btn.textContent="Continue"}
  });
  $("#panel").replaceChildren(form); email.focus();
}
function step2(j,email){
  showGate();
  const err=h("p",{class:"err",role:"alert",hidden:true});
  const boxes=codeBoxes();
  const spin=h("p",{class:"hint",hidden:true},h("span",{class:"spin","aria-hidden":"true"},""),"Checking the code");
  const resend=h("button",{type:"button",class:"textbtn",id:"resend"},"Resend code");
  const back=h("button",{type:"button",class:"textbtn"},"Back");
  let challenge=j.challenge, timer, busy=false;
  function arm(sec){clearInterval(timer);let left=sec|0;const tick=()=>{if(left<=0){resend.disabled=false;resend.textContent="Resend code";clearInterval(timer);return}resend.disabled=true;const m=Math.floor(left/60),s=String(left%60).padStart(2,"0");resend.textContent="Resend code in "+m+":"+s;left-=1};tick();timer=setInterval(tick,1000)}
  arm(j.resendAfterSeconds||60);
  resend.addEventListener("click",async()=>{err.hidden=true;try{const r=await api("/login/resend",{method:"POST",body:JSON.stringify({challenge})});arm(r.resendAfterSeconds||60)}catch(ex){err.textContent=ex.message||"Sign-in failed.";err.hidden=false}});
  back.addEventListener("click",()=>{clearInterval(timer);login()});
  async function submitCode(){
    if(busy)return;
    const code=codeValue(boxes);
    if(code.length<6)return;
    busy=true; err.hidden=true; spin.hidden=false;
    boxes.querySelectorAll("input").forEach(i=>{i.disabled=true});
    try{
      const r=await api("/login/code",{method:"POST",body:JSON.stringify({challenge,code})});
      if(!r.token)throw new Error("no token");
      clearInterval(timer); T=r.token; sessionStorage.setItem("bolo_admin",T); boot();
    }catch(_){
      err.textContent="That code is wrong or expired. Try again.";
      err.hidden=false; spin.hidden=true; busy=false;
      boxes.querySelectorAll("input").forEach(i=>{i.disabled=false;i.value=""});
      const first=boxes.querySelector("input"); if(first)first.focus();
    }
  }
  boxes.addEventListener("input",()=>{ if(codeValue(boxes).length===6) submitCode(); });
  boxes.addEventListener("keydown",(e)=>{ if(e.key==="Enter"){ e.preventDefault(); submitCode(); } });
  const form=h("form",{novalidate:true},
    h("h1",{},"Enter your code"),
    h("p",{class:"mute"},"We emailed a 6-digit code to "+email+". It works once and expires in 10 minutes."),
    h("span",{class:"k",id:"codelab"},"Email code"),
    boxes, err,
    j.totpAvailable?h("p",{class:"hint"},"You can type an authenticator code here instead."):"",
    spin, h("div",{class:"links"},resend,back)
  );
  form.addEventListener("submit",(e)=>{ e.preventDefault(); submitCode(); });
  $("#panel").replaceChildren(form); boxes.querySelector("input").focus();
}
function enrollBlock(en){
  const kids=[h("p",{class:"mute"},"Add this account in your authenticator app, then enter a code. This is optional if email codes are on.")];
  if(en&&en.qrSvg){const box=h("div",{class:"qr"});box.innerHTML=en.qrSvg;kids.push(box)}
  if(en)kids.push(h("p",{},h("a",{href:en.otpauthUrl,style:"color:var(--gold2)"},"Open authenticator setup link")),h("p",{class:"mute"},"Or enter this secret manually:"),h("code",{},en.secret||""));
  return kids;
}
function enrollView(en){
  showGate();
  const box=h("form",{class:"card"},h("h1",{},"Authenticator"),...enrollBlock(en),
    h("label",{class:"k",for:"ecd"},"Authenticator code"),h("input",{id:"ecd",inputmode:"numeric",autocomplete:"one-time-code"}),
    h("button",{class:"btn",type:"button",on:{click:async()=>{try{await api("/enroll",{method:"POST",body:JSON.stringify({code:$("#ecd").value})});toast("Authenticator ready");boot()}catch(e){toast(e.message)}}}},"Confirm"));
  $("#panel").replaceChildren(box);
}
async function boot(){
  if(!T)return login();
  showApp();
  try{ME=await api("/me")}catch(e){if(e.message!=="signed out"){T="";sessionStorage.removeItem("bolo_admin");login()}return}
  if(ME.needsEnrollment){try{const en=await api("/enroll");return enrollView(en)}catch(e){toast(e.message);return}}
  $("#who").replaceChildren(h("span",{class:"ava","aria-hidden":"true"},(ME.email||"?").slice(0,1).toUpperCase()),h("span",{class:"who-name"},ME.email),h("span",{class:"badge"},ME.role));
  $("#navTeam").hidden=!allowed("manage_team");
  document.querySelectorAll("nav button[data-perm]").forEach(b=>{b.hidden=!(ME.permissions||[]).includes(b.dataset.perm)});
  $("#m2fa").textContent=ME.email2fa?"Email codes are on":"Turn on email codes";
  $("#mtotp").hidden=!!ME.totpEnrolled;
  show(location.hash.slice(1)||"dash");
}
document.querySelectorAll("nav button[data-v]").forEach(b=>b.onclick=()=>show(b.dataset.v));
$("#who").onclick=(e)=>{e.stopPropagation();const open=$("#menu").hidden;$("#menu").hidden=!open;$("#who").setAttribute("aria-expanded",open?"true":"false")};
document.addEventListener("click",()=>{$("#menu").hidden=true;$("#who").setAttribute("aria-expanded","false")});
$("#mout").onclick=()=>{T="";sessionStorage.removeItem("bolo_admin");ME=null;$("#menu").hidden=true;login()};
$("#mtotp").onclick=async()=>{try{const en=await api("/enroll");enrollView(en.ok?en:en)}catch(e){if(e.code==="already_enrolled")toast("Authenticator is already set up.");else toast(e.message)}};
$("#m2fa").onclick=async()=>{
  $("#menu").hidden=true;
  if(ME&&ME.email2fa){toast("Email codes are on. An owner can turn them off from Team.");return}
  try{
    const j=await api("/me/email-2fa",{method:"POST",body:"{}"});
    if(j.email2fa){ME.email2fa=true;$("#m2fa").textContent="Email codes are on";toast("Email codes are on");return}
    const code=await askSecret("Turn on email codes","We emailed you a 6-digit code. Enter it to turn email codes on.","email");
    if(!code)return;
    await api("/me/email-2fa/confirm",{method:"POST",body:JSON.stringify({code})});
    ME.email2fa=true; $("#m2fa").textContent="Email codes are on"; toast("Email codes are on");
  }catch(e){toast(e.message)}
};
function closeDrawer(){const d=$("#drawer");if(!d)return;d.classList.remove("open");d.hidden=true}
function show(v){location.hash=v;$("#app").replaceChildren(h("div",{class:"card"},h("div",{class:"skel"})));document.querySelectorAll("#nav button[data-v]").forEach(b=>b.classList.toggle("on",b.dataset.v===v.split("/")[0]));if(!String(v).startsWith("user/"))closeDrawer();if(String(v).startsWith("user/")){users();return userView(v.slice(5))}({dash,users,audit,team,health,devices,limits,releases,keys,errors,feedback,settings}[v]||dash)()}
const stat=(k,v,extra)=>h("div",{class:"card stat"},h("div",{class:"k"},k),h("div",{class:"v"},v),extra||"");
function askSecret(title,text,mode){
  return new Promise((resolve)=>{
    $("#authtitle").textContent=title; $("#authtext").textContent=text;
    const input=$("#authval"); input.value=""; input.type=mode==="password"?"password":"text";
    $("#authlab").textContent=mode==="password"?"Password":mode==="totp"?"Authenticator code":"Email code";
    const d=$("#authdlg"); d.showModal();
    d.onclose=()=>{resolve(d.returnValue==="ok"?input.value.trim():"")};
  });
}
async function ensureRecent(){
  if(ME&&new Date(ME.recentAuthUntil)>new Date())return true;
  try{
    if(ME.email2fa)await api("/reauth/send",{method:"POST",body:"{}"});
    const mode=ME.email2fa?"email":ME.totpEnrolled?"totp":"password";
    const text=mode==="email"?"We emailed you a 6-digit code.":mode==="totp"?"Enter the code from your authenticator app.":"Enter your password to continue.";
    const secret=await askSecret("Confirm it's you",text,mode);
    if(!secret)return false;
    const body=mode==="password"?{password:secret}:{code:secret};
    const j=await api("/reauth",{method:"POST",body:JSON.stringify(body)});
    ME.recentAuthUntil=j.recentAuthUntil; return true;
  }catch(e){toast(e.message);return false}
}
function confirmAction(title,text,extra,run){const del=/^Delete/.test(title);$("#dtitle").textContent=title;$("#dtext").textContent=text;$("#dextra").replaceChildren(extra||"");$("#dreason").value="";$("#dconfirm").value="";$("#dconfirmwrap").hidden=!del;$("#dconfirm").hidden=!del;const d=$("#dlg");d.showModal();d.onclose=async()=>{if(d.returnValue!=="ok")return;if(del&&$("#dconfirm").value!=="DELETE"){toast("Type DELETE to confirm.");return}if(!(await ensureRecent()))return;try{await run($("#dreason").value);toast("Done. Recorded in the audit log.");show(location.hash.slice(1)||"dash")}catch(e){toast(e.message)}}}
function empty(text){return h("p",{class:"empty"},text)}
function stop(fn){return (e)=>{e.stopPropagation();fn()}}
function go(title,text,path,body){return stop(()=>confirmAction(title,text,null,(r)=>api(path,{method:"POST",body:JSON.stringify(Object.assign({reason:r},body||{}))})))}
function btn(label,cls,fn){return h("button",{type:"button",class:"btn "+(cls||"ghost"),style:"width:auto",on:{click:fn}},label)}
function save(path,body,done){return stop(async()=>{try{await api(path,{method:"POST",body:JSON.stringify(Object.assign({reason:"updated in Office"},body))});toast("Saved");if(done)done();else show(location.hash.slice(1)||"dash")}catch(e){toast(e.message)}})}
async function health(){
  const d=await api("/ops/health");
  const bits=[h("h2",{style:"margin-top:0"},"Speech health")];
  if(d.announcement)bits.push(h("div",{class:"card"},d.announcement));
  if(d.alert)bits.push(h("div",{class:"card row",style:"justify-content:space-between"},h("div",{},"Online speech failures are high."),btn("Dismiss for an hour","ghost",go("Dismiss alert","Hide this banner for one hour.","/ops/health",{}))));
  if(!d.samples)bits.push(empty("No speech samples in the last hour."));
  else bits.push(h("div",{class:"grid"},stat("Samples",d.samples),stat("Online failures",d.onlineFail+" / "+d.online),stat("p50",d.p50==null?"—":d.p50+" ms"),stat("p95",d.p95==null?"—":d.p95+" ms")));
  $("#app").replaceChildren(...bits);
}
async function devices(){
  const d=await api("/ops/devices");
  const q=h("input",{placeholder:"Filter by install or version","aria-label":"Filter devices",style:"max-width:340px"});
  const box=h("div",{class:"card"});
  const drawer=h("div",{class:"card",hidden:true});
  function draw(list){
    const rows=list.map(x=>h("tr",{class:"click",on:{click:()=>{drawer.hidden=false;drawer.replaceChildren(h("b",{},x.id),h("p",{class:"mute"},(x.version||"No version")+" · "+(x.engine||"engine unknown")+" · "+(x.lastSeenAt||"never seen")))}}},
      h("td",{},x.id),h("td",{},x.version||"—"),h("td",{},x.lastSeenAt?new Date(x.lastSeenAt).toLocaleString():"—"),h("td",{},x.engine||"—"),h("td",{},x.blocked?"blocked":x.status),
      h("td",{},h("div",{class:"row"},
        btn(x.blocked?"Unblock":"Block",x.blocked?"ghost":"bad",go(x.blocked?"Unblock device":"Block device","This device's managed access changes now.","/ops/devices/"+x.id+"/block",{blocked:!x.blocked})),
        btn("Sign out","bad",go("Sign out device","Its access tokens stop now.","/installs/"+x.id+"/revoke",{})),
        btn("Renew key","ghost",go("Renew access key","The old key stops and the device reconnects.","/installs/"+x.id+"/reissue",{}))
      ))
    ));
    box.replaceChildren(rows.length?h("table",{},h("tr",{},h("th",{},"Install"),h("th",{},"Version"),h("th",{},"Last seen"),h("th",{},"Engine"),h("th",{},"State"),h("th",{},"Actions")),...rows):empty("No devices match."));
  }
  q.oninput=()=>{const n=q.value.toLowerCase();draw((d.devices||[]).filter(x=>(x.id+" "+(x.version||"")).toLowerCase().includes(n)))};
  draw(d.devices||[]);
  $("#app").replaceChildren(h("h2",{style:"margin-top:0"},"Devices"),q,box,drawer);
}
async function limits(){
  const d=await api("/ops/limits");
  const daily=h("input",{id:"limd",type:"number",value:String(d.freeMinutesPerDay)});
  const warn=h("input",{id:"limw",type:"number",value:String(d.softWarnRatio),step:"0.05"});
  $("#app").replaceChildren(h("h2",{style:"margin-top:0"},"Usage limits"),h("div",{class:"card"},
    h("label",{class:"k",for:"limd"},"Free minutes per day"),daily,
    h("label",{class:"k",for:"limw"},"Soft warning ratio (0 to 1)"),warn,
    btn("Save limits","",(e)=>{e.stopPropagation();api("/ops/limits",{method:"POST",body:JSON.stringify({freeMinutesPerDay:Number(daily.value),softWarnRatio:Number(warn.value),reason:"updated in Office"})}).then(()=>{toast("Saved");limits()}).catch(er=>toast(er.message))})
  ));
}
async function releases(){
  const d=await api("/ops/releases"); const c=d.channels||{};
  const web=h("input",{id:"rw",value:c.website||"",placeholder:"Website version"});
  const up=h("input",{id:"ru",value:c.updater||"",placeholder:"Updater version"});
  const store=h("input",{id:"rs",value:c.store||"",placeholder:"Store version"});
  const pin=h("input",{id:"rp",value:c.updater||"",placeholder:"Version to pin"});
  function post(body){return (e)=>{e.stopPropagation();api("/ops/releases",{method:"POST",body:JSON.stringify(Object.assign({reason:"updated in Office"},body))}).then(()=>{toast("Saved");releases()}).catch(er=>toast(er.message))}}
  $("#app").replaceChildren(h("h2",{style:"margin-top:0"},"Releases"),h("div",{class:"card"},
    h("p",{},d.paused?"Updates are paused on the pinned file.":"Updates follow the website file."),
    h("label",{class:"k"},"Website"),web,h("label",{class:"k"},"Updater"),up,h("label",{class:"k"},"Store"),store,
    h("div",{class:"row",style:"margin-top:12px"},btn("Save versions","",(e)=>{e.stopPropagation();Promise.all([["website",web.value],["updater",up.value],["store",store.value]].map(([channel,version])=>api("/ops/releases",{method:"POST",body:JSON.stringify({channel,version,reason:"updated in Office"})}))).then(()=>{toast("Saved");releases()}).catch(er=>toast(er.message))})),
    h("label",{class:"k"},"Pin or roll back to"),pin,
    h("div",{class:"row"},btn(d.paused?"Resume updates":"Pause updates",d.paused?"":"bad",post(d.paused?{paused:false}:{paused:true,pinned:{version:pin.value||up.value}})),btn("Roll back","ghost",post({paused:true,pinned:{version:pin.value}})))
  ));
}
async function keys(){
  const d=await api("/ops/keys");
  const pool=h("input",{id:"kpool",placeholder:"Pool id"});
  const secret=h("input",{id:"ksec",type:"password",placeholder:"Key value, shown once as a fingerprint",autocomplete:"off"});
  const cap=h("input",{id:"kcap",type:"number",placeholder:"Daily minute cap",value:"0"});
  const rows=(d.keys||[]).map(k=>h("tr",{},
    h("td",{},k.poolId),h("td",{},h("code",{},k.fingerprint)),h("td",{},k.status),h("td",{},k.dailyMinuteCap?k.minutesToday+" / "+k.dailyMinuteCap:"no cap"),
    h("td",{},h("div",{class:"row"},
      btn("Primary","ghost",go("Make primary","Other live keys in this pool become backups.","/ops/keys",{op:"primary",id:k.id})),
      btn(k.status==="backup"?"Enable":"Disable","ghost",go(k.status==="backup"?"Enable key":"Disable key","Disabled keys stay stored but are not used.","/ops/keys",{op:k.status==="backup"?"enable":"disable",id:k.id})),
      btn("Rotate","ghost",stop(async()=>{const next=await askSecret("Rotate key","Paste the new key. It will not be shown again.","password");if(!next)return;confirmAction("Rotate key","The previous secret is replaced.",null,(r)=>api("/ops/keys",{method:"POST",body:JSON.stringify({op:"rotate",id:k.id,secret:next,reason:r})}))})),
      btn("Revoke","bad",go("Revoke key","This key is retired and the secret is dropped.","/ops/keys",{op:"retire",id:k.id}))
    ))
  ));
  $("#app").replaceChildren(h("h2",{style:"margin-top:0"},"Provider keys"),
    h("div",{class:"card"},h("div",{class:"grid"},h("div",{},h("label",{class:"k"},"Pool"),pool),h("div",{},h("label",{class:"k"},"Key"),secret),h("div",{},h("label",{class:"k"},"Daily cap"),cap)),btn("Add key","",(e)=>{e.stopPropagation();api("/ops/keys",{method:"POST",body:JSON.stringify({poolId:pool.value,secret:secret.value,dailyMinuteCap:Number(cap.value)||0,reason:"add pool key"})}).then(()=>{toast("Key added");secret.value="";keys()}).catch(er=>toast(er.message))})),
    h("div",{class:"card"},rows.length?h("table",{},h("tr",{},h("th",{},"Pool"),h("th",{},"Fingerprint"),h("th",{},"State"),h("th",{},"Today"),h("th",{},"Actions")),...rows):empty("No keys yet."))
  );
}
async function errors(){
  const d=await api("/ops/errors");
  const q=h("input",{placeholder:"Filter reports","aria-label":"Filter reports",style:"max-width:340px"});
  const box=h("div",{class:"card"});
  function draw(list){
    const rows=list.map(r=>h("tr",{},h("td",{},r.kind),h("td",{},r.version||"—"),h("td",{},r.line),h("td",{},r.fixedIn||(r.note||"—")),
      h("td",{},h("div",{class:"row"},
        btn(r.fixedIn?"Reopen":"Resolved","ghost",go(r.fixedIn?"Reopen report":"Mark resolved","Set the release this was fixed in, or clear it.", "/ops/errors/"+r.id+"/fixed", r.fixedIn?{version:""}:{version:r.version||"next"})),
        btn("Note","ghost",stop(async()=>{const note=await askSecret("Note","Assignee or note for this report.","text");if(!note)return;await api("/ops/errors/"+r.id+"/fixed",{method:"POST",body:JSON.stringify({op:"note",note,reason:"updated in Office"})});toast("Saved");errors()})),
        btn("Delete","bad",go("Delete report","Remove this report from Office.","/ops/errors/"+r.id+"/fixed",{op:"delete"}))
      ))));
    box.replaceChildren(rows.length?h("table",{},h("tr",{},h("th",{},"Kind"),h("th",{},"Version"),h("th",{},"Line"),h("th",{},"Status"),h("th",{},"Actions")),...rows):empty("No reports match."));
  }
  q.oninput=()=>{const n=q.value.toLowerCase();draw((d.reports||[]).filter(r=>(r.kind+" "+r.line).toLowerCase().includes(n)))};
  draw(d.reports||[]);
  $("#app").replaceChildren(h("h2",{style:"margin-top:0"},"Error reports"),q,box);
}
async function feedback(){
  const d=await api("/ops/feedback");
  const q=h("input",{placeholder:"Filter feedback","aria-label":"Filter feedback",style:"max-width:340px"});
  const box=h("div",{class:"card"});
  const text=h("input",{id:"fbt",placeholder:"Note from the contact form or the app"});
  const kind=h("select",{id:"fbk"},...["contact","accuracy","bug","billing","other"].map(v=>h("option",{value:v},v)));
  function draw(list){
    const rows=list.map(r=>h("tr",{},h("td",{},r.source),h("td",{},r.issueType),h("td",{},r.text),h("td",{},r.resolved?"resolved":"open"),
      h("td",{},h("div",{class:"row"},
        btn(r.resolved?"Reopen":"Resolve","ghost",save("/ops/feedback",{id:r.id,resolved:!r.resolved})),
        btn("Assign","ghost",stop(async()=>{const assignee=await askSecret("Assign","Who should handle this?","text");if(!assignee)return;await api("/ops/feedback",{method:"POST",body:JSON.stringify({id:r.id,assignee,reason:"updated in Office"})});toast("Saved");feedback()})),
        btn("Reply","ghost",stop(async()=>{const note=await askSecret("Reply note","Saved on the item. It is not emailed.","text");if(!note)return;await api("/ops/feedback",{method:"POST",body:JSON.stringify({id:r.id,note,reason:"updated in Office"})});toast("Saved");feedback()})),
        btn("Delete","bad",go("Delete feedback","Remove this item.","/ops/feedback",{id:r.id,op:"delete"}))
      ))));
    box.replaceChildren(rows.length?h("table",{},h("tr",{},h("th",{},"From"),h("th",{},"Type"),h("th",{},"Note"),h("th",{},"State"),h("th",{},"Actions")),...rows):empty("No feedback matches."));
  }
  q.oninput=()=>{const n=q.value.toLowerCase();draw((d.items||[]).filter(r=>(r.text+" "+r.issueType).toLowerCase().includes(n)))};
  draw(d.items||[]);
  $("#app").replaceChildren(h("h2",{style:"margin-top:0"},"Feedback"),h("div",{class:"card row"},kind,text,btn("Add","",(e)=>{e.stopPropagation();api("/ops/feedback",{method:"POST",body:JSON.stringify({source:"website",issueType:kind.value,text:text.value,reason:"added in Office"})}).then(()=>{toast("Saved");feedback()}).catch(er=>toast(er.message))})),q,box);
}
async function settings(){
  const d=await api("/ops/settings");
  const days=h("input",{id:"setd",type:"number",value:String(d.retentionDays)});
  const pol=h("input",{id:"setp",value:d.privacyPolicyVersion||"",placeholder:"Privacy policy version"});
  const ann=h("input",{id:"seta",value:d.announcement||"",placeholder:"Announcement shown in Office"});
  const bits=[h("h2",{style:"margin-top:0"},"Settings and privacy"),h("div",{class:"card"},
    h("label",{class:"k",for:"setd"},"Keep Office records (days)"),days,
    h("label",{class:"k",for:"setp"},"Privacy policy version"),pol,
    h("label",{class:"k",for:"seta"},"Announcement"),ann,
    btn("Save","",(e)=>{e.stopPropagation();api("/ops/settings",{method:"POST",body:JSON.stringify({retentionDays:Number(days.value),privacyPolicyVersion:pol.value,announcement:ann.value,reason:"updated in Office"})}).then(()=>{toast("Saved");settings()}).catch(er=>toast(er.message))})
  )];
  if(allowed("owner_settings")){
    const ips=h("input",{id:"setip",value:(d.ipAllowlist||[]).join(", "),placeholder:"Blank allows every IP"});
    const sessions=h("div",{class:"card"},h("h3",{style:"margin-top:0"},"Office sessions"),(d.sessions||[]).length?(d.sessions||[]).map(s=>h("div",{class:"row",style:"justify-content:space-between"},h("span",{},s.email+" · "+s.role),btn("Revoke","bad",go("Revoke session","That browser must sign in again.","/ops/settings",{op:"revoke-session",id:s.id})))):empty("No active sessions."));
    bits.push(h("div",{class:"card"},
      h("h3",{style:"margin-top:0"},"Office security"),
      btn(d.requireEmail2fa?"Email codes required":"Email codes optional","ghost",(e)=>{e.stopPropagation();api("/ops/settings",{method:"POST",body:JSON.stringify({requireEmail2fa:!d.requireEmail2fa,reason:"updated in Office"})}).then(()=>{toast("Saved");settings()}).catch(er=>toast(er.message))}),
      h("label",{class:"k",for:"setip"},"IP allowlist"),ips,
      btn("Save allowlist","",(e)=>{e.stopPropagation();api("/ops/settings",{method:"POST",body:JSON.stringify({ipAllowlist:ips.value.split(",").map(s=>s.trim()).filter(Boolean),reason:"updated in Office"})}).then(()=>{toast("Saved");settings()}).catch(er=>toast(er.message))})
    ),sessions);
  }
  $("#app").replaceChildren(...bits);
}
function spark(){return h("svg",{class:"spark",viewBox:"0 0 120 36",role:"img","aria-label":"No trend is stored"},h("title",{},"No trend is stored"),h("path",{d:"M4 28 H116",fill:"none",stroke:"currentColor","stroke-dasharray":"4 5"}))}
async function dash(){
  const d=await api("/dashboard"); $("#ex").hidden=!d.exampleData; const app=$("#app");
  const health=await api("/ops/health").catch(()=>null);
  const log=await api("/audit").catch(()=>({entries:[]}));
  const alert=d.healthAlert?h("div",{class:"card"},"Online speech failures are high. Open Health."):null;
  const pools=d.pools&&d.pools.length?h("table",{},h("tr",{},h("th",{},"Pool"),h("th",{},"Provider"),h("th",{},"Key fingerprint"),h("th",{},"Devices"),h("th",{},"State")),d.pools.map(p=>h("tr",{},h("td",{},p.label),h("td",{},p.provider),h("td",{},h("code",{},p.fingerprint)),h("td",{},p.installs),h("td",{},h("span",{class:"tag "+(p.full?"revoked":"active")},p.full?"full":p.status))))):h("p",{class:"empty"},"No provider pools yet.");
  app.replaceChildren(alert||"",
    h("div",{class:"row"},health&&health.alert?h("span",{class:"tag revoked"},"Speech failures high"):h("span",{class:"tag active"},health&&health.samples?"Speech steady":"No speech samples")),
    h("div",{class:"grid"},stat("Users",d.users,h("div",{},h("div",{class:"mute"},d.freeUsers+" free · "+d.paidUsers+" paid"),spark())),stat("Active devices",d.activeInstalls,spark()),stat("Minutes today",d.todayMinutes,h("div",{},h("div",{class:"bar"},h("i",{style:"width:"+Math.max(0,Math.min(100,d.budgetUsedPct))+"%"})),h("div",{class:"mute"},d.budgetUsedPct+"% of "+d.dailyBudgetMinutes+" min"))),stat("This month",d.monthMinutes+" min",spark()),stat("Errors today",d.errorsToday,spark())),
    h("h2",{},"Recent activity"),h("div",{class:"card"},(log.entries&&log.entries.length)?auditTable(log.entries.slice(0,8)):empty("No audit entries yet.")),
    h("h2",{},"Safety controls"),
    h("div",{class:"card row",style:"justify-content:space-between"},h("div",{},h("b",{},"Kill switch "),h("span",{class:"tag "+(d.killSwitch?"revoked":"active")},d.killSwitch?"On — managed dictation paused":"Off")),
      h("button",{class:"btn "+(d.killSwitch?"":"bad"),disabled:!allowed("kill_switch")||undefined,style:"width:auto",on:{click:()=>confirmAction(d.killSwitch?"Turn kill switch off":"Turn kill switch on","This affects every user immediately.",null,(r)=>api("/kill-switch",{method:"POST",body:JSON.stringify({on:!d.killSwitch,reason:r})}))}},d.killSwitch?"Resume managed dictation":"Pause managed dictation")),
    h("h2",{},"Provider pools"),h("div",{class:"card"},pools)
  );
}
async function users(){
  const app=$("#app"); const q=h("input",{placeholder:"Search by email",style:"max-width:340px","aria-label":"Search users"}); const box=h("div",{class:"card"});
  async function load(){const j=await api("/users?q="+encodeURIComponent(q.value));
    box.replaceChildren(j.users&&j.users.length?h("table",{},h("tr",{},h("th",{},"Email"),h("th",{},"Plan"),h("th",{},"Status"),h("th",{},"Devices"),h("th",{},"Today"),h("th",{},"Actions")),j.users.map(u=>h("tr",{},
      h("td",{},h("button",{type:"button",class:"btn ghost",style:"width:auto",on:{click:()=>show("user/"+u.userId)}},u.email)),
      h("td",{},h("span",{class:"tag "+u.plan},u.plan)),h("td",{},h("span",{class:"tag "+u.status},u.status)),h("td",{},u.activeInstalls+"/"+u.installs),h("td",{},u.dailyMinutes+" min"),
      h("td",{},h("div",{class:"row"},
        btn(u.status==="suspended"?"Reactivate":"Suspend","ghost",go(u.status==="suspended"?"Reactivate user":"Suspend user","Managed dictation stops while they are suspended.","/users/"+u.userId+"/suspend",{suspended:u.status!=="suspended"})),
        btn("Revoke tokens","bad",go("Revoke tokens","Every device for this user must sign in again.","/users/"+u.userId+"/revoke-all",{})),
        btn("Reset usage","ghost",go("Reset usage","Clears this user's recorded minutes.","/users/"+u.userId+"/reset-usage",{})),
        btn("Delete","bad",go("Delete user","Their profile is marked deleted and their devices are revoked.","/ops/users/"+u.userId+"/delete",{}))
      ))))):h("p",{class:"empty"},q.value?"No users match that search.":"No users yet."));
  }
  q.oninput=load; app.replaceChildren(h("h2",{style:"margin-top:0"},"Users"),h("div",{class:"row",style:"margin-bottom:12px"},q),box); load();
}
async function userView(id){
  const {user:u}=await api("/users/"+id);
  const act=(label,perm,cls,fn)=>h("button",{class:"btn "+cls,disabled:!allowed(perm)||undefined,style:"width:auto",on:{click:fn}},label);
  const devs=u.devices&&u.devices.length?h("table",{},h("tr",{},h("th",{},"Device"),h("th",{},"Status"),h("th",{},"Access key"),h("th",{},"Pool"),h("th",{},"Actions")),
    u.devices.map(dv=>h("tr",{},h("td",{},dv.deviceName),h("td",{},h("span",{class:"tag "+dv.status},dv.status)),
      h("td",{},dv.token?h("code",{},dv.token.tokenPublicId):"—"),
      h("td",{},dv.pool?dv.pool.label:"—"),
      h("td",{},h("div",{class:"row"},
        act("Reissue","reissue","ghost",()=>confirmAction("Reissue access key","The old key stops now. This device reconnects on its own.",null,(r)=>api("/installs/"+dv.installId+"/reissue",{method:"POST",body:JSON.stringify({reason:r})}))),
        act("Revoke","revoke","bad",()=>confirmAction("Revoke device","This device must sign in again.",null,(r)=>api("/installs/"+dv.installId+"/revoke",{method:"POST",body:JSON.stringify({reason:r})})))
      ))))):h("p",{class:"empty"},"No devices.");
  const panes={ov:h("div",{}),dev:h("div",{hidden:true}),use:h("div",{hidden:true}),act:h("div",{hidden:true},(u.recentAudit&&u.recentAudit.length)?auditTable(u.recentAudit):empty("No activity for this user."))};
  document.querySelectorAll("#drawer .tabs button").forEach(b=>b.onclick=()=>{document.querySelectorAll("#drawer .tabs button").forEach(x=>x.classList.toggle("on",x===b));Object.entries(panes).forEach(([k,el])=>{el.hidden=k!==b.dataset.tab})});
  panes.ov.replaceChildren(h("div",{class:"row"},h("span",{class:"tag "+u.plan},u.plan),h("span",{class:"tag "+u.status},u.status),
      act(u.plan==="paid"?"Set to free":"Set to paid","set_plan","ghost",()=>confirmAction("Change plan","Switch this user.",null,(r)=>api("/users/"+id+"/plan",{method:"POST",body:JSON.stringify({plan:u.plan==="paid"?"free":"paid",reason:r})}))),
      act(u.status==="suspended"?"Unsuspend":"Suspend","suspend","ghost",()=>confirmAction("Change suspension","Managed dictation stops on their devices while suspended.",null,(r)=>api("/users/"+id+"/suspend",{method:"POST",body:JSON.stringify({suspended:u.status!=="suspended",reason:r})}))),
      act("Revoke all tokens","revoke","bad",()=>confirmAction("Revoke all tokens","Every device for this user must sign in again.",null,(r)=>api("/users/"+id+"/revoke-all",{method:"POST",body:JSON.stringify({reason:r})}))),
      act("Reset usage","set_plan","ghost",()=>confirmAction("Reset usage","Clears this user's recorded minutes.",null,(r)=>api("/users/"+id+"/reset-usage",{method:"POST",body:JSON.stringify({reason:r})}))),
      act("Delete user","suspend","bad",()=>confirmAction("Delete user","Their profile is marked deleted and their devices are revoked.",null,(r)=>api("/ops/users/"+id+"/delete",{method:"POST",body:JSON.stringify({reason:r})})))
  ));
  panes.dev.replaceChildren(h("div",{class:"card"},devs));
  panes.use.replaceChildren(h("div",{class:"grid"},stat("Today",u.usage.dailyMinutes+" min"),stat("This month",u.usage.monthlyMinutes+" min"),stat("Lifetime",u.usage.lifetimeMinutes+" min")));
  const drawer=$("#drawer"); drawer.hidden=false; requestAnimationFrame(()=>drawer.classList.add("open"));
  $("#drawerbody").replaceChildren(h("h2",{},u.email),panes.ov,panes.dev,panes.use,panes.act);
}
function auditTable(rows){
  if(!rows||!rows.length)return h("p",{class:"empty"},"No audit entries yet.");
  return h("table",{},h("tr",{},h("th",{},"When"),h("th",{},"Admin"),h("th",{},"Action"),h("th",{},"Target"),h("th",{},"Reason"),h("th",{},"Result")),
    rows.map(a=>h("tr",{},h("td",{},new Date(a.at).toLocaleString()),h("td",{},a.adminEmail+" ("+a.role+")"),h("td",{},a.action),h("td",{},h("code",{},a.target)),h("td",{},a.reason||"—"),h("td",{},h("span",{class:"tag "+(a.result==="ok"?"active":"revoked")},a.result)))));
}
async function audit(){const j=await api("/audit");$("#app").replaceChildren(h("h2",{style:"margin-top:0"},"Audit log"),h("p",{class:"mute"},"Append-only. Sign-ins, changes, and denied attempts are kept."),h("div",{class:"card"},auditTable(j.entries)))}
async function team(){
  if(!allowed("manage_team")){toast("Owner only");return show("dash")}
  const app=$("#app"), box=h("div",{class:"card"}), created=h("div",{});
  async function load(){
    const j=await api("/team");
    box.replaceChildren(j.staff&&j.staff.length?h("table",{},
      h("tr",{},h("th",{},"Email"),h("th",{},"Name"),h("th",{},"Role"),h("th",{},"Status"),h("th",{},"Email codes"),h("th",{},"Actions")),
      ...j.staff.map(st=>h("tr",{},
        h("td",{},st.email),h("td",{},st.label),h("td",{},st.role),
        h("td",{},h("span",{class:"tag "+(st.enabled?"active":"disabled")},st.enabled?"enabled":"disabled")),
        h("td",{},st.email2fa?"on":"off"),
        h("td",{},h("div",{class:"row"},
          h("select",{id:"role-"+st.email,style:"width:auto"},...["owner","admin","support","viewer"].map(r=>h("option",{value:r,selected:r===st.role},r))),
          h("button",{class:"btn ghost",style:"width:auto",on:{click:()=>{const sel=document.getElementById("role-"+st.email);confirmAction("Change role","Set "+st.email+" to "+sel.value+".",null,(r)=>api("/team/"+encodeURIComponent(st.email)+"/role",{method:"POST",body:JSON.stringify({role:sel.value,reason:r})}))}}},"Save role"),
          h("button",{class:"btn ghost",style:"width:auto",on:{click:()=>confirmAction("Resend invite","Records the invite. Office does not email their password.",null,(r)=>api("/team/"+encodeURIComponent(st.email)+"/invite",{method:"POST",body:JSON.stringify({reason:r})}))}},"Resend invite"),
          h("button",{class:"btn ghost",style:"width:auto",on:{click:()=>confirmAction(st.email2fa?"Turn email codes off":"Turn email codes on",(st.email2fa?"Stop":"Start")+" emailing sign-in codes to "+st.email+".",null,(r)=>api("/team/"+encodeURIComponent(st.email)+"/email-2fa",{method:"POST",body:JSON.stringify({enabled:!st.email2fa,reason:r})}))}},st.email2fa?"Codes off":"Codes on"),
          h("button",{class:"btn "+(st.enabled?"bad":"ghost"),style:"width:auto",on:{click:()=>confirmAction(st.enabled?"Remove account":"Enable account",(st.enabled?"Disable":"Enable")+" Office access for "+st.email+".",null,(r)=>api("/team/"+encodeURIComponent(st.email)+"/enabled",{method:"POST",body:JSON.stringify({enabled:!st.enabled,reason:r})}))}},st.enabled?"Remove":"Enable")
        ))
      ))):h("p",{class:"empty"},"No staff accounts yet."));
  }
  const form=h("div",{class:"card"},
    h("h3",{style:"margin-top:0"},"Add staff"),
    h("div",{class:"grid"},
      h("div",{},h("label",{class:"k",for:"te"},"Email"),h("input",{id:"te",type:"email"})),
      h("div",{},h("label",{class:"k",for:"tn"},"Name"),h("input",{id:"tn"})),
      h("div",{},h("label",{class:"k",for:"tr"},"Role"),h("select",{id:"tr"},h("option",{value:"admin"},"admin"),h("option",{value:"support"},"support"),h("option",{value:"viewer"},"viewer"),h("option",{value:"owner"},"owner"))),
      h("div",{},h("label",{class:"k",for:"tp"},"Password (14+ characters)"),h("input",{id:"tp",type:"password"}))
    ),
    h("button",{class:"btn",style:"width:auto",on:{click:()=>confirmAction("Create staff account","They can sign in with the password. Email codes stay off until turned on.",null,async(r)=>{
      const j=await api("/team",{method:"POST",body:JSON.stringify({email:$("#te").value,label:$("#tn").value,role:$("#tr").value,password:$("#tp").value,reason:r})});
      created.replaceChildren(h("div",{class:"card"},h("h3",{style:"margin-top:0"},"Optional authenticator"),h("p",{class:"mute"},"Share this once with "+j.staff.email+" if they want an authenticator as well as email codes."),...enrollBlock(j.enrollment)));
      $("#te").value="";$("#tn").value="";$("#tp").value="";load();
    })}},"Create account")
  );
  app.replaceChildren(h("h2",{style:"margin-top:0"},"Team"),h("p",{class:"mute"},"Owners can create staff, change access, and turn email codes on or off. A person can also turn their own email codes on."),form,created,h("h2",{},"Staff"),box);
  load();
}
$("#theme").onclick=()=>{const next=document.documentElement.dataset.theme==="light"?"dark":"light";document.documentElement.dataset.theme=next;try{localStorage.setItem("bolo_office_theme",next)}catch(e){}};
$("#collapse").onclick=()=>{$("#shell").classList.toggle(matchMedia("(max-width:860px)").matches?"navopen":"collapsed")};
$("#drawerx").onclick=closeDrawer;
function paintPalette(q){
  const mods=[{v:"dash",t:"Dashboard"},{v:"users",t:"Users"},{v:"devices",t:"Devices"},{v:"health",t:"Health"},{v:"limits",t:"Limits"},{v:"releases",t:"Releases"},{v:"keys",t:"Keys"},{v:"feedback",t:"Feedback"},{v:"errors",t:"Errors"},{v:"audit",t:"Audit log"},{v:"settings",t:"Settings"},{v:"team",t:"Team"}].filter(m=>!q||m.t.toLowerCase().includes(q));
  const box=$("#hits");
  box.replaceChildren(...mods.map(m=>h("button",{type:"button",on:{click:()=>{$("#palette").hidden=true;show(m.v)}}},m.t)));
  if(q&&q.length>1)api("/users?q="+encodeURIComponent(q)).then(j=>{(j.users||[]).slice(0,8).forEach(u=>box.append(h("button",{type:"button",on:{click:()=>{$("#palette").hidden=true;show("user/"+u.userId)}}},u.email)))}).catch(()=>{});
}
function openPalette(){$("#palette").hidden=false;$("#qpal").value="";paintPalette("");$("#qpal").focus()}
$("#find").onclick=openPalette;
$("#qpal").addEventListener("input",()=>paintPalette($("#qpal").value.trim().toLowerCase()));
$("#palette").addEventListener("click",(e)=>{if(e.target.id==="palette")$("#palette").hidden=true});
document.addEventListener("keydown",(e)=>{if((e.metaKey||e.ctrlKey)&&e.key.toLowerCase()==="k"){e.preventDefault();openPalette()}if(e.key==="Escape"){$("#palette").hidden=true;closeDrawer()}});
boot().catch(()=>{});
</script></body></html>`;
