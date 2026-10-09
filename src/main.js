// The line: where each photo hangs, how it swings, and what your hands can
// do with it. The Rust side owns the files; this side only draws them.

const { invoke, convertFileSrc } = window.__TAURI__.core;
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

const $ = (id) => document.getElementById(id);
const lineEl = $("line");
const pegsEl = $("pegs");
const hintEl = $("hint");
const edgeEl = $("edge");
const actionsEl = $("actions");
const qrEl = $("qr");
const tagEl = $("tag");
const toastEl = $("toast");

let settings = { mode: "poll", sound: true, edge: true, platform: "", actions: [], line: { names: [], active: 0 } };
let revealed = false;
/** id -> { el, swingEl, card, media, data, tilt, angle, velocity } */
const pegs = new Map();
let order = [];
/** Photos picked with Ctrl/Cmd-click or Space, for joining or dragging together. */
const selected = new Set();
/** The photo the keyboard is on. */
let focusId = 0;

// ---------------------------------------------------------------- Icons

const svg = (d, extra = "") => `<svg viewBox="0 0 16 16" ${extra}><path d="${d}"/></svg>`;
const ICON = {
  copy: '<svg viewBox="0 0 16 16"><rect x="5" y="5" width="8.5" height="8.5" rx="2"/><path d="M10.5 5V3.5a1.5 1.5 0 0 0-1.5-1.5H4A1.5 1.5 0 0 0 2.5 3.5V9A1.5 1.5 0 0 0 4 10.5h1"/></svg>',
  text: svg("M3 3.5h10M8 3.5v9.5M5.5 13h5"),
  open: svg("M9 2.5h4.5V7M13.5 2.5 7.5 8.5M11.5 9.5v3a1 1 0 0 1-1 1h-7a1 1 0 0 1-1-1v-7a1 1 0 0 1 1-1h3"),
  edit: svg("M10.5 2.5l3 3-8 8H2.5v-3z"),
  redact: '<svg viewBox="0 0 16 16"><rect x="2" y="5.5" width="12" height="5" rx="1.2"/><path d="M4.5 5.5v5M7 5.5v5M9.5 5.5v5M12 5.5v5"/></svg>',
  pin: svg("M6 2.5h4l-.5 4 2 2v1h-7v-1l2-2zM8 9.5v4"),
  phone: '<svg viewBox="0 0 16 16"><rect x="4.5" y="1.5" width="7" height="13" rx="1.6"/><path d="M7 12.5h2"/></svg>',
  folder: svg("M2 4.5a1 1 0 0 1 1-1h3.2l1.5 1.5H13a1 1 0 0 1 1 1V12a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1z"),
  bolt: svg("M9 1.5 3.5 9H8l-1 5.5L12.5 7H8z"),
  down: svg("M4 4l8 8M12 4l-8 8"),
  trash: svg("M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.6 8.1a1 1 0 0 0 1 .9h3.8a1 1 0 0 0 1-.9l.6-8.1"),
  joinV: '<svg viewBox="0 0 16 16"><rect x="3.5" y="1.5" width="9" height="5.5" rx="1"/><rect x="3.5" y="9" width="9" height="5.5" rx="1"/></svg>',
  joinH: '<svg viewBox="0 0 16 16"><rect x="1.5" y="3.5" width="5.5" height="9" rx="1"/><rect x="9" y="3.5" width="5.5" height="9" rx="1"/></svg>',
  none: '<svg viewBox="0 0 16 16"><rect x="2.5" y="2.5" width="11" height="11" rx="3"/><path d="M5.5 8h5"/></svg>',
  pick: '<svg viewBox="0 0 16 16"><rect x="2.5" y="2.5" width="11" height="11" rx="3"/><path d="M5.5 8.2 7.2 10l3.3-4"/></svg>',
};
const CROSS =
  '<svg viewBox="0 0 10 10"><path d="M1.5 1.5l7 7M8.5 1.5l-7 7" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/></svg>';
