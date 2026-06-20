import { invoke } from "@tauri-apps/api/core";
import "./styles.css";

const state = {
  connected: false,
  snapshot: null,
  selectedOpMode: "",
  selectedConfig: "",
  timerStart: null,
  timerInterval: null,
  uploadXml: null,
  uploadName: "",
  uploadPath: "",
  logPath: "",
  opModeSignature: "",
  configSignature: "",
  pendingDeleteConfig: null,
  deleteResetTimer: null,
};

const app = document.querySelector("#app");

app.innerHTML = `
  <main class="shell">
    <section class="topbar">
      <div class="brand">
        <div class="mark">FTC</div>
        <div>
          <h1>EclipseDesktopStation</h1>
          <p id="statusLine">Disconnected</p>
          <p id="logPath" class="log-path"></p>
        </div>
      </div>
      <form id="connectForm" class="connect">
        <input id="robotIp" value="192.168.43.1" aria-label="Robot Controller IP" />
        <button type="submit">Connect</button>
        <button id="disconnectBtn" type="button">Disconnect</button>
      </form>
    </section>

    <div id="robotBanner" class="robot-banner hidden" role="alert">
      <div class="banner-text">
        <strong id="bannerTitle"></strong>
        <pre id="bannerBody"></pre>
      </div>
      <button id="bannerDismiss" type="button">Dismiss</button>
    </div>

    <section class="status-grid">
      <div class="status-tile good">
        <span>Robot</span>
        <strong id="robotState">Unknown</strong>
      </div>
      <div class="status-tile">
        <span>Config</span>
        <strong id="activeConfig">None</strong>
      </div>
      <div class="status-tile battery">
        <span>Battery</span>
        <strong id="battery">--</strong>
      </div>
      <div class="status-tile">
        <span>Timer</span>
        <strong id="timer">0:00</strong>
      </div>
    </section>

    <section class="workspace">
      <aside class="control-panel">
        <div class="panel-section opmode-section">
          <div class="section-head">
            <h2>OpMode</h2>
            <button id="refreshBtn" type="button">Refresh</button>
          </div>
          <div id="opModeList" class="opmode-list" tabindex="0" role="listbox" aria-label="OpMode list"></div>
          <div class="lifecycle">
            <button id="initBtn" type="button">Init</button>
            <button id="startBtn" type="button">Start</button>
            <button id="stopBtn" type="button">Stop</button>
          </div>
        </div>

        <div class="panel-section">
          <div class="section-head">
            <h2>XML Config</h2>
          </div>
          <select id="configSelect" aria-label="Config list"></select>
          <div class="config-actions">
            <div class="config-row">
              <button id="downloadConfigBtn" type="button" disabled>Download</button>
              <button id="activateConfigBtn" type="button" disabled>Activate</button>
              <button id="deleteConfigBtn" type="button" disabled>Delete</button>
            </div>
            <label class="file-picker" for="uploadConfigInput">
              <span>Select local XML file</span>
              <input id="uploadConfigInput" class="file-input" type="file" accept=".xml,text/xml,application/xml" />
            </label>
            <button id="uploadConfigBtn" type="button" disabled>Upload and Activate</button>
          </div>
          <p id="configMessage" class="message"></p>
        </div>
      </aside>

      <section class="telemetry-panel">
        <div class="section-head">
          <h2>Telemetry</h2>
          <span id="peerInfo">No peer</span>
        </div>
        <div id="telemetryList" class="telemetry-list"></div>
      </section>
    </section>
  </main>
`;

