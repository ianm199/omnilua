// Shared chrome for the omniLua guide pages: masthead, sidebar TOC (with active
// state from the current filename), and footer. Page content lives in each
// page's HTML; this only injects the surrounding navigation.

const NAV = [
  { group: "Getting started", items: [
    { t: "Overview", h: "index.html" },
    { t: "Install", h: "index.html#install", sub: true },
    { t: "Your first embed", h: "index.html#embed", sub: true },
  ]},
  { group: "Guides", items: [
    { t: "Game scripting", h: "game-scripting.html" },
    { t: "Lua in the browser", h: "browser.html" },
    { t: "Embedding in Rust", h: "embedding.html" },
    { t: "Sandboxing scripts", h: "sandboxing.html" },
    { t: "Coming from mlua", h: "from-mlua.html" },
    { t: "Choosing a version", h: "versions.html" },
  ]},
  { group: "Reference", items: [
    { t: "API reference ↗", h: "https://docs.rs/omnilua" },
    { t: "Playground ↗", h: "../index.html#playground" },
  ]},
];

const here = location.pathname.split("/").pop() || "index.html";

function masthead() {
  const m = document.createElement("header");
  m.className = "masthead";
  m.innerHTML = `<div class="frame">
    <a class="wordmark" href="../index.html">omniLua</a>
    <nav class="mast-nav">
      <a href="../index.html#playground">Playground</a>
      <a href="../index.html#why">Why omniLua</a>
      <a href="../index.html#use-cases">Use cases</a>
      <a href="./index.html">Docs</a>
      <a href="../performance.html">Performance</a>
      <a class="cta" href="https://github.com/ianm199/omnilua">GitHub ↗</a>
    </nav>
  </div>`;
  document.body.prepend(m);
}

function sidebar() {
  const side = document.getElementById("docs-side");
  if (!side) return;
  const nav = document.createElement("nav");
  for (const sec of NAV) {
    const head = document.createElement("a");
    head.className = "top";
    head.textContent = sec.group;
    head.style.pointerEvents = "none";
    nav.appendChild(head);
    for (const it of sec.items) {
      const a = document.createElement("a");
      a.href = it.h;
      a.textContent = it.t;
      if (it.sub) a.className = "sub";
      if (it.h === here) a.classList.add("active");
      nav.appendChild(a);
    }
  }
  side.appendChild(nav);
}

function footer() {
  const f = document.createElement("footer");
  f.innerHTML = `<div class="frame">
    <div class="flinks">
      <a href="../index.html">Home</a>
      <a href="https://github.com/ianm199/omnilua">GitHub ↗</a>
      <a href="https://crates.io/crates/omnilua">crates.io ↗</a>
      <a href="https://www.npmjs.com/package/omnilua">npm ↗</a>
      <a href="https://docs.rs/omnilua">docs.rs ↗</a>
    </div>
    <p class="colophon">omniLua is a port of <a href="https://www.lua.org/">Lua</a> (PUC-Rio). Lua and this port are both MIT licensed.</p>
  </div>`;
  document.body.appendChild(f);
}

masthead();
sidebar();
footer();