const CHECK =
  '<svg viewBox="0 0 12 12" width="10" height="10"><path d="M2 6.5l2.6 2.6L10 3.5" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>';

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

function makePeg(data) {
  const el = document.createElement("div");
  el.className = "peg arriving";
  const media =
    data.kind === "video"
      ? `<video muted loop playsinline preload="metadata"></video><span class="badge">▶</span>`
      : `<img alt="" draggable="false" />`;
  el.innerHTML = `
    <div class="swing">
      <div class="pin"></div>
      <div class="card">
        ${media}
        <button class="x" aria-label="Take down">${CROSS}</button>
        <div class="copied">${CHECK}<span>Copied</span></div>
      </div>
    </div>`;
  const peg = {
    el,
    swingEl: el.querySelector(".swing"),
    card: el.querySelector(".card"),
    media: el.querySelector("img, video"),
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

function sizeMedia(peg, w, h) {
  const s = photoSize(w, h);
  peg.media.style.width = `${s.w}px`;
  peg.media.style.height = `${s.h}px`;
}

function update(peg, data) {
  const old = peg.data;
  if (data.kind === "video") {
    if (!old) {
      peg.media.src = convertFileSrc(data.path);
      if (data.thumb) peg.media.poster = data.thumb;
      sizeMedia(peg, data.width || 16, data.height || 9);
      // The first frame stands in for a thumbnail.
      peg.media.addEventListener("loadedmetadata", () => {
        sizeMedia(peg, peg.media.videoWidth, peg.media.videoHeight);
        if (!data.thumb) peg.media.currentTime = Math.min(0.5, peg.media.duration / 2 || 0);
        settleFor(600);
      });
    }
  } else if (!old || old.thumb !== data.thumb) {
    peg.media.src = data.thumb;
    sizeMedia(peg, data.width, data.height);
  }
  peg.media.title = data.name;
  peg.el.classList.toggle("pinned", data.pinned);
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
  for (const id of [...selected]) if (!pegs.has(id)) selected.delete(id);
  if (!pegs.has(focusId)) focusId = 0;
  paintSelection();
}

function setItems(items, { arrive = true } = {}) {
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
      if (arrive) {
        // Drops onto the line on the next frame, then swings to rest.
        requestAnimationFrame(() => requestAnimationFrame(() => fresh.el.classList.remove("arriving")));
      } else {
        fresh.el.classList.remove("arriving");
        fresh.angle = 0;
      }
    }
  }
  order = ids;
  layout();
  animate();
  settleFor(650);
}

