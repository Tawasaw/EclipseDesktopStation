// On-screen gamepad driven by the keyboard (or mouse), for users without a
// USB controller. The backend registers it as a normal gamepad, so it can be
// assigned to gamepad1/gamepad2 from the Gamepads bar like any other.

// [KeyboardEvent.code, control, key label]. Codes are physical key positions,
// so the layout stays the same on non-QWERTY keyboards.
const KEY_BINDINGS = [
  ["KeyW", "ls_up", "W"], ["KeyA", "ls_left", "A"], ["KeyS", "ls_down", "S"], ["KeyD", "ls_right", "D"],
  ["KeyF", "left_stick_button", "F"],
  ["KeyI", "rs_up", "I"], ["KeyJ", "rs_left", "J"], ["KeyK", "rs_down", "K"], ["KeyL", "rs_right", "L"],
  ["KeyH", "right_stick_button", "H"],
  ["ArrowUp", "dpad_up", "↑"], ["ArrowDown", "dpad_down", "↓"],
  ["ArrowLeft", "dpad_left", "←"], ["ArrowRight", "dpad_right", "→"],
  ["Space", "a", "Space"], ["KeyB", "b", "B"], ["KeyX", "x", "X"], ["KeyY", "y", "Y"],
  ["KeyQ", "left_bumper", "Q"], ["KeyE", "left_trigger", "E"],
  ["KeyO", "right_bumper", "O"], ["KeyU", "right_trigger", "U"],
  ["Backspace", "back", "⌫"], ["Enter", "start", "Enter"], ["KeyG", "guide", "G"],
  ["ShiftLeft", "precision", "Shift"], ["ShiftRight", "precision", "Shift"],
];
const CONTROL_BY_CODE = new Map(KEY_BINDINGS.map(([code, control]) => [code, control]));

const BUTTONS = [
  "a", "b", "x", "y", "back", "start", "guide", "left_bumper", "right_bumper",
  "left_stick_button", "right_stick_button", "dpad_up", "dpad_down", "dpad_left", "dpad_right",
];

/** Fraction of full travel while Shift is held, for fine positioning. */
const PRECISION_SCALE = 0.5;
/** Resend interval; the backend zeroes the pad after 500 ms of silence. */
const HEARTBEAT_MS = 150;
/** How far a stick cap moves on screen at full deflection (SVG units). */
const STICK_TRAVEL = 16;
const STICK_GRAB_RADIUS = 40;

const STICKS = {
  left: { cx: 285, cy: 350 },
  right: { cx: 475, cy: 350 },
};

const LEGEND = [
  ["Left stick", "W A S D", "Click: F"],
  ["Right stick", "I J K L", "Click: H"],
  ["D-pad", "Arrow keys", ""],
  ["A / B / X / Y", "Space / B / X / Y", ""],
  ["LB / LT", "Q / E", ""],
  ["RB / RT", "O / U", ""],
  ["Back / Start", "⌫ / Enter", "Guide: G"],
  ["Half speed", "Hold Shift", ""],
];

