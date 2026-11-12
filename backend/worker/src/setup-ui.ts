import { OFFICE_CSS } from "../../office/office-css";

/** One-time owner password bootstrap. Split layout, no nav, no external assets. */
export const SETUP_HTML = String.raw`<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="robots" content="noindex, nofollow">
<title>Bolo Office Setup</title>
<link rel="icon" href="/favicon.ico" type="image/x-icon">
<style>/*OFFICE_CSS*/</style>
</head><body>
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
</body></html>`.replace("/*OFFICE_CSS*/", () => OFFICE_CSS);

