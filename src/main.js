// The line: where each photo hangs, how it swings, and what your hands can
// do with it. The Rust side owns the files; this side only draws them.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const Layout = {
  ropeTop: 10,
  spacing: 174,
  cardWidth: 150,
  pinAbove: 9.5,
  maxPhoto: { w: 136, h: 104 },
  sag: (w) => Math.min(30, w * 0.018),
  ropeY(x, w) {
    if (w <= 0) return this.ropeTop;
    const f = x / w;
    return this.ropeTop + 4 * this.sag(w) * f * (1 - f);
  },
  x(i, n, w) {
    return w / 2 - ((Math.max(n, 1) - 1) * this.spacing) / 2 + i * this.spacing;
  },
};

const LONG_PRESS = 600;
const DOUBLE_CLICK = 320;
const DRAG_START = 5;

const lineEl = document.getElementById("line");
const pegsEl = document.getElementById("pegs");
const hintEl = document.getElementById("hint");
const edgeEl = document.getElementById("edge");
const actionsEl = document.getElementById("actions");

let settings = { mode: "poll", sound: true, edge: true, platform: "" };
let revealed = false;
/** id -> { el, swingEl, card, img, data, tilt, angle, velocity } */
const pegs = new Map();
let order = [];

// ---------------------------------------------------------------- Rope

function drawRope() {
  const w = window.innerWidth;
  const top = Layout.ropeTop;
  const d = `M -20 ${top} Q ${w / 2} ${top + 2 * Layout.sag(w)} ${w + 20} ${top}`;
  for (const p of document.querySelectorAll("#rope path")) p.setAttribute("d", d);
}

// ---------------------------------------------------------------- Photos

/** Every photo hangs a little crooked, like on a real line. */
function tiltFor(id) {
  const r = Math.sin(id * 12.9898 + 78.233) * 43758.5453;
  return (r - Math.floor(r)) * 5 - 2.5;
}

function photoSize(w, h) {
  const { w: maxW, h: maxH } = Layout.maxPhoto;
  if (!w || !h) return { w: maxW, h: maxH };
  const s = Math.min(maxW / w, maxH / h);
  return { w: Math.round(w * s), h: Math.round(h * s) };
}

const CROSS =
  '<svg viewBox="0 0 10 10"><path d="M1.5 1.5l7 7M8.5 1.5l-7 7" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/></svg>';
const CHECK =
  '<svg viewBox="0 0 12 12" width="10" height="10"><path d="M2 6.5l2.6 2.6L10 3.5" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>';

function makePeg(data) {
  const el = document.createElement("div");
  el.className = "peg arriving";
  el.innerHTML = `
    <div class="swing">
      <div class="pin"></div>
      <div class="card">
        <img alt="" draggable="false" />
        <button class="x" aria-label="Take down">${CROSS}</button>
        <div class="copied">${CHECK}<span>Copied</span></div>
      </div>
    </div>`;
  const peg = {
    el,
    swingEl: el.querySelector(".swing"),
    card: el.querySelector(".card"),
    img: el.querySelector("img"),
    data: null,
    tilt: tiltFor(data.id),
    angle: 16,
    velocity: 0,
  };
  update(peg, data);
  wireGestures(peg, data.id);
  pegsEl.appendChild(el);
  return peg;
}

function update(peg, data) {
  if (!peg.data || peg.data.thumb !== data.thumb) {
    peg.img.src = data.thumb;
    const s = photoSize(data.width, data.height);
    peg.img.style.width = `${s.w}px`;
    peg.img.style.height = `${s.h}px`;
  }
  peg.img.title = data.name;
  peg.data = data;
}

function layout() {
  const w = window.innerWidth;
  order.forEach((id, i) => {
    const peg = pegs.get(id);
    const x = Layout.x(i, order.length, w);
    peg.el.style.left = `${x - Layout.cardWidth / 2}px`;
    peg.el.style.top = `${Layout.ropeY(x, w) - Layout.pinAbove}px`;
  });
  hintEl.style.top = `${Layout.ropeY(w / 2, w) + 22}px`;
  hintEl.classList.toggle("shown", order.length === 0);
}