const elements = {
  statusLine: document.querySelector("#statusLine"),
  logPath: document.querySelector("#logPath"),
  connectForm: document.querySelector("#connectForm"),
  robotIp: document.querySelector("#robotIp"),
  disconnectBtn: document.querySelector("#disconnectBtn"),
  robotState: document.querySelector("#robotState"),
  activeConfig: document.querySelector("#activeConfig"),
  battery: document.querySelector("#battery"),
  timer: document.querySelector("#timer"),
  refreshBtn: document.querySelector("#refreshBtn"),
  opModeList: document.querySelector("#opModeList"),
  initBtn: document.querySelector("#initBtn"),
  startBtn: document.querySelector("#startBtn"),
  stopBtn: document.querySelector("#stopBtn"),
  configSelect: document.querySelector("#configSelect"),
  downloadConfigBtn: document.querySelector("#downloadConfigBtn"),
  activateConfigBtn: document.querySelector("#activateConfigBtn"),
  deleteConfigBtn: document.querySelector("#deleteConfigBtn"),
  uploadConfigInput: document.querySelector("#uploadConfigInput"),
  uploadConfigBtn: document.querySelector("#uploadConfigBtn"),
  configMessage: document.querySelector("#configMessage"),
  telemetryList: document.querySelector("#telemetryList"),
  peerInfo: document.querySelector("#peerInfo"),
  shell: document.querySelector(".shell"),
  robotBanner: document.querySelector("#robotBanner"),
  bannerTitle: document.querySelector("#bannerTitle"),
  bannerBody: document.querySelector("#bannerBody"),
  bannerDismiss: document.querySelector("#bannerDismiss"),
};

elements.connectForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  await runAction(async () => {
    const snapshot = await invoke("connect_robot", { ip: elements.robotIp.value });
    state.connected = true;
    renderSnapshot(snapshot);
    startPolling();
  });
});

elements.disconnectBtn.addEventListener("click", async () => {
  await runAction(async () => {
    await invoke("disconnect_robot");
    state.connected = false;
    state.snapshot = null;
    stopTimer();
    renderDisconnected();
  });
});

elements.refreshBtn.addEventListener("click", () => runAction(() => invoke("refresh_robot")));

elements.bannerDismiss.addEventListener("click", async () => {
  hideBanner();
  try {
    await invoke("clear_robot_message");
  } catch {
    // ignore: nothing to clear if disconnected
  }
});

elements.opModeList.addEventListener("click", (event) => {
  const row = event.target.closest("[data-opmode]");
  if (!row) return;
  state.selectedOpMode = row.dataset.opmode;
  renderOpModeSelection(true);
});

elements.opModeList.addEventListener("keydown", (event) => {
  const modes = state.snapshot?.op_modes ?? [];
  if (!modes.length) return;
  const currentIndex = Math.max(
    0,
    modes.findIndex((mode) => mode.name === state.selectedOpMode),
  );
  let nextIndex = currentIndex;
  if (event.key === "ArrowDown") nextIndex = Math.min(modes.length - 1, currentIndex + 1);
  if (event.key === "ArrowUp") nextIndex = Math.max(0, currentIndex - 1);
  if (event.key === "Home") nextIndex = 0;
  if (event.key === "End") nextIndex = modes.length - 1;
  if (nextIndex !== currentIndex) {
    event.preventDefault();
    state.selectedOpMode = modes[nextIndex].name;
    renderOpModeSelection(true);
  }
});

elements.configSelect.addEventListener("change", () => {
  state.selectedConfig = elements.configSelect.value;
  resetDeleteButton();
  updateConfigButtons();
});

elements.initBtn.addEventListener("click", async () => {
  if (!state.selectedOpMode) return;
  await runAction(() => invoke("init_op_mode", { name: state.selectedOpMode }));
  stopTimer();
});

elements.startBtn.addEventListener("click", async () => {
  if (!state.selectedOpMode) return;
  await runAction(() => invoke("run_op_mode", { name: state.selectedOpMode }));
  startTimer();
});

elements.stopBtn.addEventListener("click", async () => {
  await runAction(() => invoke("stop_op_mode"));
  stopTimer();
});

elements.downloadConfigBtn.addEventListener("click", async () => {
  if (!state.selectedConfig) return;
  await runAction(async () => {
    const result = await invoke("download_config_xml", { configName: state.selectedConfig });
    setConfigMessage(`Saved ${result.path}`);
  });
});

