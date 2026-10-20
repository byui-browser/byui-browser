/** Shared lifecycle and panel visibility behavior for Devtools panels. */
export class DevTools {
    constructor(rootSelector) {
        this.root = document.querySelector(rootSelector);
        if (!this.root) {
            throw new Error(`DevTools: root element "${rootSelector}" not found`);
        }

        this.active = false;
        this.isOpen = !this.root.hidden;
    }

    onActivate() {}
    onDeactivate() {}
    onOpen() {}
    onClose() {}

    activate() {
        if (this.active) return;
        this.active = true;
        this.onActivate();
    }

    deactivate() {
        if (!this.active) return;
        this.active = false;
        this.onDeactivate();
    }

    open() {
        if (this.isOpen) return;
        this.isOpen = true;
        this.root.hidden = false;
        this.onOpen();
    }

    close() {
        if (!this.isOpen) return;
        this.isOpen = false;
        this.root.hidden = true;
        this.onClose();
    }

    toggle() {
        this.isOpen ? this.close() : this.open();
    }

    require(selector) {
        const element = document.querySelector(selector);
        if (!element) throw new Error(`DevTools: element "${selector}" not found`);
        return element;
    }
}
