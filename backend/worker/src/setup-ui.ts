/** One-time owner password bootstrap. Split layout, no nav, no external assets. */
export const SETUP_HTML = String.raw`<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="robots" content="noindex, nofollow">
<title>Bolo Office Setup</title>
<style>
:root{--bg:#0c0b09;--line:#3a3224;--gold:#e0b85a;--gold2:#f3d48a;--text:#f6f1e6;--mute:#cbbfa8;--bad:#ffb4a8}
*{box-sizing:border-box}html,body{margin:0;min-height:100%}
body{background:#0e0d0b;color:var(--text);font:16px/1.5 "Segoe UI",system-ui,sans-serif}
[hidden]{display:none !important}
#gate{min-height:100vh;display:flex}
.brand{flex:1.05;display:flex;flex-direction:column;justify-content:center;padding:64px 56px;position:relative;overflow:hidden;background:
radial-gradient(900px 420px at 15% 0%,rgba(224,184,90,.2),transparent 60%),linear-gradient(165deg,#1c160e,#0c0b09 58%)}
.mark{font-weight:800;font-size:28px;letter-spacing:.2px;color:var(--gold)}.mark span{color:var(--text);font-weight:560;margin-left:8px;font-size:18px}
.brand h2{font-size:34px;line-height:1.15;margin:18px 0 10px;font-weight:650;max-width:16ch}
.lede{color:var(--mute);max-width:38ch;margin:0}
.wave{position:absolute;left:0;right:0;bottom:0;color:rgba(224,184,90,.45);width:100%}
.panel{flex:.95;display:flex;align-items:center;justify-content:center;padding:40px 28px}
.card{width:100%;max-width:420px}
h1{font-size:26px;margin:0 0 8px;font-weight:650}
.mute{color:var(--mute)}
label{display:block;color:var(--mute);font-size:12px;font-weight:650;letter-spacing:.06em;text-transform:uppercase;margin:16px 0 6px}
input{width:100%;height:48px;background:#0c0b09;border:1px solid var(--line);color:var(--text);border-radius:12px;padding:0 14px;font:inherit}
input:focus{outline:none;border-color:var(--gold);box-shadow:0 0 0 3px rgba(224,184,90,.28)}
.btn{margin-top:18px;width:100%;height:48px;background:linear-gradient(180deg,#f0d48a,#d4af37);color:#1a1408;border:0;border-radius:12px;font:inherit;font-weight:750;cursor:pointer}
.btn[disabled]{opacity:.55;cursor:not-allowed}
.err{color:var(--bad);margin:12px 0 0;min-height:0}
.ok{color:var(--gold2);margin:12px 0 0}
@media (max-width:860px){
#gate{flex-direction:column}.panel{order:1;padding:28px 18px 8px}.brand{order:2;padding:12px 22px 28px}
.brand h2,.wave{display:none}.mark{font-size:22px}
}
</style></head><body>
<div id="gate">
<section class="brand">
  <div class="mark">Bolo <span>Office</span></div>
  <h2>Speak anywhere in Windows.</h2>
  <p class="lede">Push-to-talk dictation for Windows, by Kingdom Vision. Hold a hotkey, speak, and the text lands at the caret.</p>
  <svg class="wave" viewBox="0 0 640 110" aria-hidden="true"><path d="M0 70c40 0 40-40 80-40s40 40 80 40 40-50 80-50 40 60 80 60 40-40 80-40 40 50 80 50 40-45 80-45 40 45 80 45" fill="none" stroke="currentColor" stroke-width="2"/></svg>
</section>
<section class="panel"><div class="card">
  <h1>Create the owner password</h1>
  <p class="mute">One-time setup. After this, sign in with your password and, if email codes are on, a code sent to you.</p>
  <form id="f" autocomplete="off">
    <label for="token">Setup token</label>
    <input id="token" name="token" type="password" autocomplete="off" required>
    <label for="pw">New password (14+ characters)</label>
    <input id="pw" name="password" type="password" autocomplete="new-password" minlength="14" required>
    <label for="cf">Confirm password</label>
    <input id="cf" name="confirm" type="password" autocomplete="new-password" minlength="14" required>
    <p class="err" id="err" role="alert" hidden></p>
    <p class="ok" id="ok" hidden>Password saved. Taking you to sign in.</p>
    <button class="btn" id="go" type="submit">Set owner password</button>
  </form>
</div></section>
</div>
<script>
const CFG=window.__BOLO_OFFICE__||{apiBase:"/admin/api",home:"/admin"};
const err=document.getElementById("err"); const ok=document.getElementById("ok"); const go=document.getElementById("go");
document.getElementById("f").addEventListener("submit", async (e)=>{
  e.preventDefault(); err.hidden=true; ok.hidden=true; go.disabled=true; go.textContent="Saving…";
  const token=document.getElementById("token").value;
  const password=document.getElementById("pw").value;
  const confirm=document.getElementById("cf").value;
  try{
    const r=await fetch(CFG.apiBase+"/setup",{method:"POST",headers:{"content-type":"application/json"},body:JSON.stringify({token,password,confirm})});
    const j=await r.json().catch(()=>({}));
    if(!r.ok){ err.textContent=j.error||"Setup failed."; err.hidden=false; go.disabled=false; go.textContent="Set owner password"; return; }
    ok.hidden=false;
    setTimeout(()=>{ location.href=CFG.home||"/"; }, 900);
  }catch(_){ err.textContent="Setup failed."; err.hidden=false; go.disabled=false; go.textContent="Set owner password"; }
});
</script>
</body></html>`;
