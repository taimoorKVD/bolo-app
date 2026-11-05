/** One-time owner password bootstrap page. Black & gold. No secrets in markup. */
export const SETUP_HTML = String.raw`<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Bolo Office Setup</title>
<style>
:root{--bg:#0b0b0c;--panel:#141416;--line:#2a2722;--gold:#d4af37;--gold2:#f1d27a;--text:#f3efe6;--mute:#a59f92;--bad:#e5715f}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--text);font:15px/1.5 "Segoe UI",system-ui,sans-serif}
.wrap{max-width:440px;margin:64px auto;padding:0 18px}
.logo{font-weight:800;font-size:22px;color:var(--gold);margin-bottom:18px}.logo span{color:var(--text);font-weight:500;margin-left:6px;font-size:14px}
.card{background:var(--panel);border:1px solid var(--line);border-radius:14px;padding:22px}
h1{font-size:18px;margin:0 0 8px;color:var(--gold2)}.mute{color:var(--mute)}
label{display:block;color:var(--mute);font-size:12px;text-transform:uppercase;letter-spacing:.08em;margin:14px 0 6px}
input{width:100%;background:#0f0f11;border:1px solid var(--line);color:var(--text);border-radius:9px;padding:10px 12px;font:inherit}
.btn{margin-top:18px;width:100%;background:var(--gold);color:#111;border:0;border-radius:9px;padding:10px 14px;font-weight:700;cursor:pointer;font:inherit}
.err{display:none;margin-top:12px;color:var(--bad);font-size:14px}
.ok{display:none;margin-top:12px;color:var(--gold2)}
</style></head><body>
<div class="wrap">
  <div class="logo">Bolo<span>Office Setup</span></div>
  <div class="card">
    <h1>Create the owner password</h1>
    <p class="mute">One-time setup for the Office owner account. You will need your authenticator app for sign-in after this.</p>
    <form id="f" autocomplete="off">
      <label for="token">Setup token</label>
      <input id="token" name="token" type="password" autocomplete="off" required>
      <label for="pw">New password (14+ characters)</label>
      <input id="pw" name="password" type="password" autocomplete="new-password" minlength="14" required>
      <label for="cf">Confirm password</label>
      <input id="cf" name="confirm" type="password" autocomplete="new-password" minlength="14" required>
      <button class="btn" type="submit">Set owner password</button>
      <p class="err" id="err"></p>
      <p class="ok" id="ok">Password saved. You can sign in at Office.</p>
    </form>
  </div>
</div>
<script>
const err=document.getElementById("err"); const ok=document.getElementById("ok");
document.getElementById("f").addEventListener("submit", async (e)=>{
  e.preventDefault(); err.style.display="none"; ok.style.display="none";
  const token=document.getElementById("token").value;
  const password=document.getElementById("pw").value;
  const confirm=document.getElementById("cf").value;
  try{
    const r=await fetch("/office/api/setup",{method:"POST",headers:{"content-type":"application/json"},body:JSON.stringify({token,password,confirm})});
    const j=await r.json().catch(()=>({}));
    if(!r.ok){ err.textContent=j.error||"Setup failed."; err.style.display="block"; return; }
    ok.style.display="block";
    setTimeout(()=>{ location.href="/admin"; }, 900);
  }catch(_){ err.textContent="Setup failed."; err.style.display="block"; }
});
</script>
</body></html>`;