elements.activateConfigBtn.addEventListener("click", async () => {
  if (!state.selectedConfig) return;
  const name = state.selectedConfig;
  await runAction(async () => {
    elements.activateConfigBtn.disabled = true;
    setConfigMessage(`Activating ${name}...`);
    const result = await invoke("activate_config", { configName: name });
    const active = result.active_config?.name;
    state.configSignature = ""; // force dropdown rebuild so the * moves to the new active config
    if (active && active.toLowerCase() === name.toLowerCase()) {
      setConfigMessage(`Activated ${active}`);
    } else {
      setConfigMessage(`Activation sent for ${name}; active config is ${active ?? "unknown"}`);
    }
    await invoke("refresh_robot");
  }, (message) => setConfigMessage(`Activate failed: ${message}`));
  updateConfigButtons();
});

elements.deleteConfigBtn.addEventListener("click", async () => {
  if (!state.selectedConfig) return;
  const name = state.selectedConfig;
  // First click arms the confirm; second click (within the timeout) deletes.
  if (state.pendingDeleteConfig !== name) {
    state.pendingDeleteConfig = name;
    elements.deleteConfigBtn.textContent = "Confirm?";
    elements.deleteConfigBtn.classList.add("danger");
    setConfigMessage(`Press Confirm to delete ${name}`);
    window.clearTimeout(state.deleteResetTimer);
    state.deleteResetTimer = window.setTimeout(resetDeleteButton, 3500);
    return;
  }
  resetDeleteButton();
  await runAction(async () => {
    setConfigMessage(`Deleting ${name}...`);
    await invoke("delete_config", { configName: name });
    state.selectedConfig = "";
    state.configSignature = ""; // force dropdown rebuild without the deleted config
    setConfigMessage(`Deleted ${name}`);
    await invoke("refresh_robot");
  }, (message) => setConfigMessage(`Delete failed: ${message}`));
  updateConfigButtons();
});

function resetDeleteButton() {
  state.pendingDeleteConfig = null;
  window.clearTimeout(state.deleteResetTimer);
  elements.deleteConfigBtn.textContent = "Delete";
  elements.deleteConfigBtn.classList.remove("danger");
}

elements.uploadConfigInput.addEventListener("change", async () => {
  const file = elements.uploadConfigInput.files?.[0];
  state.uploadXml = null;
  state.uploadName = "";
  state.uploadPath = "";
  elements.uploadConfigBtn.disabled = true;
  if (!file) {
    setConfigMessage("");
    return;
  }

  await runAction(async () => {
    const xml = await file.text();
    await invoke("validate_config_xml", { xml });
    const name = file.name.replace(/\.xml$/i, "");
    state.uploadXml = xml;
    state.uploadName = name;
    state.uploadPath = file.name;
    elements.uploadConfigBtn.disabled = false;
    setConfigMessage(`Selected ${file.name}`);
  }, (message) => setConfigMessage(`Could not load XML: ${message}`));
});

elements.uploadConfigBtn.addEventListener("click", async () => {
  if (!state.uploadXml) return;
  const configName = state.uploadName;
  if (!configName) return;

  await runAction(async () => {
    elements.uploadConfigBtn.disabled = true;
    setConfigMessage(`Uploading and activating ${configName}...`);
    const result = await invoke("upload_config_xml", { configName, xml: state.uploadXml });
    const warnings = result.validation.warnings.length
      ? ` (${result.validation.warnings.join("; ")})`
      : "";
    state.selectedConfig = configName; // select the uploaded config in the dropdown
    state.configSignature = ""; // force dropdown rebuild so it picks up the new config
    setConfigMessage(`Uploaded and activated ${configName}.${warnings}`);
    await invoke("refresh_robot");
    // Upload button stays disabled to confirm success; re-enabled when a new file is chosen.
  }, (message) => {
    elements.uploadConfigBtn.disabled = false;
    setConfigMessage(`Upload failed: ${message}`);
  });
});