function setItems(items) {
  const ids = items.map((i) => i.id);
  for (const [id, peg] of pegs) {
    if (!ids.includes(id)) fall(id, peg);
  }
  for (const data of items) {
    const peg = pegs.get(data.id);
    if (peg) {
      update(peg, data);
    } else {
      const fresh = makePeg(data);
      pegs.set(data.id, fresh);
      // Drops onto the line on the next frame, then swings to rest.
      requestAnimationFrame(() => requestAnimationFrame(() => fresh.el.classList.remove("arriving")));
    }
  }
  order = ids;
  layout();
  animate();
}

/** Pulled off the line: it falls and fades. */
function fall(id, peg) {
  pegs.delete(id);
  if (actionsFor === id) closeActions();
  const dir = Math.random() < 0.5 ? -1 : 1;
  peg.el.classList.add("falling");
  peg.el.style.transform = `translateY(240px) rotate(${dir * (14 + Math.random() * 14)}deg)`;
  setTimeout(() => peg.el.remove(), 650);
}

// ---------------------------------------------------------------- Swing

// A damped spring per photo, so they swing and settle like real ones.
const STIFFNESS = 40;
const DAMPING = 2.5;
let animating = false;
let lastFrame = 0;

function nudge(peg, degrees) {
  peg.velocity += degrees * 6.3 * (Math.random() < 0.5 ? -1 : 1);
  animate();
}

function animate() {
  if (animating) return;
  animating = true;
  lastFrame = performance.now();
  requestAnimationFrame(frame);
}

function frame(now) {
  const dt = Math.min(0.05, (now - lastFrame) / 1000);
  lastFrame = now;
  let moving = false;
  for (const peg of pegs.values()) {
    // Small steps keep the spring stable on slow frames.
    for (let i = 0; i < 4; i++) {
      const h = dt / 4;
      const a = -STIFFNESS * peg.angle - DAMPING * peg.velocity;
      peg.velocity += a * h;
      peg.angle += peg.velocity * h;
    }
    if (Math.abs(peg.angle) < 0.02 && Math.abs(peg.velocity) < 0.05) {
      peg.angle = 0;
      peg.velocity = 0;
    } else {
      moving = true;
    }
    peg.swingEl.style.transform = `rotate(${(peg.tilt + peg.angle).toFixed(3)}deg)`;
  }
  reportShapes();
  if (moving || transitioning()) {
    requestAnimationFrame(frame);
  } else {
    animating = false;
    reportShapes();
  }
}

/** Slides and drops run for about half a second after any change. */
let busyUntil = 0;
function settleFor(ms) {
  busyUntil = Math.max(busyUntil, performance.now() + ms);
  animate();
}
function transitioning() {
  return performance.now() < busyUntil;
}

/** Every so often a little wind moves the line. */
function breeze() {
  setTimeout(breeze, 7000 + Math.random() * 9000);
  if (!revealed || dragging) return;
  for (const peg of pegs.values()) {
    setTimeout(() => nudge(peg, 1.6 + Math.random() * 1.8), Math.random() * 350);
  }
}

// ---------------------------------------------------------------- Hands

let dragging = false;
let lastClick = { id: 0, t: 0 };

function wireGestures(peg, id) {
  const { card } = peg;
  let down = null;
  let pressTimer = 0;

  const cancelPress = () => {
    clearTimeout(pressTimer);
    card.classList.remove("pressing");
  };

  card.addEventListener("pointerdown", (e) => {
    if (e.button !== 0 || e.target.closest(".x")) return;
    down = { x: e.screenX, y: e.screenY, long: false };
    card.classList.add("pressing");
    pressTimer = setTimeout(() => {
      if (!down) return;
      down.long = true;
      card.classList.remove("pressing");
      invoke("edit", { id });
    }, LONG_PRESS);
  });

  card.addEventListener("pointermove", (e) => {
    if (!down || down.long) return;
    if (Math.hypot(e.screenX - down.x, e.screenY - down.y) < DRAG_START) return;
    cancelPress();
    down = null;
    dragging = true;
    card.classList.add("dragging");
    invoke("start_drag", { id }).catch((err) => {
      console.warn("drag failed", err);
      endDrag(id);
    });
  });

  card.addEventListener("pointerup", (e) => {
    if (!down) return;
    const wasLong = down.long;
    down = null;
    cancelPress();
    if (wasLong || e.target.closest(".x")) return;
    const now = performance.now();
    if (lastClick.id === id && now - lastClick.t < DOUBLE_CLICK) {
      lastClick = { id: 0, t: 0 };
      invoke("open", { id });
    } else {
      lastClick = { id, t: now };
      invoke("copy", { id });
    }
  });

  card.addEventListener("pointerleave", () => {
    if (down && !down.long) cancelPress();
  });

  card.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    cancelPress();
    down = null;
    if (settings.platform === "linux") {
      openActions(id);
    } else {
      invoke("menu", { id, x: e.clientX, y: e.clientY });
    }
  });

  card.querySelector(".x").addEventListener("click", (e) => {
    e.stopPropagation();
    invoke("take_down", { id });
  });
}

