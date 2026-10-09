// Finds screenshots by the text in them, their name or their date.

const { invoke, convertFileSrc } = window.__TAURI__.core;
const { getCurrentWindow } = window.__TAURI__.window;

const q = document.getElementById("q");
const grid = document.getElementById("grid");
const note = document.getElementById("note");
let hits = [];
let active = 0;
let timer = 0;

function escape(s) {
  return s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
}

function highlight(text, words) {
  let html = escape(text);
  for (const w of words) {
    if (!w) continue;
    const re = new RegExp(`(${w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")})`, "gi");
    html = html.replace(re, "<mark>$1</mark>");
  }
  return html;
}

function when(secs) {
  return new Date(secs * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

async function run() {
  const query = q.value.trim();
  hits = await invoke("search", { query });
  const words = query.toLowerCase().split(/\s+/);
  grid.innerHTML = "";
  hits.forEach((h, i) => {
    const b = document.createElement("button");
    b.className = "hit";
    const src = convertFileSrc(h.path);
    const media = h.kind === "video" ? `<video src="${src}" muted preload="metadata"></video>` : `<img src="${src}" alt="" loading="lazy" />`;
    b.innerHTML = `${media}<b>${escape(h.name)}</b><small>${h.snippet ? highlight(h.snippet, words) : when(h.added)}</small>`;
    b.title = `${h.name}\n${when(h.added)}`;
    b.addEventListener("click", () => select(i));
    b.addEventListener("dblclick", () => invoke("open_path", { path: h.path }));
    grid.appendChild(b);
  });
  note.textContent = hits.length
    ? `${hits.length}${hits.length >= 120 ? "+" : ""} screenshot${hits.length === 1 ? "" : "s"}`
    : query
      ? "Nothing found."
      : "No screenshots yet.";
  select(0, false);
}

function select(i, scroll = true) {
  active = Math.max(0, Math.min(i, hits.length - 1));
  [...grid.children].forEach((el, n) => el.classList.toggle("active", n === active));
  if (scroll) grid.children[active]?.scrollIntoView({ block: "nearest" });
}

async function use() {
  const h = hits[active];
  if (!h) return;
  try {
    await invoke("hang_again", { path: h.path });
    note.textContent = `Copied ${h.name} and hung it on the line.`;
  } catch (e) {
    note.textContent = String(e);
  }
}

function columns() {
  const first = grid.children[0];
  if (!first) return 1;
  return Math.max(1, Math.round(grid.clientWidth / (first.offsetWidth + 12)));
}

q.addEventListener("input", () => {
  clearTimeout(timer);
  timer = setTimeout(run, 120);
});

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") return getCurrentWindow().hide();
  if (e.key === "Enter") return e.preventDefault(), use();
  const step = { ArrowRight: 1, ArrowLeft: -1, ArrowDown: columns(), ArrowUp: -columns() }[e.key];
  if (step && (e.target !== q || e.key === "ArrowDown" || e.key === "ArrowUp")) {
    e.preventDefault();
    select(active + step);
  }
});

window.addEventListener("focus", () => {
  q.select();
  run();
});

run();
