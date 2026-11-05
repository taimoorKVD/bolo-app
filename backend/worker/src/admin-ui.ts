/** Self-contained admin portal (served by the same Worker at /admin). Black & gold Bolo styling. */
export const ADMIN_HTML = String.raw`<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Bolo Office</title>
<style>
:root{--bg:#0b0b0c;--panel:#141416;--line:#2a2722;--gold:#d4af37;--gold2:#f1d27a;--text:#f3efe6;--mute:#a59f92;--bad:#e5715f;--ok:#7fcf8b}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--text);font:15px/1.5 "Segoe UI",system-ui,sans-serif}
header{display:flex;align-items:center;gap:14px;padding:14px 22px;border-bottom:1px solid var(--line);background:#0e0e10;position:sticky;top:0;z-index:2}
.logo{font-weight:800;font-size:20px;letter-spacing:.5px;color:var(--gold)}.logo span{color:var(--text);font-weight:500;margin-left:6px;font-size:14px}
.pill{border:1px solid var(--line);border-radius:999px;padding:2px 10px;font-size:12px;color:var(--mute)}
.example{background:#2b230c;color:var(--gold2);border-color:#5a4a17}
nav{margin-left:auto;display:flex;gap:6px;flex-wrap:wrap}nav button{background:none;border:1px solid transparent;color:var(--mute);padding:6px 12px;border-radius:8px;cursor:pointer;font:inherit}
nav button.on{color:var(--gold);border-color:var(--line);background:var(--panel)}
main{max-width:1180px;margin:0 auto;padding:22px}
.grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(190px,1fr));gap:14px}
.card{background:var(--panel);border:1px solid var(--line);border-radius:14px;padding:16px}
.k{color:var(--mute);font-size:12px;text-transform:uppercase;letter-spacing:.08em}.v{font-size:28px;font-weight:700;margin-top:4px}
.bar{height:8px;background:#222;border-radius:99px;overflow:hidden;margin-top:10px}.bar>i{display:block;height:100%;background:linear-gradient(90deg,var(--gold),var(--gold2))}
h2{font-size:18px;margin:26px 0 12px;color:var(--gold2)}
table{width:100%;border-collapse:collapse}th,td{text-align:left;padding:10px 8px;border-bottom:1px solid var(--line);vertical-align:top}th{color:var(--mute);font-weight:600;font-size:12px;text-transform:uppercase}
tr.click{cursor:pointer}tr.click:hover{background:#1a1916}
.btn{background:var(--gold);color:#111;border:0;border-radius:9px;padding:8px 14px;font-weight:700;cursor:pointer;font:inherit;font-weight:700}
.btn.ghost{background:none;color:var(--gold);border:1px solid #5a4a17}.btn.bad{background:var(--bad);color:#111}
.btn[disabled]{opacity:.35;cursor:not-allowed}
input,select{background:#0f0f11;border:1px solid var(--line);color:var(--text);border-radius:9px;padding:9px 11px;font:inherit;width:100%}
.login{max-width:420px;margin:60px auto}.row{display:flex;gap:10px;flex-wrap:wrap;align-items:center}
.tag{display:inline-block;padding:1px 8px;border-radius:99px;font-size:12px;border:1px solid var(--line)}
.tag.paid{color:var(--gold);border-color:#5a4a17}.tag.active{color:var(--ok)}.tag.revoked,.tag.suspended,.tag.disabled{color:var(--bad)}.tag.reissue_pending{color:var(--gold2)}
code{font-family:Consolas,monospace;color:var(--gold2);font-size:13px}.mute{color:var(--mute)}
dialog{background:var(--panel);color:var(--text);border:1px solid #5a4a17;border-radius:14px;max-width:440px;width:92%}dialog::backdrop{background:#000a}
.toast{position:fixed;bottom:20px;right:20px;background:var(--panel);border:1px solid var(--gold);padding:10px 16px;border-radius:10px;display:none}
.qr{width:180px;height:180px;background:#fff;border-radius:12px;padding:8px;margin:12px 0}
@media(max-width:700px){th:nth-child(n+5),td:nth-child(n+5){display:none}header{flex-wrap:wrap}}
</style></head><body>
<header><div class="logo">Bolo<span>Office</span></div><span class="pill example" id="ex" hidden>Example data</span>
<nav id="nav" hidden>
<button data-v="dash" class="on">Dashboard</button>
<button data-v="users">Users</button>
<button data-v="team" id="navTeam" hidden>Team</button>
<button data-v="audit">Audit log</button>
<button id="who" class="pill"></button>
</nav></header>
<main id="app"></main>
<dialog id="dlg"><form method="dialog" id="dform"><h3 id="dtitle" style="margin-top:0;color:var(--gold2)"></h3><p id="dtext" class="mute"></p>
<div id="dextra"></div><label class="k">Reason (required, saved to audit log)</label><input id="dreason" minlength="5" required placeholder="e.g. User reported key stopped working">
<div class="row" style="margin-top:14px;justify-content:flex-end"><button class="btn ghost" value="cancel" formnovalidate>Cancel</button><button class="btn" id="dok" value="ok">Confirm</button></div></form></dialog>
<div class="toast" id="toast"></div>
<script>
const $= (s)=>document.querySelector(s);let T=sessionStorage.getItem("bolo_admin")||"";let ME=null;
const CFG=window.__BOLO_OFFICE__||{mock:false};
function h(tag,attrs,...kids){const e=document.createElement(tag);for(const[k,v]of Object.entries(attrs||{})){if(k==="on")for(const[ev,f]of Object.entries(v))e.addEventListener(ev,f);else if(v!==false&&v!=null)e.setAttribute(k,v===true?"":v)}for(const c of kids.flat())e.append(c instanceof Node?c:document.createTextNode(c==null?"":String(c)));return e}
function toast(m){const t=$("#toast");t.textContent=m;t.style.display="block";setTimeout(()=>t.style.display="none",3500)}
async function api(p,o={}){const r=await fetch("/office/api"+p,{...o,headers:{"content-type":"application/json",...(T?{authorization:"Bearer "+T}:{})}});const j=await r.json().catch(()=>({}));if(r.status===401&&j.code==="admin_unauthenticated"){T="";sessionStorage.removeItem("bolo_admin");login();throw new Error("signed out")}if(!r.ok)throw Object.assign(new Error(j.error||"Failed"),{code:j.code});return j}
const allowed=(a)=>ME&&ME.permissions.includes(a);
function enrollBlock(en){
  const kids=[
    h("p",{class:"mute"},"Add this account in your authenticator app (Google Authenticator, Microsoft Authenticator, etc.).")
  ];
  if(en&&en.qrSvg){
    const box=h("div",{class:"qr",style:"background:#fff;border-radius:12px;padding:8px;display:inline-block;margin:10px 0"});
    box.innerHTML=en.qrSvg;
    kids.push(box);
  }
  kids.push(
    h("p",{},h("a",{href:en.otpauthUrl,style:"color:var(--gold)"},"Open authenticator setup link")),
    h("p",{class:"mute"},"Or enter this secret manually:"),
    h("code",{},en.secret||""),
    h("p",{class:"mute",style:"font-size:12px;word-break:break-all;margin-top:8px"},en.otpauthUrl||"")
  );
  return kids;
}
function login(){$("#nav").hidden=true;$("#ex").hidden=true;const app=$("#app");
app.replaceChildren(h("div",{class:"card login"},
h("h2",{style:"margin-top:0"},"Office sign-in"),
h("p",{class:"mute"},"Sign in with your Office email, password, and authenticator code."),
h("label",{class:"k"},"Email"),h("input",{id:"em",autocomplete:"username"}),h("div",{style:"height:10px"}),
h("label",{class:"k"},"Password"),h("input",{id:"pw",type:"password",autocomplete:"current-password"}),h("div",{style:"height:10px"}),
h("label",{class:"k"},"Authenticator code"),h("input",{id:"cd",inputmode:"numeric",autocomplete:"one-time-code",placeholder:"6-digit code"}),
h("button",{class:"btn",style:"margin-top:14px",on:{click:async()=>{try{const j=await api("/login",{method:"POST",body:JSON.stringify({email:$("#em").value,password:$("#pw").value,code:$("#cd").value})});T=j.token;sessionStorage.setItem("bolo_admin",T);if(j.needsEnrollment){ME={email:j.email,role:j.role,permissions:[],needsEnrollment:true};return enrollView(j.enrollment)}boot()}catch(e){toast(e.message)}}}},"Sign in")
))}
function enrollView(en){ $("#nav").hidden=true; const app=$("#app");
const box=h("div",{class:"card login"},h("h2",{style:"margin-top:0"},"Set up authenticator"),
h("p",{class:"mute"},"Set up your authenticator app with the secret below, then enter the 6-digit code to finish."),
...(en?enrollBlock(en):[]),
h("div",{style:"height:12px"}),h("label",{class:"k"},"Authenticator code"),h("input",{id:"ecd",inputmode:"numeric"}),
h("button",{class:"btn",style:"margin-top:12px",on:{click:async()=>{try{await api("/enroll",{method:"POST",body:JSON.stringify({code:$("#ecd").value})});toast("Authenticator ready");boot()}catch(e){toast(e.message)}}}},"Confirm and continue"));
app.replaceChildren(box)}
async function boot(){if(!T)return login();ME=await api("/me");if(ME.needsEnrollment){const en=await api("/enroll");return enrollView(en)}
$("#nav").hidden=false;$("#who").textContent=ME.email+" · "+ME.role;$("#navTeam").hidden=!allowed("manage_team");show(location.hash.slice(1)||"dash")}
document.querySelectorAll("nav button[data-v]").forEach(b=>b.onclick=()=>show(b.dataset.v));
function show(v){location.hash=v;document.querySelectorAll("nav button[data-v]").forEach(b=>b.classList.toggle("on",b.dataset.v===v.split("/")[0]));if(v.startsWith("user/"))return userView(v.slice(5));({dash,users,audit,team}[v]||dash)()}
const stat=(k,v,extra)=>h("div",{class:"card"},h("div",{class:"k"},k),h("div",{class:"v"},v),extra||"");
async function ensureRecent(){if(new Date(ME.recentAuthUntil)>new Date())return true;const c=prompt("For safety, enter your authenticator code to continue:");if(!c)return false;try{const j=await api("/reauth",{method:"POST",body:JSON.stringify({code:c})});ME.recentAuthUntil=j.recentAuthUntil;return true}catch(e){toast(e.message);return false}}
function confirmAction(title,text,extra,run){$("#dtitle").textContent=title;$("#dtext").textContent=text;$("#dextra").replaceChildren(extra||"");$("#dreason").value="";const d=$("#dlg");d.showModal();d.onclose=async()=>{if(d.returnValue!=="ok")return;if(!(await ensureRecent()))return;try{await run($("#dreason").value);toast("Done · recorded in audit log");show(location.hash.slice(1))}catch(e){toast(e.message)}}}
async function dash(){const d=await api("/dashboard");$("#ex").hidden=!d.exampleData;const app=$("#app");
app.replaceChildren(h("div",{class:"grid"},stat("Users",d.users,h("div",{class:"mute"},d.freeUsers+" free · "+d.paidUsers+" paid")),stat("Active devices",d.activeInstalls),stat("Minutes today",d.todayMinutes,h("div",{},h("div",{class:"bar"},h("i",{style:"width:"+d.budgetUsedPct+"%"})),h("div",{class:"mute",style:"font-size:12px;margin-top:4px"},d.budgetUsedPct+"% of "+d.dailyBudgetMinutes+" min free budget"))),stat("This month",d.monthMinutes+" min",h("div",{class:"mute"},d.lifetimeMinutes+" lifetime")),stat("Errors today",d.errorsToday)),
h("h2",{},"Safety controls"),h("div",{class:"card row",style:"justify-content:space-between"},h("div",{},h("b",{},"Kill switch: "),h("span",{class:"tag "+(d.killSwitch?"revoked":"active")},d.killSwitch?"ON — managed dictation paused":"Off"),h("div",{class:"mute",style:"font-size:13px"},"Paid usage: "+(d.paidUsageEnabled?"enabled":"disabled")+" · OpenRouter: "+(d.openRouterEnabled?"on":"off")+". Users fall back to their own key or on-device.")),
h("button",{class:"btn "+(d.killSwitch?"":"bad"),disabled:!allowed("kill_switch"),title:allowed("kill_switch")?"":"Owner only",on:{click:()=>confirmAction(d.killSwitch?"Turn kill switch off":"Turn kill switch ON","This affects every user immediately.",null,(r)=>api("/kill-switch",{method:"POST",body:JSON.stringify({on:!d.killSwitch,reason:r})}))}},d.killSwitch?"Resume managed dictation":"Pause all managed dictation")),
h("h2",{},"Provider pool keys"),h("div",{class:"card"},h("table",{},h("tr",{},h("th",{},"Pool"),h("th",{},"Provider"),h("th",{},"Key fingerprint"),h("th",{},"Devices"),h("th",{},"State")),d.pools.map(p=>h("tr",{},h("td",{},p.label),h("td",{},p.provider),h("td",{},h("code",{},p.fingerprint)),h("td",{},p.installs),h("td",{},h("span",{class:"tag "+(p.full?"revoked":"active")},p.full?"full":p.status))))),h("p",{class:"mute",style:"font-size:12px"},"Only fingerprints are shown. Secrets never reach this page.")))}
async function users(){const app=$("#app");const q=h("input",{placeholder:"Search by email…",style:"max-width:340px"});const box=h("div",{class:"card"});
async function load(){const j=await api("/users?q="+encodeURIComponent(q.value));box.replaceChildren(h("table",{},h("tr",{},h("th",{},"Email"),h("th",{},"Plan"),h("th",{},"Status"),h("th",{},"Devices"),h("th",{},"Today"),h("th",{},"Month"),h("th",{},"Lifetime")),j.users.map(u=>h("tr",{class:"click",on:{click:()=>show("user/"+u.userId)}},h("td",{},u.email),h("td",{},h("span",{class:"tag "+u.plan},u.plan)),h("td",{},h("span",{class:"tag "+u.status},u.status)),h("td",{},u.activeInstalls+"/"+u.installs),h("td",{},u.dailyMinutes+" min"),h("td",{},u.monthlyMinutes+" min"),h("td",{},u.lifetimeMinutes+" min")))))}
q.oninput=load;app.replaceChildren(h("h2",{style:"margin-top:0"},"Users"),h("div",{class:"row",style:"margin-bottom:12px"},q),box);load()}
async function userView(id){const {user:u}=await api("/users/"+id);const app=$("#app");
const act=(label,perm,cls,fn)=>h("button",{class:"btn "+cls,disabled:!allowed(perm),title:allowed(perm)?"":"Your role cannot do this",on:{click:fn}},label);
app.replaceChildren(h("button",{class:"btn ghost",on:{click:()=>show("users")}},"← All users"),h("h2",{},u.email+" "),
h("div",{class:"row",style:"margin-bottom:14px"},h("span",{class:"tag "+u.plan},u.plan),h("span",{class:"tag "+u.status},u.status),h("span",{class:"mute"},"Joined "+new Date(u.createdAt).toLocaleDateString()),
act(u.plan==="paid"?"Set to free":"Set to paid","set_plan","ghost",()=>confirmAction("Change plan","Switch this user to "+(u.plan==="paid"?"free":"paid")+".",null,(r)=>api("/users/"+id+"/plan",{method:"POST",body:JSON.stringify({plan:u.plan==="paid"?"free":"paid",reason:r})}))),
act(u.status==="suspended"?"Unsuspend":"Suspend","suspend","ghost",()=>confirmAction(u.status==="suspended"?"Unsuspend user":"Suspend user","Managed dictation stops for all their devices.",null,(r)=>api("/users/"+id+"/suspend",{method:"POST",body:JSON.stringify({suspended:u.status!=="suspended",reason:r})})))),
h("div",{class:"grid"},stat("Today",u.usage.dailyMinutes+" min",h("div",{class:"mute"},"limit "+u.quota.dailyLimit+" · left "+u.quota.remainingDaily)),stat("This month",u.usage.monthlyMinutes+" min",h("div",{class:"mute"},"limit "+u.quota.monthlyLimit)),stat("Lifetime",u.usage.lifetimeMinutes+" min"),stat("Errors",Object.values(u.errorsByClass).reduce((a,b)=>a+b,0),h("div",{class:"mute"},Object.entries(u.errorsByClass).map(([k,v])=>k+": "+v).join(" · ")||"none")),stat("Quota",u.quota.blockedBy||"OK",h("div",{class:"mute"},Object.entries(u.minutesByProvider).map(([k,v])=>k+" "+v+"m").join(" · ")))),
h("h2",{},"Devices & access keys"),h("div",{class:"card"},h("table",{},h("tr",{},h("th",{},"Device"),h("th",{},"Status"),h("th",{},"Access key"),h("th",{},"Pool"),h("th",{},"Version"),h("th",{},"Last seen"),h("th",{},"Actions")),
u.devices.map(dv=>h("tr",{},h("td",{},dv.deviceName,h("div",{class:"mute",style:"font-size:12px"},"device ",h("code",{},dv.deviceKeyFingerprint))),h("td",{},h("span",{class:"tag "+dv.status},dv.status)),
h("td",{},dv.token?h("span",{},h("code",{},dv.token.tokenPublicId),h("div",{class:"mute",style:"font-size:12px"},"…"+dv.token.last4+(dv.token.revoked?" (revoked)":""))):"—"),
h("td",{},dv.pool?h("span",{},dv.pool.label,h("div",{},h("code",{},dv.pool.fingerprint))):"—"),h("td",{},dv.appVersion),h("td",{},dv.lastSeenAt?new Date(dv.lastSeenAt).toLocaleString():"—",h("div",{class:"mute",style:"font-size:12px"},(dv.lastProvider||"")+" "+(dv.lastStatus||""))),
h("td",{},h("div",{class:"row"},act("Reissue","reissue","",()=>confirmAction("Reissue access key","The old key stops now. Bolo on this device reconnects automatically — the user does nothing.",null,(r)=>api("/installs/"+dv.installId+"/reissue",{method:"POST",body:JSON.stringify({reason:r})}))),
act("Revoke","revoke","bad",()=>confirmAction("Revoke device","This device must sign in again before managed dictation works.",null,(r)=>api("/installs/"+dv.installId+"/revoke",{method:"POST",body:JSON.stringify({reason:r})}))),
act("Move pool","move_pool","ghost",()=>{const sel=h("select",{id:"poolsel"},["pool_wai_a","pool_wai_b","pool_groq_1"].map(p=>h("option",{value:p},p)));confirmAction("Move to another pool key","Device reconnects automatically on the new pool.",h("div",{style:"margin-bottom:10px"},sel),(r)=>api("/installs/"+dv.installId+"/move-pool",{method:"POST",body:JSON.stringify({poolId:sel.value,reason:r})}))}))))))),
h("h2",{},"User notices (placeholder)"),h("div",{class:"card"},u.notices.length?u.notices.map(n=>h("p",{},n.message,h("span",{class:"mute"}," — "+n.createdBy))):h("p",{class:"mute"},"No notices yet. Notices appear in the Bolo status check."),
h("div",{class:"row"},h("input",{id:"nmsg",placeholder:"Short friendly message to show in Bolo",style:"flex:1"}),act("Send notice","notice","ghost",async()=>{try{await api("/users/"+id+"/notice",{method:"POST",body:JSON.stringify({message:$("#nmsg").value})});toast("Notice queued");userView(id)}catch(e){toast(e.message)}}))),
h("h2",{},"Recent admin actions"),h("div",{class:"card"},auditTable(u.recentAudit)))}
function auditTable(rows){return h("table",{},h("tr",{},h("th",{},"When"),h("th",{},"Admin"),h("th",{},"Action"),h("th",{},"Target"),h("th",{},"Reason"),h("th",{},"Result")),rows.map(a=>h("tr",{},h("td",{},new Date(a.at).toLocaleString()),h("td",{},a.adminEmail+" ("+a.role+")"),h("td",{},a.action+(a.detail?" · "+a.detail:"")),h("td",{},h("code",{},a.target)),h("td",{},a.reason||"—"),h("td",{},h("span",{class:"tag "+(a.result==="ok"?"active":"revoked")},a.result)))))}
async function audit(){const j=await api("/audit");$("#app").replaceChildren(h("h2",{style:"margin-top:0"},"Audit log"),h("p",{class:"mute"},"Append-only. Every sign-in, change and denied attempt is recorded."),h("div",{class:"card"},auditTable(j.entries)))}
async function team(){if(!allowed("manage_team")){toast("Owner only");return show("dash")}
const app=$("#app");const box=h("div",{class:"card"});const created=h("div",{});
async function load(){
  const j=await api("/team");
  box.replaceChildren(h("table",{},
    h("tr",{},h("th",{},"Email"),h("th",{},"Name"),h("th",{},"Role"),h("th",{},"Status"),h("th",{},"Authenticator"),h("th",{},"Actions")),
    ...j.staff.map(st=>h("tr",{},
      h("td",{},st.email),
      h("td",{},st.label),
      h("td",{},h("span",{class:"tag"},st.role)),
      h("td",{},h("span",{class:"tag "+(st.enabled?"active":"disabled")},st.enabled?"enabled":"disabled")),
      h("td",{},st.totpEnrolled?"ready":"pending"),
      h("td",{},h("div",{class:"row"},
        h("select",{id:"role-"+st.email,style:"width:auto"},...["owner","support","viewer"].map(r=>h("option",{value:r,selected:r===st.role?true:false},r))),
        h("button",{class:"btn ghost",on:{click:()=>{const sel=document.getElementById("role-"+st.email);confirmAction("Change role","Set "+st.email+" to the selected role.",null,(r)=>api("/team/"+encodeURIComponent(st.email)+"/role",{method:"POST",body:JSON.stringify({role:sel.value,reason:r})}))}}},"Save role"),
        h("button",{class:"btn "+(st.enabled?"bad":""),on:{click:()=>confirmAction(st.enabled?"Disable account":"Enable account",(st.enabled?"Disable":"Enable")+" Office access for "+st.email+".",null,(r)=>api("/team/"+encodeURIComponent(st.email)+"/enabled",{method:"POST",body:JSON.stringify({enabled:!st.enabled,reason:r})}))}},st.enabled?"Disable":"Enable")
      ))
    ))
  ));
}
const form=h("div",{class:"card"},
  h("h3",{style:"margin-top:0;color:var(--gold2)"},"Add staff"),
  h("div",{class:"grid"},
    h("div",{},h("label",{class:"k"},"Email"),h("input",{id:"te"})),
    h("div",{},h("label",{class:"k"},"Name"),h("input",{id:"tn"})),
    h("div",{},h("label",{class:"k"},"Role"),h("select",{id:"tr"},h("option",{value:"support"},"support"),h("option",{value:"viewer"},"viewer"),h("option",{value:"owner"},"owner"))),
    h("div",{},h("label",{class:"k"},"Password (14+ characters)"),h("input",{id:"tp",type:"password"}))
  ),
  h("button",{class:"btn",style:"margin-top:12px",on:{click:()=>confirmAction("Create staff account","Creates an Office login. You will see a one-time authenticator setup to share securely.",null,async(r)=>{
    const j=await api("/team",{method:"POST",body:JSON.stringify({email:$("#te").value,label:$("#tn").value,role:$("#tr").value,password:$("#tp").value,reason:r})});
    created.replaceChildren(h("div",{class:"card"},h("h3",{style:"margin-top:0;color:var(--gold2)"},"One-time authenticator setup"),h("p",{class:"mute"},"Show this once to "+j.staff.email+". It will not be shown again from this page."),...enrollBlock(j.enrollment)));
    $("#te").value="";$("#tn").value="";$("#tp").value="";load();
  })}},"Create account")
);
app.replaceChildren(h("h2",{style:"margin-top:0"},"Team"),h("p",{class:"mute"},"Owner-only. Create Office staff, assign roles, and enable or disable access."),form,created,h("h2",{},"Staff accounts"),box);load()}
boot().catch(()=>{});
</script></body></html>`;