function svgMarkup() {
  const face = (control, cx, cy, color, label) => `
    <g class="vp-ctl vp-face" data-control="${control}">
      <circle cx="${cx}" cy="${cy}" r="27" class="vp-face-well" />
      <circle cx="${cx}" cy="${cy}" r="22" fill="${color}" class="vp-press" />
      <circle cx="${cx - 6}" cy="${cy - 7}" r="9" fill="#fff" opacity="0.18" />
      <text x="${cx}" y="${cy + 7}" class="vp-face-letter">${label}</text>
    </g>`;
  const small = (control, x, y, label, labelY) => `
    <g class="vp-ctl" data-control="${control}">
      <rect x="${x}" y="${y}" width="40" height="20" rx="9" class="vp-small vp-press" />
    </g>
    <text x="${x + 20}" y="${labelY}" class="vp-label">${label}</text>`;
  const shoulder = (side, x, hintX, anchor) => `
    <g class="vp-ctl" data-control="${side}_trigger">
      <rect x="${x}" y="40" width="120" height="34" rx="14" class="vp-trigger vp-press" />
      <rect x="${x}" y="40" width="0" height="34" rx="14" class="vp-trigger-fill" data-fill="${side}_trigger" />
      <text x="${x + 60}" y="62" class="vp-label">${side === "left" ? "LT" : "RT"}</text>
    </g>
    <g class="vp-ctl" data-control="${side}_bumper">
      <rect x="${x - 6}" y="70" width="132" height="34" rx="14" class="vp-bumper vp-press" />
      <text x="${x + 60}" y="90" class="vp-label">${side === "left" ? "LB" : "RB"}</text>
    </g>
    <text x="${hintX}" y="62" class="vp-hint" text-anchor="${anchor}">${side === "left" ? "E" : "U"}</text>
    <text x="${hintX}" y="92" class="vp-hint" text-anchor="${anchor}">${side === "left" ? "Q" : "O"}</text>`;
  const stick = (side, hint) => {
    const { cx, cy } = STICKS[side];
    return `
      <circle cx="${cx}" cy="${cy}" r="66" class="vp-well" />
      <g class="vp-ctl vp-stick" data-control="${side}_stick_button" data-stick="${side}">
        <circle cx="${cx}" cy="${cy}" r="50" class="vp-stick-base" />
        <g data-cap="${side}">
          <circle cx="${cx}" cy="${cy}" r="40" class="vp-stick-cap vp-press" />
          <circle cx="${cx}" cy="${cy}" r="30" class="vp-stick-top" />
        </g>
      </g>
      <text x="${cx}" y="${cy + 82}" class="vp-hint">${hint}</text>`;
  };
  const dpadArm = (control, x, y, w, h) =>
    `<rect class="vp-ctl vp-dpad-arm vp-press" data-control="${control}" x="${x}" y="${y}" width="${w}" height="${h}" rx="6" />`;

  return `
  <svg class="vpad-svg" viewBox="0 0 760 470" role="img" aria-label="Virtual gamepad">
    ${shoulder("left", 140, 128, "end")}
    ${shoulder("right", 500, 632, "start")}

    <ellipse cx="118" cy="330" rx="78" ry="128" transform="rotate(18 118 330)" class="vp-grip" />
    <ellipse cx="642" cy="330" rx="78" ry="128" transform="rotate(-18 642 330)" class="vp-grip" />

    <g class="vp-body">
      <circle cx="200" cy="232" r="142" />
      <circle cx="560" cy="232" r="142" />
      <rect x="200" y="112" width="360" height="190" />
      <circle cx="285" cy="345" r="92" />
      <circle cx="475" cy="345" r="92" />
      <rect x="285" y="280" width="190" height="120" />
    </g>
    <circle cx="200" cy="236" r="108" class="vp-ring" />
    <circle cx="572" cy="240" r="104" class="vp-ring" />

    <circle cx="200" cy="236" r="62" class="vp-dpad-base" />
    ${dpadArm("dpad_up", 182, 182, 36, 42)}
    ${dpadArm("dpad_down", 182, 248, 36, 42)}
    ${dpadArm("dpad_left", 146, 218, 42, 36)}
    ${dpadArm("dpad_right", 212, 218, 42, 36)}
    <rect x="182" y="218" width="36" height="36" class="vp-dpad-center" />
    <text x="200" y="322" class="vp-hint">Arrows</text>

    ${small("back", 285, 172, "BACK", 210)}
    ${small("start", 435, 172, "START", 210)}
    <rect x="298" y="238" width="40" height="20" rx="9" class="vp-small vp-inert">
      <title>Mode (not emulated)</title>
    </rect>
    <text x="318" y="274" class="vp-label">MODE</text>
    <circle cx="352" cy="248" r="3" class="vp-led" />
    <text x="305" y="166" class="vp-hint">⌫</text>
    <text x="455" y="166" class="vp-hint">Enter</text>

    <g class="vp-ctl vp-badge" data-control="guide">
      <rect x="342" y="186" width="76" height="54" rx="14" class="vp-press" />
      <text x="380" y="212" class="vp-badge-title">FTC</text>
      <text x="380" y="228" class="vp-badge-sub">VIRTUAL · G</text>
    </g>

    ${face("y", 572, 190, "#f0a01a", "Y")}
    ${face("x", 522, 240, "#2756c4", "X")}
    ${face("b", 622, 240, "#d3302c", "B")}
    ${face("a", 572, 290, "#3ea644", "A")}
    <text x="606" y="176" class="vp-hint" text-anchor="start">Y</text>
    <text x="490" y="226" class="vp-hint" text-anchor="end">X</text>
    <text x="654" y="226" class="vp-hint" text-anchor="start">B</text>
    <text x="606" y="318" class="vp-hint" text-anchor="start">Space</text>

    ${stick("left", "WASD · F")}
    ${stick("right", "IJKL · H")}
  </svg>`;
}