// ---------------------------------------------------------------- Actions

// Native menus opened from the web view get no pointer input on Wayland, so
// Linux gets this bar instead.
let actionsFor = 0;

function openActions(id) {
  const peg = pegs.get(id);
  if (!peg) return;
  actionsFor = id;
  const card = peg.card.getBoundingClientRect();
  const w = actionsEl.offsetWidth;
  const h = actionsEl.offsetHeight;
  const left = Math.min(Math.max(4, card.left + card.width / 2 - w / 2), window.innerWidth - w - 4);
  const top = Math.min(card.bottom + 8, window.innerHeight - h - 4);
  actionsEl.style.left = `${left}px`;
  actionsEl.style.top = `${top}px`;
  actionsEl.classList.add("open");
  reportShapes(true);
}

function closeActions() {
  if (!actionsFor) return;
  actionsFor = 0;
  actionsEl.classList.remove("open");
  reportShapes(true);
}

actionsEl.addEventListener("click", (e) => {
  const button = e.target.closest("button");
  if (!button || !actionsFor) return;
  const id = actionsFor;
  closeActions();
  invoke(button.dataset.act, { id });
});

document.addEventListener("pointerdown", (e) => {
  if (actionsFor && !e.target.closest("#actions")) closeActions();
});
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") closeActions();
});

function endDrag(id) {
  dragging = false;
  pegs.get(id)?.card.classList.remove("dragging");
  reportShapes(true);
}

// ---------------------------------------------------------------- Pointer

// macOS and Windows: tell Rust where the photos are, so clicks anywhere else
// go through to the apps underneath.
// Linux: Wayland never says where the pointer is, so the window's input
// region shrinks to the photos instead, and GTK reports when it comes and goes.

let lastShapes = "";

function rectOf(el, pad = 0) {
  const r = el.getBoundingClientRect();
  return { x: r.left - pad, y: r.top - pad, w: r.width + pad * 2, h: r.height + pad * 2 };
}

function reportShapes(force = false) {
  const cards = [...pegs.values()].map((p) => rectOf(p.card, 2));
  if (actionsFor) cards.push(rectOf(actionsEl, 2));
  if (settings.mode === "poll") {
    const key = JSON.stringify(cards.map((r) => [r.x | 0, r.y | 0, r.w | 0, r.h | 0]));
    if (force || key !== lastShapes) {
      lastShapes = key;
      invoke("hit_rects", { rects: cards });
    }
    return;
  }
  let region = [];
  if (revealed) {
    // From the top of the window down to the photos, so the pointer can come
    // down from the bar without the line tucking away under it.
    const shapes = order.length ? cards : [rectOf(hintEl, 6)];
    if (shapes.length) {
      const left = Math.min(...shapes.map((r) => r.x)) - 12;
      const right = Math.max(...shapes.map((r) => r.x + r.w)) + 12;
      const bottom = Math.max(...shapes.map((r) => r.y + r.h)) + 20;
      region = [{ x: Math.max(0, left), y: 0, w: right - Math.max(0, left), h: bottom }];
    }
    // When a photo goes, the photos left may no longer be under the pointer.
    // Keep the old area until it moves away, or the line would tuck at once.
    const prev = lastRegion[0];
    if (prev && region[0] && within(prev, pointerAt) && !within(region[0], pointerAt)) {
      const r = region[0];
      const x = Math.min(r.x, prev.x);
      region = [{ x, y: 0, w: Math.max(r.x + r.w, prev.x + prev.w) - x, h: Math.max(r.h, prev.h) }];
    }
  } else if (settings.edge && order.length) {
    region = [{ x: 0, y: 0, w: window.innerWidth, h: 4 }];
  }
  lastRegion = region;
  const key = JSON.stringify(region.map((r) => [r.x | 0, r.y | 0, r.w | 0, r.h | 0]));
  if (force || key !== lastShapes) {
    lastShapes = key;
    invoke("input_region", { rects: region });
  }
}

