import { DevConsole } from "./console.js";
import { Elements } from "./elements.js";
import { Network } from "./network.js";

/** Coordinates Devtools visibility, tool selection, and keyboard shortcuts. */
export class Devtools {
    constructor(rootSelector = "#devtools") {
        this.root = document.querySelector(rootSelector);
        if (!this.root) throw new Error(`Devtools: root element "${rootSelector}" not found`);

        this.isOpen = !this.root.hidden;
        this.tools = {
            console: new DevConsole(),
            elements: new Elements(),
            network: new Network(),
        };
        this.activeTool = null;
        this.bindControls();
        this.bindKeyboardShortcut();
        this.selectTool("console");
    }

    bindControls() {
        this.root.querySelectorAll("[data-devtools-tool]").forEach((button) => {
            button.addEventListener("click", () => this.selectTool(button.dataset.devtoolsTool));
        });
        this.root.querySelector("#dev-close-btn")
            .addEventListener("click", () => this.close());
    }

    bindKeyboardShortcut() {
        document.addEventListener("keydown", (event) => {
            if (event.key === "Escape" && this.isOpen) {
                this.close();
                return;
            }
            if (event.ctrlKey && event.shiftKey && event.key.toLowerCase() === "h") {
                event.preventDefault();
                this.toggle();
            }
        });
    }

    selectTool(name) {
        const selectedTool = this.tools[name];
        if (!selectedTool) throw new Error(`Devtools: unknown tool "${name}"`);

        if (this.activeTool && this.activeTool !== selectedTool) {
            this.activeTool.deactivate();
            this.activeTool.close();
        }

        this.root.querySelectorAll("[data-devtools-panel]").forEach((panel) => {
            panel.hidden = panel.dataset.devtoolsPanel !== name;
        });
        this.root.querySelectorAll("[data-devtools-tool]").forEach((button) => {
            button.setAttribute("aria-pressed", String(button.dataset.devtoolsTool === name));
        });

        this.activeTool = selectedTool;
        selectedTool.open();
        selectedTool.activate();
        if (!this.isOpen) selectedTool.close();
    }

    open() {
        if (this.isOpen) return;
        this.isOpen = true;
        this.root.hidden = false;
        this.activeTool?.open();
    }

    close() {
        if (!this.isOpen) return;
        this.isOpen = false;
        this.root.hidden = true;
        this.activeTool?.close();
    }

    toggle() {
        this.isOpen ? this.close() : this.open();
    }
}