/** Pulled off the line: it falls and fades. */
function fall(id, peg) {
  pegs.delete(id);
  if (actionsFor === id) closeActions();
  if (qrFor === id) closeQr();
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

/** The photos an action applies to: the selection when the photo is in it. */
function targets(id) {
  return selected.has(id) && selected.size > 1 ? order.filter((i) => selected.has(i)) : [id];
}

function toggleSelected(id) {
  if (selected.has(id)) selected.delete(id);
  else selected.add(id);
  paintSelection();
  if (selected.size > 1) showToast(`${selected.size} picked. Right click one to join them`);
}

function paintSelection() {
  for (const [id, peg] of pegs) {
    peg.el.classList.toggle("selected", selected.has(id));
    peg.el.classList.toggle("focused", keyboardOn && id === focusId);
  }
}

function wireGestures(peg, id) {
  const { card } = peg;
  let down = null;
  let pressTimer = 0;

  const cancelPress = () => {
    clearTimeout(pressTimer);
    card.classList.remove("pressing");
  };

  card.addEventListener("pointerdown", (e) => {
    invoke("activity");
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
    const ids = targets(id);
    for (const i of ids) pegs.get(i)?.card.classList.add("dragging");
    invoke("start_drag", { ids }).catch((err) => {
      console.warn("drag failed", err);
      endDrag();
    });
  });

  card.addEventListener("pointerup", (e) => {
    if (!down) return;
    const wasLong = down.long;
    down = null;
    cancelPress();
    if (wasLong || e.target.closest(".x")) return;
    focusId = id;
    if (e.ctrlKey || e.metaKey) {
      toggleSelected(id);
      return;
    }
    if (e.altKey) {
      invoke("copy_text", { id });
      return;
    }
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

  // Recordings play and GIFs move while the pointer is on them.
  card.addEventListener("pointerenter", () => {
    const d = peg.data;
    if (d.kind === "video") peg.media.play().catch(() => {});
    if (d.kind === "gif") peg.media.src = convertFileSrc(d.path);
  });
  card.addEventListener("pointerleave", () => {
    const d = peg.data;
    if (d.kind === "video") peg.media.pause();
    if (d.kind === "gif") peg.media.src = d.thumb;
  });

  card.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    cancelPress();
    down = null;
    focusId = id;
    if (settings.platform === "linux") {
      openActions(id);
    } else {
      invoke("menu", { id, selected: targets(id), x: e.clientX, y: e.clientY });
    }
  });

  card.querySelector(".x").addEventListener("click", (e) => {
    e.stopPropagation();
    invoke("take_down", { ids: [id] });
  });
}

function endDrag() {
  dragging = false;
  for (const peg of pegs.values()) peg.card.classList.remove("dragging");
  reportShapes(true);
}

// ---------------------------------------------------------------- Actions

// Native menus opened from the web view get no pointer input on Wayland, so
// Linux gets this bar instead. It also shows what keys do what.
let actionsFor = 0;

function actionList(id) {
  const ids = selected.size > 1 && selected.has(id) ? targets(id) : [id];
  if (ids.length > 1) {
    return [
      ["joinV", `Join ${ids.length} top to bottom (J)`, () => invoke("join", { ids, vertical: true })],
      ["joinH", `Join ${ids.length} side by side (Shift J)`, () => invoke("join", { ids, vertical: false })],
      "|",
      ["down", `Take down ${ids.length}`, () => invoke("take_down", { ids })],
      ["pick", selected.has(id) ? "Unpick this one" : "Pick this one too", () => toggleSelected(id)],
      ["none", "Clear picks", () => (selected.clear(), paintSelection())],
    ];
  }
  const pinned = pegs.get(id)?.data.pinned;
  const list = [
    ["copy", "Copy (Enter)", () => invoke("copy", { id })],
    ["text", "Copy text (T)", () => invoke("copy_text", { id })],
    ["open", "Open (O)", () => invoke("open", { id })],
    ["edit", "Mark up (E)", () => invoke("edit", { id })],
    ["redact", "Redact (R)", () => invoke("redact", { id })],
    ["pin", pinned ? "Don't keep (K)" : "Keep (K)", () => invoke("pin", { id })],
    ["phone", "Send to phone (S)", () => shareToPhone(id)],
    ["folder", "Show in folder (F)", () => invoke("show_in_folder", { id })],
    ["pick", "Pick, to join or drag several (Space, or Ctrl-click)", () => toggleSelected(id)],
  ];
  if (settings.actions.length) {
    list.push("|");
    settings.actions.forEach((name, index) => list.push(["bolt", name, () => invoke("run_action", { id, index })]));
  }
  list.push("|");
  list.push(["down", "Take down (X)", () => invoke("take_down", { ids: [id] })]);
  list.push(["trash", "Move to trash (Shift Delete)", () => invoke("trash", { ids: [id] }), "danger"]);
  return list;
}

function openActions(id) {
  const peg = pegs.get(id);
  if (!peg) return;
  closeQr();
  actionsFor = id;
  actionsEl.innerHTML = "";
  for (const entry of actionList(id)) {
    if (entry === "|") {
      actionsEl.appendChild(document.createElement("i"));
      continue;
    }
    const [icon, label, run, cls] = entry;
    const b = document.createElement("button");
    b.innerHTML = ICON[icon];
    b.title = label;
    b.setAttribute("aria-label", label);
    if (cls) b.className = cls;
    b.addEventListener("click", (e) => {
      e.stopPropagation();
      closeActions();
      run();
    });
    actionsEl.appendChild(b);
  }
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

document.addEventListener("pointerdown", (e) => {
  if (actionsFor && !e.target.closest("#actions")) closeActions();
  if (qrFor && !e.target.closest("#qr")) closeQr();
});

// ---------------------------------------------------------------- Phone

let qrFor = 0;

async function shareToPhone(id) {
  try {
    showQr(await invoke("share", { id }));
  } catch (err) {
    showToast(String(err));
  }
}

function showQr(shared) {
  const peg = pegs.get(shared.id);
  if (!peg) return;
  closeActions();
  qrFor = shared.id;
  qrEl.querySelector(".code").innerHTML = shared.svg;
  qrEl.querySelector(".words span").textContent =
    `Same Wi-Fi only. The link stops working in ${shared.minutes} minutes.`;
  qrEl.classList.add("open");
  const card = peg.card.getBoundingClientRect();
  const w = qrEl.offsetWidth;
  const right = card.right + 14;
  const left = right + w < window.innerWidth - 8 ? right : card.left - 14 - w;
  qrEl.style.left = `${Math.max(8, left)}px`;
  qrEl.style.top = `${Math.max(14, Math.min(card.top - 6, window.innerHeight - qrEl.offsetHeight - 6))}px`;
  reportShapes(true);
}

function closeQr() {
  if (!qrFor) return;
  qrFor = 0;
  qrEl.classList.remove("open");
  reportShapes(true);
}

// ---------------------------------------------------------------- Keyboard

// Open the line with the shortcut and it takes the keyboard until it tucks
// away. Arrows (or H and L) move, and single keys act on the photo.
let keyboardOn = false;

function focused() {
  if (!pegs.has(focusId)) focusId = order[order.length - 1] || 0;
  return focusId;
}

function moveFocus(step) {
  if (!order.length) return;
  const i = Math.max(0, order.indexOf(focused()));
  focusId = order[(i + step + order.length) % order.length];
  paintSelection();
  const peg = pegs.get(focusId);
  if (peg) nudge(peg, 1.2);
}

document.addEventListener("keydown", (e) => {
  invoke("activity");
  const id = focused();
  const key = e.key;
  if (key === "Escape") {
    if (actionsFor || qrFor) return closeActions(), closeQr();
    return invoke("hide");
  }
  if (key === "Tab") {
    e.preventDefault();
    return invoke("next_line", { delta: e.shiftKey ? -1 : 1 });
  }
  if (key === "/") return e.preventDefault(), invoke("open_search");
  if (key === "?") return showToast("←→ move · Enter copy · T text · O open · E mark up · R redact · K keep · S phone · Space pick · J join · X take down");
  if (key === "ArrowLeft" || key === "h") return moveFocus(-1);
  if (key === "ArrowRight" || key === "l") return moveFocus(1);
  if (/^[1-9]$/.test(key) && order[Number(key) - 1]) {
    focusId = order[Number(key) - 1];
    paintSelection();
    return invoke("copy", { id: focusId });
  }
  if (!id) return;
  const ids = targets(id);
  switch (key.toLowerCase()) {
    case "enter":
    case "c":
      return invoke("copy", { id });
    case "t":
      return invoke("copy_text", { id });
    case "o":
      return invoke("open", { id });
    case "e":
      return invoke("edit", { id });
    case "r":
      return invoke("redact", { id });
    case "k":
    case "p":
      return invoke("pin", { id });
    case "s":
      return shareToPhone(id);
    case "f":
      return invoke("show_in_folder", { id });
    case " ":
      e.preventDefault();
      return toggleSelected(id);
    case "j":
      return invoke("join", { ids: order.filter((i) => selected.has(i)), vertical: !e.shiftKey });
    case "x":
    case "backspace":
    case "delete":
      return e.shiftKey ? invoke("trash", { ids }) : invoke("take_down", { ids });
    case "a":
      if (e.ctrlKey || e.metaKey) {
        e.preventDefault();
        order.forEach((i) => selected.add(i));
        paintSelection();
      }
  }
});

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

/** Everything that should take clicks: photos, and any open panel. */
function shapes() {
  const list = [...pegs.values()].map((p) => rectOf(p.card, 2));
  if (actionsFor) list.push(rectOf(actionsEl, 2));
  if (qrFor) list.push(rectOf(qrEl, 2));
  if (settings.line.names.length > 1) list.push(rectOf(tagEl, 2));
  return list;
}

function reportShapes(force = false) {
  const cards = shapes();
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
    const parts = order.length ? cards : [rectOf(hintEl, 6), ...cards];
    if (parts.length) {
      const left = Math.min(...parts.map((r) => r.x)) - 12;
      const right = Math.max(...parts.map((r) => r.x + r.w)) + 12;
      const bottom = Math.max(...parts.map((r) => r.y + r.h)) + 20;
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

// ---------------------------------------------------------------- Messages

let toastTimer = 0;
function showToast(text) {
  toastEl.textContent = text;
  toastEl.classList.add("shown");
  toastEl.style.top = `${window.innerHeight - 34}px`;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => toastEl.classList.remove("shown"), 2600);
}

function showLine(info) {
  settings.line = info;
  const many = info.names.length > 1;
  tagEl.hidden = !many;
  tagEl.textContent = many ? `${info.names[info.active]} · ${info.active + 1}/${info.names.length}` : "";
  selected.clear();
  reportShapes(true);
}
tagEl.addEventListener("click", () => invoke("next_line", { delta: 1 }));

// ---------------------------------------------------------------- Theme

/** Omarchy theme colours, or the system's light or dark. */
function applyTheme(theme) {
  const root = document.documentElement;
  if (theme.mode === "light" || theme.mode === "dark") root.dataset.theme = theme.mode;
  else delete root.dataset.theme;
  const set = (name, value) => (value ? root.style.setProperty(name, value) : root.style.removeProperty(name));
  set("--glass", theme.background && hexAlpha(theme.background, 0.66));
  set("--ink", theme.foreground);
  set("--ink-soft", theme.foreground && hexAlpha(theme.foreground, 0.72));
  set("--accent", theme.accent);
}

function hexAlpha(hex, a) {
  const m = /^#?([0-9a-f]{6})/i.exec(hex);
  if (!m) return hex;
  const n = parseInt(m[1], 16);
  return `rgba(${n >> 16}, ${(n >> 8) & 255}, ${n & 255}, ${a})`;
}

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
  if (!v) {
    closeActions();
    closeQr();
  }
  lineEl.classList.toggle("down", v);
  settleFor(600);
  reportShapes(true);
}

async function init() {
  drawRope();
  settings = await invoke("settings");
  document.documentElement.dataset.platform = settings.platform;
  applyTheme(settings.theme);
  showLine(settings.line);

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
  await listen("busy", (e) => pegs.get(e.payload)?.card.classList.add("busy"));
  await listen("idle", (e) => pegs.get(e.payload)?.card.classList.remove("busy"));
  await listen("toast", (e) => showToast(e.payload));
  await listen("shared", (e) => showQr(e.payload));
  await listen("line", (e) => showLine(e.payload));
  await listen("theme", (e) => applyTheme(e.payload));
  await listen("keyboard", (e) => {
    keyboardOn = e.payload;
    if (keyboardOn) focusId = order[order.length - 1] || 0;
    paintSelection();
  });
  await listen("drag-end", () => endDrag());
  await listen("settings", async () => {
    settings = await invoke("settings");
    applyTheme(settings.theme);
    reportShapes(true);
  });

  // What was already hanging comes back without the drop-in.
  setItems(await invoke("items"), { arrive: false });
  setRevealed(settings.revealed);
  setTimeout(breeze, 9000);
}

window.addEventListener("resize", () => {
  drawRope();
  layout();
  settleFor(600);
});

init();
