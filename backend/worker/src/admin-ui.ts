/** Bolo Office. Split sign-in, app shell hidden until the session is complete. No external assets. */
export const ADMIN_HTML = String.raw`<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="robots" content="noindex, nofollow">
<title>Bolo Office</title>
<style>
:root{--bg:#0c0b09;--panel:#17150f;--line:#3a3224;--gold:#e0b85a;--gold2:#f3d48a;--text:#f6f1e6;--mute:#cbbfa8;--bad:#ffb4a8;--ok:#9dceb0}
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
header{display:flex;align-items:center;gap:16px;padding:14px 22px;border-bottom:1px solid var(--line);background:#100e0b;position:sticky;top:0;z-index:3}
.logo{font-weight:800;color:var(--gold);font-size:20px}.logo span{color:var(--text);font-weight:500;margin-left:6px;font-size:14px}
nav{display:flex;gap:4px;flex-wrap:wrap}
nav button{background:transparent;border:1px solid transparent;color:var(--mute);border-radius:999px;padding:6px 12px;cursor:pointer}
nav button.on{color:var(--text);background:#241c10;border-color:var(--line)}
.acct{margin-left:auto;position:relative}
#who{background:#17150f;border:1px solid var(--line);color:var(--text);border-radius:999px;padding:6px 12px;cursor:pointer;max-width:280px}
.menu{position:absolute;right:0;top:calc(100% + 8px);background:#1c1812;border:1px solid var(--line);border-radius:12px;min-width:240px;padding:6px;box-shadow:0 16px 40px rgba(0,0,0,.35)}
.menu button{display:block;width:100%;text-align:left;background:transparent;border:0;color:var(--text);padding:10px 12px;border-radius:8px;cursor:pointer}
.menu button:hover{background:#2a2418}
main{max-width:1100px;margin:0 auto;padding:28px 22px 64px}
.grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(180px,1fr));gap:14px}
.card{background:var(--panel);border:1px solid var(--line);border-radius:16px;padding:16px 18px}
.stat .k{margin:0}.stat .v{font-size:28px;font-weight:700;margin-top:4px}
h2{font-size:20px;margin:28px 0 12px;font-weight:650}
table{width:100%;border-collapse:collapse}th,td{text-align:left;padding:10px 8px;border-bottom:1px solid var(--line);vertical-align:top}
th{color:var(--mute);font-size:12px;letter-spacing:.05em;text-transform:uppercase}
tr.click{cursor:pointer}tr.click:hover{background:#221c12}
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
@media(max-width:860px){
#gate{flex-direction:column}.panel{order:1;padding:28px 18px 8px}.brand{order:2;padding:8px 22px 28px}
.brand h2,.facts,.wave{display:none}.mark{font-size:22px}
header{flex-wrap:wrap}nav{order:3;width:100%}
th:nth-child(n+5),td:nth-child(n+5){display:none}
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
<header>
  <div class="logo">Bolo<span>Office</span></div>
  <span class="pill example" id="ex" hidden>Example data</span>
  <nav id="nav" aria-label="Office">
    <button type="button" data-v="dash" class="on">Dashboard</button>
    <button type="button" data-v="users">Users</button>
    <button type="button" data-v="team" id="navTeam" hidden>Team</button>
    <button type="button" data-v="audit">Audit log</button>
  </nav>
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
<dialog id="dlg"><form method="dialog" id="dform">
  <h3 id="dtitle" style="margin-top:0"></h3>
  <p id="dtext" class="mute"></p>
  <div id="dextra"></div>
  <label class="k" for="dreason">Reason (required, saved to the audit log)</label>
  <input id="dreason" minlength="5" required>
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
    inp.addEventListener("paste",(e)=>{const t=(e.clipboardData.getData("text")||"").replace(/\D/g,"").slice(0,6);if(!t)return;e.preventDefault();[...wrap.children].forEach((el,idx)=>{el.value=t[idx]||""});(wrap.children[Math.min(t.length,5)]).focus()});
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
  const resend=h("button",{type:"button",class:"textbtn",id:"resend"},"Resend code");
  const back=h("button",{type:"button",class:"textbtn"},"Back");
  const btn=h("button",{class:"btn",type:"submit"},"Sign in");
  let challenge=j.challenge, timer;
  function arm(sec){clearInterval(timer);let left=sec|0;const tick=()=>{if(left<=0){resend.disabled=false;resend.textContent="Resend code";clearInterval(timer);return}resend.disabled=true;const m=Math.floor(left/60),s=String(left%60).padStart(2,"0");resend.textContent="Resend code in "+m+":"+s;left-=1};tick();timer=setInterval(tick,1000)}
  arm(j.resendAfterSeconds||60);
  resend.addEventListener("click",async()=>{err.hidden=true;try{const r=await api("/login/resend",{method:"POST",body:JSON.stringify({challenge})});arm(r.resendAfterSeconds||60)}catch(ex){err.textContent=ex.message||"Sign-in failed.";err.hidden=false}});
  back.addEventListener("click",()=>{clearInterval(timer);login()});
  const form=h("form",{novalidate:true},
    h("h1",{},"Enter your code"),
    h("p",{class:"mute"},"We emailed a 6-digit code to "+email+". It works once and expires in 10 minutes."),
    h("span",{class:"k",id:"codelab"},"Email code"),
    boxes,
    j.totpAvailable?h("p",{class:"hint"},"You can type an authenticator code here instead."):"",
    err,btn,h("div",{class:"links"},resend,back)
  );
  form.addEventListener("submit",async(e)=>{
    e.preventDefault(); const code=codeValue(boxes);
    if(code.length<6){err.textContent="Enter all 6 digits.";err.hidden=false;return}
    err.hidden=true; btn.disabled=true; btn.textContent="Signing in…";
    try{
      const r=await api("/login/code",{method:"POST",body:JSON.stringify({challenge,code})});
      if(!r.token)throw new Error("Sign-in failed.");
      clearInterval(timer); T=r.token; sessionStorage.setItem("bolo_admin",T); boot();
    }catch(ex){err.textContent=ex.message||"Sign-in failed.";err.hidden=false;btn.disabled=false;btn.textContent="Sign in"}
  });
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
  $("#who").textContent=ME.email+" · "+ME.role;
  $("#navTeam").hidden=!allowed("manage_team");
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
function show(v){location.hash=v;document.querySelectorAll("nav button[data-v]").forEach(b=>b.classList.toggle("on",b.dataset.v===v.split("/")[0]));if(v.startsWith("user/"))return userView(v.slice(5));({dash,users,audit,team}[v]||dash)()}
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
function confirmAction(title,text,extra,run){$("#dtitle").textContent=title;$("#dtext").textContent=text;$("#dextra").replaceChildren(extra||"");$("#dreason").value="";const d=$("#dlg");d.showModal();d.onclose=async()=>{if(d.returnValue!=="ok")return;if(!(await ensureRecent()))return;try{await run($("#dreason").value);toast("Done. Recorded in the audit log.");show(location.hash.slice(1))}catch(e){toast(e.message)}}}
async function dash(){
  const d=await api("/dashboard"); $("#ex").hidden=!d.exampleData; const app=$("#app");
  const pools=d.pools&&d.pools.length?h("table",{},h("tr",{},h("th",{},"Pool"),h("th",{},"Provider"),h("th",{},"Key fingerprint"),h("th",{},"Devices"),h("th",{},"State")),d.pools.map(p=>h("tr",{},h("td",{},p.label),h("td",{},p.provider),h("td",{},h("code",{},p.fingerprint)),h("td",{},p.installs),h("td",{},h("span",{class:"tag "+(p.full?"revoked":"active")},p.full?"full":p.status))))):h("p",{class:"empty"},"No provider pools yet.");
  app.replaceChildren(
    h("div",{class:"grid"},stat("Users",d.users,h("div",{class:"mute"},d.freeUsers+" free · "+d.paidUsers+" paid")),stat("Active devices",d.activeInstalls),stat("Minutes today",d.todayMinutes,h("div",{},h("div",{class:"bar"},h("i",{style:"width:"+d.budgetUsedPct+"%"})),h("div",{class:"mute"},d.budgetUsedPct+"% of "+d.dailyBudgetMinutes+" min"))),stat("This month",d.monthMinutes+" min"),stat("Errors today",d.errorsToday)),
    h("h2",{},"Safety controls"),
    h("div",{class:"card row",style:"justify-content:space-between"},h("div",{},h("b",{},"Kill switch "),h("span",{class:"tag "+(d.killSwitch?"revoked":"active")},d.killSwitch?"On — managed dictation paused":"Off")),
      h("button",{class:"btn "+(d.killSwitch?"":"bad"),disabled:!allowed("kill_switch")||undefined,style:"width:auto",on:{click:()=>confirmAction(d.killSwitch?"Turn kill switch off":"Turn kill switch on","This affects every user immediately.",null,(r)=>api("/kill-switch",{method:"POST",body:JSON.stringify({on:!d.killSwitch,reason:r})}))}},d.killSwitch?"Resume managed dictation":"Pause managed dictation")),
    h("h2",{},"Provider pools"),h("div",{class:"card"},pools)
  );
}
async function users(){
  const app=$("#app"); const q=h("input",{placeholder:"Search by email",style:"max-width:340px","aria-label":"Search users"}); const box=h("div",{class:"card"});
  async function load(){const j=await api("/users?q="+encodeURIComponent(q.value));
    box.replaceChildren(j.users&&j.users.length?h("table",{},h("tr",{},h("th",{},"Email"),h("th",{},"Plan"),h("th",{},"Status"),h("th",{},"Devices"),h("th",{},"Today"),h("th",{},"Month")),j.users.map(u=>h("tr",{class:"click",on:{click:()=>show("user/"+u.userId)}},h("td",{},u.email),h("td",{},h("span",{class:"tag "+u.plan},u.plan)),h("td",{},h("span",{class:"tag "+u.status},u.status)),h("td",{},u.activeInstalls+"/"+u.installs),h("td",{},u.dailyMinutes+" min"),h("td",{},u.monthlyMinutes+" min")))):h("p",{class:"empty"},q.value?"No users match that search.":"No users yet."));
  }
  q.oninput=load; app.replaceChildren(h("h2",{style:"margin-top:0"},"Users"),h("div",{class:"row",style:"margin-bottom:12px"},q),box); load();
}
async function userView(id){
  const {user:u}=await api("/users/"+id); const app=$("#app");
  const act=(label,perm,cls,fn)=>h("button",{class:"btn "+cls,disabled:!allowed(perm)||undefined,style:"width:auto",on:{click:fn}},label);
  const devs=u.devices&&u.devices.length?h("table",{},h("tr",{},h("th",{},"Device"),h("th",{},"Status"),h("th",{},"Access key"),h("th",{},"Pool"),h("th",{},"Actions")),
    u.devices.map(dv=>h("tr",{},h("td",{},dv.deviceName),h("td",{},h("span",{class:"tag "+dv.status},dv.status)),
      h("td",{},dv.token?h("code",{},dv.token.tokenPublicId):"—"),
      h("td",{},dv.pool?dv.pool.label:"—"),
      h("td",{},h("div",{class:"row"},
        act("Reissue","reissue","ghost",()=>confirmAction("Reissue access key","The old key stops now. This device reconnects on its own.",null,(r)=>api("/installs/"+dv.installId+"/reissue",{method:"POST",body:JSON.stringify({reason:r})}))),
        act("Revoke","revoke","bad",()=>confirmAction("Revoke device","This device must sign in again.",null,(r)=>api("/installs/"+dv.installId+"/revoke",{method:"POST",body:JSON.stringify({reason:r})})))
      ))))):h("p",{class:"empty"},"No devices.");
  app.replaceChildren(h("button",{class:"btn ghost",on:{click:()=>show("users")}},"All users"),h("h2",{},u.email),
    h("div",{class:"row"},h("span",{class:"tag "+u.plan},u.plan),h("span",{class:"tag "+u.status},u.status),
      act(u.plan==="paid"?"Set to free":"Set to paid","set_plan","ghost",()=>confirmAction("Change plan","Switch this user.",null,(r)=>api("/users/"+id+"/plan",{method:"POST",body:JSON.stringify({plan:u.plan==="paid"?"free":"paid",reason:r})}))),
      act(u.status==="suspended"?"Unsuspend":"Suspend","suspend","ghost",()=>confirmAction("Change suspension","Managed dictation stops on their devices while suspended.",null,(r)=>api("/users/"+id+"/suspend",{method:"POST",body:JSON.stringify({suspended:u.status!=="suspended",reason:r})})))
    ),
    h("div",{class:"grid",style:"margin-top:14px"},stat("Today",u.usage.dailyMinutes+" min"),stat("This month",u.usage.monthlyMinutes+" min"),stat("Lifetime",u.usage.lifetimeMinutes+" min")),
    h("h2",{},"Devices"),h("div",{class:"card"},devs)
  );
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
          h("button",{class:"btn ghost",on:{click:()=>confirmAction(st.email2fa?"Turn email codes off":"Turn email codes on",(st.email2fa?"Stop":"Start")+" emailing sign-in codes to "+st.email+".",null,(r)=>api("/team/"+encodeURIComponent(st.email)+"/email-2fa",{method:"POST",body:JSON.stringify({enabled:!st.email2fa,reason:r})}))}},st.email2fa?"Turn off":"Turn on"),
          h("button",{class:"btn "+(st.enabled?"bad":"ghost"),on:{click:()=>confirmAction(st.enabled?"Disable account":"Enable account",(st.enabled?"Disable":"Enable")+" Office access for "+st.email+".",null,(r)=>api("/team/"+encodeURIComponent(st.email)+"/enabled",{method:"POST",body:JSON.stringify({enabled:!st.enabled,reason:r})}))}},st.enabled?"Disable":"Enable")
        ))
      ))):h("p",{class:"empty"},"No staff accounts yet."));
  }
  const form=h("div",{class:"card"},
    h("h3",{style:"margin-top:0"},"Add staff"),
    h("div",{class:"grid"},
      h("div",{},h("label",{class:"k",for:"te"},"Email"),h("input",{id:"te",type:"email"})),
      h("div",{},h("label",{class:"k",for:"tn"},"Name"),h("input",{id:"tn"})),
      h("div",{},h("label",{class:"k",for:"tr"},"Role"),h("select",{id:"tr"},h("option",{value:"support"},"support"),h("option",{value:"viewer"},"viewer"),h("option",{value:"owner"},"owner"))),
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
boot().catch(()=>{});
</script></body></html>`;
