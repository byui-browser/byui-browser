const devtools_div = document.querySelector("#devtools");
let devtoolsCreated = false;

devtools_div.hidden = true;

function getLoadedResources() {
  if (!performance || typeof performance.getEntriesByType !== "function") {
    return [];
  }

  return performance.getEntriesByType("resource").map((entry) => ({
    name: entry.name,
    status: entry.responseStatus ?? "Unknown",
    type: getResourceType(entry),
    initiator: entry.initiatorType || "Other",
    size: entry.transferSize ?? 0,
    startTime: entry.startTime,
    time: entry.duration
  }));
}

function getResourceType(entry) {
  const name = entry.name.split(/[?#]/)[0].toLowerCase();
  const extension = name.includes(".") ? name.split(".").pop() : "";

  if (entry.initiatorType === "fetch" || entry.initiatorType === "xmlhttprequest") {
    return "fetch/xhr";
  }

  if (extension === "css") return "stylesheet";
  if (["js", "mjs"].includes(extension)) return "script";
  if (["png", "jpg", "jpeg", "gif", "svg", "webp", "ico"].includes(extension)) {
    return "image";
  }
  if (["woff", "woff2", "ttf", "otf"].includes(extension)) return "font";
  if (["mp3", "mp4", "webm", "ogg", "wav"].includes(extension)) return "media";
  if (entry.initiatorType === "navigation") return "document";

  return entry.initiatorType || "other";
}

function formatResourceSize(bytes) {
  if (!bytes) return "—";
  if (bytes < 1024) return bytes + " B";
  return (bytes / 1024).toFixed(1) + " kB";
}

function createTimeline(requests) {
  const timeline = document.createElement("section");
  timeline.id = "network-waterfall";
  timeline.className = "network-waterfall";
  timeline.setAttribute("aria-label", "Network request timing overview");

  const firstStart = requests.length
    ? Math.min(...requests.map((request) => request.startTime))
    : 0;
  const lastEnd = requests.length
    ? Math.max(...requests.map((request) => request.startTime + request.time))
    : 0;
  const duration = Math.max(lastEnd - firstStart, 20);

  timeline.dataset.startTimeMs = String(firstStart);
  timeline.dataset.durationMs = String(duration);

  const axis = document.createElement("div");
  axis.className = "waterfall-axis";
  axis.setAttribute("aria-label", "Elapsed time in milliseconds");

  const tickInterval = duration <= 200 ? 20 : duration <= 500 ? 50 : 100;
  const tickCount = Math.ceil(duration / tickInterval);

  for (let index = 0; index <= tickCount; index += 1) {
    const tick = document.createElement("span");
    const time = index * tickInterval;
    tick.className = "waterfall-tick";
    tick.dataset.timeMs = String(time);
    tick.textContent = time + " ms";
    axis.appendChild(tick);
  }

  timeline.appendChild(axis);

  const tracks = document.createElement("div");
  tracks.className = "waterfall-tracks";

  requests.forEach((request, index) => {
    const track = document.createElement("div");
    track.className = "waterfall-track";
    track.dataset.requestIndex = String(index);

    const bar = document.createElement("span");
    bar.className = "waterfall-bar";
    bar.dataset.startMs = String(request.startTime - firstStart);
    bar.dataset.durationMs = String(request.time);
    bar.dataset.resourceType = request.type;
    bar.setAttribute(
      "aria-label",
      request.name + ": starts at " +
        (request.startTime - firstStart).toFixed(1) +
        " ms and lasts " + request.time.toFixed(1) + " ms"
    );

    track.appendChild(bar);
    tracks.appendChild(track);
  });

  timeline.appendChild(tracks);
  return timeline;
}

function displayNetworkRequests(requests) {
  const table = document.querySelector("#network-options");
  const tbody = document.querySelector("#network-rows");
  const oldTimeline = document.querySelector("#network-waterfall");

  if (oldTimeline) oldTimeline.remove();
  table.before(createTimeline(requests));
  tbody.replaceChildren();

  for (const request of requests) {
    const row = document.createElement("tr");

    for (const value of [
      request.name,
      request.status,
      request.type,
      request.initiator,
      formatResourceSize(request.size),
      Math.round(request.time) + " ms"
    ]) {
      const cell = document.createElement("td");
      cell.textContent = value;
      row.appendChild(cell);
    }

    tbody.appendChild(row);
  }
}

function ReloadNetworkLog() {
  displayNetworkRequests(getLoadedResources());
}

document.addEventListener("keydown", (event) => {
  if (
    event.ctrlKey &&
    event.shiftKey &&
    event.key.toLowerCase() === "h"
  ) {
    event.preventDefault();

    if (!devtoolsCreated) {
      devtools_div.innerHTML = `
        <h1>Welcome to devtools!</h1>
        <button id="console-btn">Console</button>
        <button id="element-btn">Elements</button>
        <button id="hover-btn">Hover</button>
        <button id="network-button">Network</button>
        <button id="reload-network-button" type="button">Reload</button>

        <table id="network-options" hidden>
          <thead>
            <tr>
              <th>Name</th>
              <th>Status</th>
              <th>Type</th>
              <th>Initiator</th>
              <th>Size</th>
              <th>Time</th>
            </tr>
          </thead>
          <tbody id="network-rows"></tbody>
        </table>
      `;

      document
        .querySelector("#network-button")
        .addEventListener("click", () => {
          const table = document.querySelector("#network-options");
          table.hidden = !table.hidden;

          if (!table.hidden) ReloadNetworkLog();
        });

      document
        .querySelector("#reload-network-button")
        .addEventListener("click", ReloadNetworkLog);

      devtoolsCreated = true;
    }

    devtools_div.hidden = !devtools_div.hidden;
  }
});