function startPolling() {
  window.clearInterval(window.ftcPoller);
  window.ftcPoller = window.setInterval(async () => {
    if (!state.connected) return;
    try {
      const snapshot = await invoke("get_snapshot");
      renderSnapshot(snapshot);
    } catch (error) {
      elements.statusLine.textContent = String(error);
    }
  }, 250);
}

async function runAction(action, onError) {
  try {
    setStatus("working", "Working");
    await action();
  } catch (error) {
    const message = String(error);
    setStatus("disconnected", message);
    onError?.(message);
  }
}

function renderSnapshot(snapshot) {
  state.snapshot = snapshot;
  const blocked = state.connected && snapshot.peer_conflict;
  const active = snapshot.connected && !blocked;
  elements.shell.classList.toggle("stale", !active);
  let statusKind = "connected";
  let statusText = "Robot Connected";
  if (blocked) {
    statusKind = "disconnected";
    statusText = "Blocked — another device is connected";
  } else if (!snapshot.connected) {
    statusKind = "stale";
    statusText = "Connection lost — robot not responding";
  }
  setStatus(statusKind, statusText);
  elements.robotState.textContent = snapshot.robot_state || "Unknown";
  elements.activeConfig.textContent = snapshot.active_config?.name ?? "None";
  elements.battery.textContent = snapshot.robot_battery ? `${snapshot.robot_battery} V` : "--";
  elements.peerInfo.textContent = snapshot.peer
    ? `${snapshot.peer} ${snapshot.local_port ? `(local ${snapshot.local_port})` : ""}`
    : "No peer";

  renderRobotMessage(snapshot, blocked);
  renderOpModes(snapshot.op_modes ?? []);
  renderConfigs(snapshot.configs ?? [], snapshot.active_config?.name);
  renderTelemetry(snapshot.telemetry ?? []);
}

function renderRobotMessage(snapshot, blocked) {
  if (blocked) {
    showBanner(
      "error",
      "Another device is already connected",
      "A Driver Station or Driver Hub is already connected to this Robot Controller. Disconnect that device — this window will reconnect automatically.",
      false,
    );
  } else if (snapshot.robot_error) {
    showBanner("error", "OpMode error", snapshot.robot_error, true);
  } else if (snapshot.robot_notice) {
    showBanner("notice", "Robot message", snapshot.robot_notice, true);
  } else {
    hideBanner();
  }
}

function showBanner(kind, title, body, dismissible = true) {
  elements.robotBanner.classList.remove("hidden");
  elements.robotBanner.classList.toggle("error", kind === "error");
  elements.robotBanner.classList.toggle("notice", kind === "notice");
  elements.bannerTitle.textContent = title;
  elements.bannerBody.textContent = body;
  elements.bannerDismiss.classList.toggle("hidden", !dismissible);
}

function hideBanner() {
  elements.robotBanner.classList.add("hidden");
  elements.bannerBody.textContent = "";
}

function renderOpModes(opModes) {
  const signature = opModes.map((mode) => `${mode.flavor}:${mode.name}`).join("|");
  if (signature !== state.opModeSignature) {
    state.opModeSignature = signature;
    elements.opModeList.replaceChildren(
      ...opModes.map((mode) => {
        const row = document.createElement("div");
        row.className = "opmode-row";
        row.dataset.opmode = mode.name;
        row.setAttribute("role", "option");
        row.innerHTML = `<span>${mode.flavor || "OPMODE"}</span><strong></strong>`;
        row.querySelector("strong").textContent = mode.name;
        return row;
      }),
    );
  }
  if (!state.selectedOpMode && opModes.length) {
    state.selectedOpMode = opModes[0].name;
  }
  renderOpModeSelection(false);
}

function renderOpModeSelection(scrollSelected) {
  const rows = [...elements.opModeList.querySelectorAll("[data-opmode]")];
  for (const row of rows) {
    const selected = row.dataset.opmode === state.selectedOpMode;
    row.classList.toggle("selected", selected);
    row.setAttribute("aria-selected", selected ? "true" : "false");
    if (selected && scrollSelected) row.scrollIntoView({ block: "nearest" });
  }
}