export function createVirtualGamepad({ invoke, onStatus }) {
  const root = document.createElement("div");
  root.className = "vpad-window hidden";
  root.setAttribute("role", "dialog");
  root.setAttribute("aria-label", "Virtual gamepad");
  root.innerHTML = `
    <header class="vpad-titlebar">
      <strong>Virtual Gamepad</strong>
      <span class="vpad-status"></span>
      <button type="button" class="vpad-close" aria-label="Close virtual gamepad">×</button>
    </header>
    ${svgMarkup()}
    <dl class="vpad-legend">
      ${LEGEND.map(([what, keys, extra]) => `
        <div><dt>${what}</dt><dd><kbd>${keys}</kbd>${extra ? ` <span>${extra}</span>` : ""}</dd></div>`).join("")}
    </dl>`;
  document.body.append(root);

  const statusEl = root.querySelector(".vpad-status");
  const svg = root.querySelector("svg");
  const controlEls = [...root.querySelectorAll("[data-control]")];
  const fills = [...root.querySelectorAll("[data-fill]")];
  const caps = Object.fromEntries([...root.querySelectorAll("[data-cap]")].map((el) => [el.dataset.cap, el]));

  const view = {
    open: false,
    padId: null,
    user: null,
    /** Physical keys held, as KeyboardEvent.code. */
    keys: new Set(),
    /** Controls held with the mouse, keyed by pointerId. */
    pointerControls: new Map(),
    /** Sticks being dragged with the mouse: side -> {pointerId, x, y}. */
    drags: new Map(),
    lastSent: "",
    heartbeat: null,
    paused: "",
  };

  function held(control) {
    for (const code of view.keys) if (CONTROL_BY_CODE.get(code) === control) return true;
    for (const value of view.pointerControls.values()) if (value === control) return true;
    return false;
  }

  function stickFromKeys(prefix, scale) {
    let x = (held(`${prefix}_right`) ? 1 : 0) - (held(`${prefix}_left`) ? 1 : 0);
    // FTC convention: pushing the stick up is negative.
    let y = (held(`${prefix}_down`) ? 1 : 0) - (held(`${prefix}_up`) ? 1 : 0);
    // Diagonals stay on the unit circle, like a real stick.
    const length = Math.hypot(x, y);
    if (length > 1) {
      x /= length;
      y /= length;
    }
    return [x * scale, y * scale];
  }

  function computeState() {
    const scale = held("precision") ? PRECISION_SCALE : 1;
    let [lx, ly] = stickFromKeys("ls", scale);
    let [rx, ry] = stickFromKeys("rs", scale);
    const left = view.drags.get("left");
    if (left) [lx, ly] = [left.x, left.y];
    const right = view.drags.get("right");
    if (right) [rx, ry] = [right.x, right.y];
    const round = (value) => Math.round(value * 1000) / 1000 || 0;
    const pad = {
      left_stick_x: round(lx),
      left_stick_y: round(ly),
      right_stick_x: round(rx),
      right_stick_y: round(ry),
      left_trigger: held("left_trigger") ? scale : 0,
      right_trigger: held("right_trigger") ? scale : 0,
    };
    for (const button of BUTTONS) pad[button] = held(button);
    return pad;
  }

  function render(pad) {
    for (const el of controlEls) {
      const control = el.dataset.control;
      const value = pad[control];
      el.classList.toggle("on", value === true || value > 0);
    }
    for (const el of fills) {
      el.setAttribute("width", String(120 * pad[el.dataset.fill]));
    }
    const move = (side, x, y) =>
      caps[side].setAttribute("transform", `translate(${x * STICK_TRAVEL} ${y * STICK_TRAVEL})`);
    move("left", pad.left_stick_x, pad.left_stick_y);
    move("right", pad.right_stick_x, pad.right_stick_y);
  }

  function send(force = false) {
    if (!view.open) return;
    const pad = computeState();
    render(pad);
    const serialized = JSON.stringify(pad);
    if (!force && serialized === view.lastSent) return;
    view.lastSent = serialized;
    invoke("update_virtual_gamepad", { pad }).catch(() => {
      view.lastSent = "";
    });
  }

  function releaseAll(reason) {
    view.keys.clear();
    view.pointerControls.clear();
    view.drags.clear();
    if (reason !== undefined) view.paused = reason;
    renderStatusLine();
    send();
  }

  function isTextField(target) {
    if (target instanceof HTMLTextAreaElement) return true;
    if (!(target instanceof HTMLInputElement)) return false;
    return !["checkbox", "radio", "button", "submit", "file", "range"].includes(target.type);
  }

  function onKeyDown(event) {
    if (!view.open) return;
    // macOS drops keyup for keys released while Cmd is held; start clean.
    if (event.key === "Meta") {
      releaseAll();
      return;
    }
    if (event.metaKey || event.ctrlKey || event.altKey) return;
    if (isTextField(event.target)) return;
    if (!CONTROL_BY_CODE.has(event.code)) return;
    // Keep Space/Enter from clicking the focused button, arrows from
    // scrolling or moving the OpMode selection.
    event.preventDefault();
    event.stopPropagation();
    if (view.paused) {
      view.paused = "";
      renderStatusLine();
    }
    view.keys.add(event.code);
    send();
  }

  function onKeyUp(event) {
    if (!view.open || !view.keys.has(event.code)) return;
    event.preventDefault();
    event.stopPropagation();
    view.keys.delete(event.code);
    send();
  }

  function svgPoint(event) {
    const matrix = svg.getScreenCTM();
    if (!matrix) return null;
    const point = new DOMPoint(event.clientX, event.clientY).matrixTransform(matrix.inverse());
    return point;
  }

  function updateDrag(side, event) {
    const point = svgPoint(event);
    if (!point) return;
    const { cx, cy } = STICKS[side];
    let x = (point.x - cx) / STICK_GRAB_RADIUS;
    let y = (point.y - cy) / STICK_GRAB_RADIUS;
    const length = Math.hypot(x, y);
    if (length > 1) {
      x /= length;
      y /= length;
    }
    view.drags.set(side, { pointerId: event.pointerId, x, y });
  }

  svg.addEventListener("pointerdown", (event) => {
    const target = event.target.closest("[data-control]");
    if (!target || event.button !== 0) return;
    event.preventDefault();
    svg.setPointerCapture(event.pointerId);
    const stickSide = target.dataset.stick;
    if (stickSide) {
      // Dragging a stick moves it; a click without moving does nothing.
      updateDrag(stickSide, event);
    } else {
      view.pointerControls.set(event.pointerId, target.dataset.control);
    }
    send();
  });

  svg.addEventListener("pointermove", (event) => {
    for (const [side, drag] of view.drags) {
      if (drag.pointerId === event.pointerId) {
        updateDrag(side, event);
        send();
      }
    }
  });

  const endPointer = (event) => {
    view.pointerControls.delete(event.pointerId);
    for (const [side, drag] of view.drags) {
      if (drag.pointerId === event.pointerId) view.drags.delete(side);
    }
    send();
  };
  svg.addEventListener("pointerup", endPointer);
  svg.addEventListener("pointercancel", endPointer);
  svg.addEventListener("lostpointercapture", endPointer);

  // Capture phase, so mapped keys reach the gamepad before page controls.
  window.addEventListener("keydown", onKeyDown, true);
  window.addEventListener("keyup", onKeyUp, true);
  window.addEventListener("blur", () => {
    if (view.open) releaseAll("Paused: click this window to keep driving");
  });
  window.addEventListener("focus", () => {
    if (view.open && view.paused) {
      view.paused = "";
      renderStatusLine();
    }
  });
  document.addEventListener("visibilitychange", () => {
    if (view.open && document.hidden) releaseAll("Paused: window hidden");
  });
  document.addEventListener("focusin", (event) => {
    if (view.open && isTextField(event.target)) releaseAll();
    renderStatusLine();
  });
  document.addEventListener("focusout", () => window.setTimeout(renderStatusLine));

  // Drag the window by its title bar.
  const titlebar = root.querySelector(".vpad-titlebar");
  titlebar.addEventListener("pointerdown", (event) => {
    if (event.target.closest("button") || event.button !== 0) return;
    const rect = root.getBoundingClientRect();
    const offsetX = event.clientX - rect.left;
    const offsetY = event.clientY - rect.top;
    titlebar.setPointerCapture(event.pointerId);
    const move = (moveEvent) => {
      const maxX = window.innerWidth - rect.width;
      const maxY = window.innerHeight - rect.height;
      root.style.left = `${Math.max(0, Math.min(maxX, moveEvent.clientX - offsetX))}px`;
      root.style.top = `${Math.max(0, Math.min(maxY, moveEvent.clientY - offsetY))}px`;
      root.style.right = "auto";
      root.style.bottom = "auto";
    };
    const stop = () => {
      titlebar.removeEventListener("pointermove", move);
      titlebar.removeEventListener("pointerup", stop);
      titlebar.removeEventListener("pointercancel", stop);
    };
    titlebar.addEventListener("pointermove", move);
    titlebar.addEventListener("pointerup", stop);
    titlebar.addEventListener("pointercancel", stop);
  });

  root.querySelector(".vpad-close").addEventListener("click", () => close());

  function renderStatusLine() {
    let text;
    let kind = "ok";
    if (view.paused) {
      text = view.paused;
      kind = "warn";
    } else if (isTextField(document.activeElement)) {
      text = "Typing in a text field: keys not captured";
      kind = "warn";
    } else if (view.user) {
      text = `Driving gamepad${view.user}`;
    } else {
      text = "Off: assign it in the Gamepads bar";
      kind = "warn";
    }
    statusEl.textContent = text;
    statusEl.dataset.kind = kind;
  }

  async function open() {
    if (view.open) return;
    const status = await invoke("open_virtual_gamepad");
    view.open = true;
    view.paused = "";
    view.lastSent = "";
    root.classList.remove("hidden");
    // Don't leave focus in the IP field, or keys would type into it.
    if (isTextField(document.activeElement)) document.activeElement.blur();
    view.heartbeat = window.setInterval(() => send(true), HEARTBEAT_MS);
    onStatus?.(status);
    send(true);
  }

  async function close() {
    if (!view.open) return;
    view.open = false;
    window.clearInterval(view.heartbeat);
    view.keys.clear();
    view.pointerControls.clear();
    view.drags.clear();
    root.classList.add("hidden");
    onStatus?.(await invoke("close_virtual_gamepad"));
  }

  /** Feed the polled gamepad status in, to show the current assignment. */
  function syncStatus(status) {
    const pad = (status.pads ?? []).find((candidate) => candidate.is_virtual);
    view.padId = pad?.id ?? null;
    view.user = pad?.user ?? null;
    if (view.open) renderStatusLine();
  }

  render(computeState());

  return {
    open,
    close,
    toggle: () => (view.open ? close() : open()),
    isOpen: () => view.open,
    syncStatus,
  };
}
