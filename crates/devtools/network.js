import { DevTools } from "./devtools-base.js";

const DEMO_REQUEST_LOG = [
    { name: "https://example.test/index.html", event: "200 OK (sample)" },
    { name: "https://example.test/app.js", event: "200 OK (sample)" },
    { name: "https://example.test/missing.css", event: "404 Not Found (sample)" },
];

/** Network panel for inspecting loaded script and stylesheet resources. */
export class Network extends DevTools {
    constructor() {
        super("#network-panel");
        this.rows = this.require("#network-rows");
        this.requestLog = DEMO_REQUEST_LOG;
        this.reloadButton = this.require("#reload-network-button");
        this.onReload = () => {};
        this.reloadButton.addEventListener("click", () => {
            this.reload(DEMO_REQUEST_LOG);
            this.onReload();
        });
    }

    getLoadedResources() {
        return performance
            .getEntriesByType("resource")
            .filter((entry) => /\.(css|js|mjs)([?#]|$)/i.test(entry.name))
            .map((entry) => ({
                name: entry.name,
                status: entry.responseStatus ?? "Unknown",
                type: /\.css([?#]|$)/i.test(entry.name) ? "stylesheet" : "script",
                time: Math.round(entry.duration),
            }));
    }

    createRow(values) {
        const row = document.createElement("tr");
        for (const value of values) {
            const cell = document.createElement("td");
            cell.textContent = String(value);
            row.appendChild(cell);
        }
        return row;
    }

    renderResources(resources) {
        this.rows.replaceChildren();
        for (const resource of resources) {
            this.rows.appendChild(
                this.createRow([resource.name, resource.status, resource.type, resource.time]),
            );
        }
    }

    renderRequestLog() {
        for (const entry of this.requestLog) {
            this.rows.appendChild(this.createRow([entry.name, entry.event, "", ""]));
        }
    }

    reload(requestLog = this.requestLog) {
        this.requestLog = requestLog;
        this.renderResources(this.getLoadedResources());
        this.renderRequestLog();
    }

    onOpen() {
        this.reload();
    }
}