function renderConfigs(configs, activeName) {
  const signature = `${activeName ?? ""}|${configs.map((config) => `${config.name}:${config.is_dirty}`).join("|")}`;
  if (signature === state.configSignature) return;
  state.configSignature = signature;
  const current = state.selectedConfig || activeName || elements.configSelect.value;
  elements.configSelect.replaceChildren(
    ...configs.map((config) => {
      const option = document.createElement("option");
      option.value = config.name;
      option.textContent = config.name === activeName ? `${config.name} *` : config.name;
      if (config.name === current) option.selected = true;
      return option;
    }),
  );
  // Keep the intended selection even if it isn't in the list yet (e.g. a just-
  // uploaded config before the refresh lands), so it gets selected once it appears.
  const exists = configs.some((config) => config.name === current);
  state.selectedConfig = exists ? current : state.selectedConfig || elements.configSelect.value;
  updateConfigButtons();
}

function updateConfigButtons() {
  const hasSelection = Boolean(state.selectedConfig);
  elements.downloadConfigBtn.disabled = !hasSelection;
  elements.activateConfigBtn.disabled = !hasSelection;
  elements.deleteConfigBtn.disabled = !hasSelection;
  if (state.pendingDeleteConfig && state.pendingDeleteConfig !== state.selectedConfig) {
    resetDeleteButton();
  }
}

function renderTelemetry(lines) {
  if (!lines.length) {
    elements.telemetryList.innerHTML = `<div class="empty">No telemetry</div>`;
    return;
  }
  elements.telemetryList.replaceChildren(
    ...lines.map((line) => {
      const row = document.createElement("div");
      row.className = "telemetry-row";
      const key = document.createElement("span");
      key.textContent = line.key;
      const value = document.createElement("strong");
      value.textContent = line.value;
      row.append(key, value);
      return row;
    }),
  );
}

function renderDisconnected() {
  setStatus("disconnected", "Disconnected");
  elements.shell.classList.add("stale");
  hideBanner();
  elements.robotState.textContent = "Unknown";
  elements.activeConfig.textContent = "None";
  elements.battery.textContent = "--";
  elements.peerInfo.textContent = "No peer";
  elements.opModeList.replaceChildren();
  elements.configSelect.replaceChildren();
  elements.downloadConfigBtn.disabled = true;
  elements.activateConfigBtn.disabled = true;
  elements.deleteConfigBtn.disabled = true;
  resetDeleteButton();
  elements.uploadConfigInput.value = "";
  elements.uploadConfigBtn.disabled = true;
  state.uploadXml = null;
  state.uploadName = "";
  state.uploadPath = "";
  state.selectedConfig = "";
  state.opModeSignature = "";
  state.configSignature = "";
  renderTelemetry([]);
}

function startTimer() {
  state.timerStart = Date.now();
  window.clearInterval(state.timerInterval);
  state.timerInterval = window.setInterval(renderTimer, 250);
  renderTimer();
}

function stopTimer() {
  state.timerStart = null;
  window.clearInterval(state.timerInterval);
  elements.timer.textContent = "0:00";
}

function renderTimer() {
  if (!state.timerStart) return;
  const elapsed = Math.floor((Date.now() - state.timerStart) / 1000);
  const minutes = Math.floor(elapsed / 60);
  const seconds = String(elapsed % 60).padStart(2, "0");
  elements.timer.textContent = `${minutes}:${seconds}`;
}

function setConfigMessage(message) {
  elements.configMessage.textContent = message;
}

function setStatus(kind, message) {
  elements.statusLine.textContent = message;
  elements.statusLine.className = `status-${kind}`;
}

renderDisconnected();

invoke("get_log_path")
  .then((path) => {
    state.logPath = path;
    elements.logPath.textContent = `Log: ${path}`;
  })
  .catch(() => {
    elements.logPath.textContent = "";
  });
