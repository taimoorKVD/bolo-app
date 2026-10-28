(() => {
  const releaseEl = document.getElementById("bolo-release");
  let release = null;
  try {
    release = JSON.parse(releaseEl.textContent);
  } catch (_) {
    release = null;
  }

  const formatBytes = (n) => {
    if (!n || n < 1) return null;
    if (n < 1024) return `${n} B`;
    const kb = n / 1024;
    if (kb < 1024) return `${kb < 10 ? kb.toFixed(1) : Math.round(kb)} KB`;
    const mb = kb / 1024;
    return `${mb < 10 ? mb.toFixed(2) : mb.toFixed(1)} MB`;
  };

  const applyRelease = (data) => {
    if (!data) return;
    const nsis = data.downloads && data.downloads.nsis;
    document.querySelectorAll("[data-field='version']").forEach((el) => {
      el.textContent = `Version ${data.version || "0.1.0"}`;
    });
    document.querySelectorAll("[data-field='requirements']").forEach((el) => {
      el.textContent = data.requirements || "Windows 10 or 11, 64-bit";
    });
    if (nsis) {
      const size = formatBytes(nsis.bytes);
      document.querySelectorAll("[data-field='size']").forEach((el) => {
        el.textContent = size ? `Installer ${size}` : "Installer size is published with the build";
      });
      document.querySelectorAll("[data-download='nsis']").forEach((a) => {
        a.setAttribute("href", nsis.href);
      });
    }
  };

  applyRelease(release);
  fetch("version.json", { cache: "no-store" })
    .then((r) => (r.ok ? r.json() : null))
    .then((data) => {
      if (data) applyRelease(data);
    })
    .catch(() => {});

  const toggle = document.querySelector(".nav-toggle");
  const nav = document.getElementById("site-nav");
  if (toggle && nav) {
    toggle.addEventListener("click", () => {
      const open = toggle.getAttribute("aria-expanded") === "true";
      toggle.setAttribute("aria-expanded", open ? "false" : "true");
      nav.classList.toggle("is-open", !open);
    });
  }

  const PHASES = [
    {
      id: "listening",
      ms: 3200,
      caption: "Hold Ctrl+Win — the listener watches while you speak",
      text: "",
      src: "assets/listen.webm",
    },
    {
      id: "transcribing",
      ms: 2800,
      caption: "Release — it writes your words with a tiny pen",
      text: "",
      src: "assets/write.webm",
    },
    {
      id: "inserted",
      ms: 2600,
      caption: "Clean text lands at the caret — the overlay slips away",
      text: "Hey, can you listen to me?",
      src: null,
    },
  ];

  const root = document.querySelector("[data-demo-root]");
  if (!root) return;

  const overlay = root.querySelector("[data-demo-overlay]");
  const video = root.querySelector("[data-demo-video]");
  const caption = root.querySelector("[data-demo-caption]");
  const textEl = root.querySelector("[data-demo-text]");
  let i = 0;
  let timer = null;

  const playClip = (src) => {
    if (!video) return;
    if (!src) {
      video.removeAttribute("src");
      video.load();
      return;
    }
    if (video.getAttribute("src") !== src) {
      video.src = src;
      video.load();
    }
    const p = video.play();
    if (p && typeof p.catch === "function") p.catch(() => {});
  };

  const show = (phase) => {
    if (overlay) {
      overlay.dataset.phase = phase.id;
      overlay.classList.toggle("is-hidden", phase.id === "inserted");
    }
    if (caption) caption.textContent = phase.caption;
    if (textEl) textEl.textContent = phase.text;
    playClip(phase.src);
  };

  const tick = () => {
    const phase = PHASES[i % PHASES.length];
    show(phase);
    i += 1;
    timer = window.setTimeout(tick, phase.ms);
  };

  const reduce = window.matchMedia("(prefers-reduced-motion: reduce)");
  if (reduce.matches) {
    show(PHASES[2]);
    if (overlay) overlay.classList.add("is-hidden");
  } else {
    tick();
  }

  document.addEventListener("visibilitychange", () => {
    if (document.hidden) {
      if (timer) window.clearTimeout(timer);
      if (video) video.pause();
    } else if (!reduce.matches) {
      tick();
    }
  });
})();
