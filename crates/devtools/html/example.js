const devtools_div = document.querySelector("#devtools");
let devtoolsCreated = false;

devtools_div.hidden = true;

function getLoadedCssAndJs() {
  const requests = performance
    .getEntriesByType("resource")
    .filter((entry) => /\.(css|js|mjs)([?#]|$)/i.test(entry.name))
    .map((entry) => ({
      name: entry.name,
      status: entry.responseStatus ?? "Unknown",
      type: /\.css([?#]|$)/i.test(entry.name) ? "stylesheet" : "script",
      time: Math.round(entry.duration)
    }));

  console.log("reload", requests);
  return requests;
}

function displayNetworkRequests(requests) {
  const tbody = document.querySelector("#network-rows");
  tbody.replaceChildren();

  for (const request of requests) {
    const row = document.createElement("tr");

    for (const value of [
      request.name,
      request.status,
      request.type,
      request.time
    ]) {
      const cell = document.createElement("td");
      cell.textContent = value;
      row.appendChild(cell);
    }

    tbody.appendChild(row);
  }
}

function ReloadNetworkLog() {
  displayNetworkRequests(getLoadedCssAndJs());
  // this needs another paramater to read from a new variable "request_log" that is an array of objects
  // [{nmame: data, event: data}] and displays it
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

          if (!table.hidden) {
            ReloadNetworkLog();
          }
        });

      document
        .querySelector("#reload-network-button")
        .addEventListener("click", ReloadNetworkLog);

      devtoolsCreated = true;
    }

    devtools_div.hidden = !devtools_div.hidden;
  }
});