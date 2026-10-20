import { DevTools } from "./devtools-base.js";

/** Elements panel with a toggleable hover inspector for page content. */
export class Elements extends DevTools {
    constructor() {
        super("#elements-panel");
        this.toggleButton = this.require("#hover-inspect");
        this.details = this.require("#hover-element-details");
        this.isInspecting = false;
        this.currentElement = null;
        this.hoverStyles = document.createElement("style");
        this.hoverStyles.textContent = `
            .devtools-hover-highlight {
                outline: 3px solid #007acc !important;
                background-color: rgba(0, 122, 204, 0.1) !important;
            }
        `;

        this.toggleButton.addEventListener("click", () => this.toggleHover());
        this.onMouseOver = (event) => this.handleMouseOver(event);
        this.onMouseOut = (event) => this.handleMouseOut(event);
    }

    toggleHover() {
        this.isInspecting ? this.stopInspecting() : this.startInspecting();
    }

    startInspecting() {
        if (this.isInspecting) return;
        this.isInspecting = true;
        document.head.appendChild(this.hoverStyles);
        document.addEventListener("mouseover", this.onMouseOver);
        document.addEventListener("mouseout", this.onMouseOut);
        this.toggleButton.textContent = "Stop hover inspection";
        this.details.textContent = "Move the pointer over page content to inspect an element.";
    }

    stopInspecting() {
        if (!this.isInspecting) return;
        this.isInspecting = false;
        document.removeEventListener("mouseover", this.onMouseOver);
        document.removeEventListener("mouseout", this.onMouseOut);
        this.clearCurrentElement();
        this.hoverStyles.remove();
        this.toggleButton.textContent = "Inspect elements on hover";
        this.details.textContent = "Hover inspection is off.";
    }

    handleMouseOver(event) {
        const element = event.target;
        if (!(element instanceof Element) || this.root.closest("#devtools")?.contains(element)) return;

        this.clearCurrentElement();
        this.currentElement = element;
        element.classList.add("devtools-hover-highlight");
        this.details.textContent = this.describeElement(element);
    }

    handleMouseOut(event) {
        if (this.currentElement && event.target === this.currentElement) {
            this.clearCurrentElement();
        }
    }

    describeElement(element) {
        const id = element.id ? `#${element.id}` : "";
        const classes = [...element.classList]
            .filter((name) => name !== "devtools-hover-highlight")
            .map((name) => `.${name}`)
            .join("");
        return `<${element.tagName.toLowerCase()}${id}${classes}>`;
    }

    clearCurrentElement() {
        this.currentElement?.classList.remove("devtools-hover-highlight");
        this.currentElement = null;
    }

    onDeactivate() {
        this.stopInspecting();
    }

    onClose() {
        this.stopInspecting();
    }
}