let lastRegion = [];
let pointerAt = { x: -1, y: -1 };
document.addEventListener("mousemove", (e) => {
  pointerAt = { x: e.clientX, y: e.clientY };
});

function within(r, p) {
  return p.x >= r.x && p.x <= r.x + r.w && p.y >= r.y && p.y <= r.y + r.h;
}

// Resting at the top edge brings the line down (Linux; Rust watches the
// cursor itself elsewhere).
let edgeTimer = 0;
edgeEl.addEventListener("mousemove", () => {
  if (revealed || edgeTimer) return;
  edgeTimer = setTimeout(() => {
    edgeTimer = 0;
    if (!revealed) invoke("edge");
  }, 180);
});
edgeEl.addEventListener("mouseleave", () => {
  clearTimeout(edgeTimer);
  edgeTimer = 0;
});

// ---------------------------------------------------------------- Sound

let audio;
function chime(kind) {
  if (!settings.sound) return;
  try {
    audio ??= new AudioContext();
    const t = audio.currentTime;
    const osc = audio.createOscillator();
    const gain = audio.createGain();
    osc.type = "sine";
    if (kind === "tink") {
      osc.frequency.setValueAtTime(2350, t);
      gain.gain.setValueAtTime(0.06, t);
      gain.gain.exponentialRampToValueAtTime(0.0001, t + 0.18);
    } else {
      osc.frequency.setValueAtTime(620, t);
      osc.frequency.exponentialRampToValueAtTime(280, t + 0.09);
      gain.gain.setValueAtTime(0.08, t);
      gain.gain.exponentialRampToValueAtTime(0.0001, t + 0.12);
    }
    osc.connect(gain).connect(audio.destination);
    osc.start(t);
    osc.stop(t + 0.2);
  } catch {
    // No audio device: stay quiet.
  }
}

// ---------------------------------------------------------------- Wiring

function setRevealed(v) {
  revealed = v;
  if (!v) closeActions();
  lineEl.classList.toggle("down", v);
  settleFor(600);
  reportShapes(true);
}

async function init() {
  drawRope();
  settings = await invoke("settings");
  document.documentElement.dataset.platform = settings.platform;

  await listen("items", (e) => setItems(e.payload));
  await listen("reveal", (e) => setRevealed(e.payload));
  await listen("hung", (e) => {
    chime("tink");
    const peg = pegs.get(e.payload.id);
    if (peg) {
      peg.angle = 16;
      peg.velocity = 0;
    }
    settleFor(700);
  });
  await listen("fell", () => chime("pop"));
  await listen("cleared", () => chime("pop"));
  await listen("copied", (e) => {
    const peg = pegs.get(e.payload);
    if (!peg) return;
    peg.card.classList.add("copied-now");
    nudge(peg, 3);
    clearTimeout(peg.copiedTimer);
    peg.copiedTimer = setTimeout(() => peg.card.classList.remove("copied-now"), 1200);
  });
  await listen("drag-end", (e) => endDrag(e.payload));
  await listen("settings", async () => {
    settings = await invoke("settings");
    reportShapes(true);
  });

  const items = await invoke("items");
  // What was already hanging comes back without the drop-in.
  for (const data of items) {
    const peg = makePeg(data);
    peg.el.classList.remove("arriving");
    peg.angle = 0;
    pegs.set(data.id, peg);
  }
  order = items.map((i) => i.id);
  layout();
  setRevealed(settings.revealed);
  setTimeout(breeze, 9000);
}

window.addEventListener("resize", () => {
  drawRope();
  layout();
  settleFor(600);
});

init();
