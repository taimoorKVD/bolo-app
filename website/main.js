const SENTENCE = "Ship the notes to Priya before Thursday.";
const PHASES = [
  { id: "listening", label: "Listening", ms: 2200 },
  { id: "transcribing", label: "Transcribing", ms: 1400 },
  { id: "inserted", label: "Inserted", ms: 2400 },
];

function formatBytes(bytes) {
  if (bytes == null || Number.isNaN(Number(bytes))) return "Installer size is published with the build";
  const value = Number(bytes);
  if (value >= 1024 * 1024) return `${(value / (1024 * 1024)).toFixed(1)} MB installer`;
  if (value >= 1024) return `${Math.max(1, Math.round(value / 1024))} KB installer`;
  return `${value} byte installer`;
}

async function readRelease() {
  try {
    const response = await fetch(new URL("version.json", document.baseURI), { cache: "no-store" });
    if (response.ok) return await response.json();
  } catch {
    // A double-clicked file:// page cannot fetch. The embedded copy is the same JSON.
  }
  const embedded = document.getElementById("bolo-release");
  if (!embedded) return null;
  try {
    return JSON.parse(embedded.textContent);
  } catch {
    return null;
  }
}

function applyRelease(release) {
  if (!release) return;
  const version = release.version ? `Version ${release.version}` : "Version 0.1.0";
  const requirements = release.requirements || "Windows 10 or 11, 64-bit";
  const size = formatBytes(release.downloads && release.downloads.nsis && release.downloads.nsis.bytes);
  document.querySelectorAll("[data-field='version']").forEach((node) => {
    node.textContent = version;
  });
  document.querySelectorAll("[data-field='size']").forEach((node) => {
    node.textContent = size;
  });
  document.querySelectorAll("[data-field='requirements']").forEach((node) => {
    node.textContent = requirements;
  });
  const nsis = release.downloads && release.downloads.nsis && release.downloads.nsis.href;
  if (nsis) {
    document.querySelectorAll("[data-download='nsis']").forEach((node) => {
      node.setAttribute("href", nsis);
    });
  }
}

function setupNav() {
  const toggle = document.querySelector(".nav-toggle");
  const nav = document.getElementById("site-nav");
  if (!toggle || !nav) return;
  toggle.addEventListener("click", () => {
    const open = nav.classList.toggle("is-open");
    toggle.setAttribute("aria-expanded", open ? "true" : "false");
  });
  nav.querySelectorAll("a").forEach((link) => {
    link.addEventListener("click", () => {
      nav.classList.remove("is-open");
      toggle.setAttribute("aria-expanded", "false");
    });
  });
}

function setupDemo() {
  const pill = document.getElementById("demo-pill");
  const label = document.getElementById("demo-label");
  const timer = document.getElementById("demo-timer");
  const text = document.getElementById("demo-text");
  if (!pill || !label || !text) return;

  const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  let phaseIndex = 0;
  let timerId = 0;
  let typeId = 0;
  let seconds = 1;

  function show(phase) {
    window.clearInterval(typeId);
    pill.dataset.phase = phase.id;
    label.textContent = phase.label;
    if (phase.id === "listening") {
      seconds = 1;
      if (timer) timer.textContent = "0:01";
      text.textContent = "";
    } else if (phase.id === "transcribing") {
      text.textContent = "";
    } else {
      if (reduced) {
        text.textContent = SENTENCE;
        return;
      }
      let count = 0;
      text.textContent = "";
      typeId = window.setInterval(() => {
        count += 1;
        text.textContent = SENTENCE.slice(0, count);
        if (count >= SENTENCE.length) window.clearInterval(typeId);
      }, 28);
    }
  }

  show(PHASES[0]);
  if (reduced) {
    show(PHASES[2]);
    return;
  }
  window.setInterval(() => {
    if (pill.dataset.phase === "listening" && timer) {
      seconds += 1;
      timer.textContent = `0:0${Math.min(seconds, 9)}`;
    }
  }, 1000);
  function advance() {
    phaseIndex = (phaseIndex + 1) % PHASES.length;
    const phase = PHASES[phaseIndex];
    show(phase);
    timerId = window.setTimeout(advance, phase.ms);
  }
  timerId = window.setTimeout(advance, PHASES[0].ms);
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) window.clearTimeout(timerId);
  });
}

readRelease().then(applyRelease);
setupNav();
setupDemo();
