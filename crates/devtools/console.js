import { DevTools } from "./devtools-base.js";

const originalConsole = {
    log: console.log.bind(console),
    error: console.error.bind(console),
    warn: console.warn.bind(console),
};

export class DevConsole extends DevTools {
    constructor() {
        super("#console-panel");

        this.console_div = this.require("#dev-console-div");
        this.input = this.require("#dev-console-input");
        this.output = this.require("#dev-console-output");

        this.history = [];
        this.historyIndex = -1;

        this.input.addEventListener("keydown", (e) => this.onKeyDown(e));

        this.activate();
    }

    initTryCatchBlock(callback) {
        try {
            callback();
        } catch (error) {
            // this will make any error be printed in our console.
            this.print(error, "error");
            throw error;
        }
    }

    onKeyDown(event) {
        if (event.isComposing) return;

        if (event.key === "Enter") {
            const code = this.input.value;
            if (code.trim() !== "") {
                if (this.history[this.history.length - 1] !== code) {
                    this.history.push(code);
                }
                this.historyIndex = this.history.length;
                this.print("> " + code, "command");
                this.execute(code);
            }
            this.input.value = "";
        } else if (event.key === "ArrowUp") {
            if (this.history.length === 0) return;
            this.historyIndex = Math.max(0, this.historyIndex - 1);
            this.input.value = this.history[this.historyIndex];
            event.preventDefault();
        } else if (event.key === "ArrowDown") {
            if (this.history.length === 0) return;
            this.historyIndex++;
            if (this.historyIndex >= this.history.length) {
                this.historyIndex = this.history.length;
                this.input.value = "";
            } else {
                this.input.value = this.history[this.historyIndex];
            }
            event.preventDefault();
        }
    }

    format(value) {
        if (value instanceof Error) return value.stack || String(value);
        if (typeof value === "function") return value.toString();
        if (typeof value === "symbol") return value.toString();
        if (value === undefined) return "undefined";
        if (value instanceof Map) return `Map(${value.size}) ` + this.format([...value]);
        if (value instanceof Set) return `Set(${value.size}) ` + this.format([...value]);

        if (typeof value === "object" && value !== null) {
            try {
                return JSON.stringify(value, null, 2);
            } catch {
                return String(value);
            }
        }
        return String(value);
    }

    print(value, type = "result") {
        const element = document.createElement("div");
        element.className = type;
        element.textContent = typeof value === "string" ? value : this.format(value);
        this.output.appendChild(element);
        this.output.scrollTop = this.output.scrollHeight;
    }

    execute(code) {
        try {
            const result = (0, eval)(code);
            if (result !== undefined) this.print(result, "result");
        } catch (error) {
            this.print(error, "error");
        }
    }

    clear() {
        this.output.innerHTML = "";
    }

    focusInput() {
        this.input.focus();
    }

    // Overrides of the parent's hooks

    onActivate() {
        console.log = (...args) => this.write("log", args);
        console.error = (...args) => this.write("error", args);
        console.warn = (...args) => this.write("warn", args);

        this.onError = (e) => this.print(`Uncaught ${e.message}`, "error");
        this.onRejection = (e) =>
            this.print(`Uncaught (in promise) ${this.format(e.reason)}`, "error");
        window.addEventListener("error", this.onError);
        window.addEventListener("unhandledrejection", this.onRejection);
    }

    onDeactivate() {
        console.log = originalConsole.log;
        console.error = originalConsole.error;
        console.warn = originalConsole.warn;

        window.removeEventListener("error", this.onError);
        window.removeEventListener("unhandledrejection", this.onRejection);
    }

    // Replaces the three near-identical consoleLog/Error/Warn methods
    write(level, args) {
        originalConsole[level](...args);
        this.print(args.map((a) => this.format(a)).join(" "), level);
    }

     onOpen() {
        this.focusInput();
        this.output.scrollTop = this.output.scrollHeight;
    }

    onClose() {
        this.input.blur();
    }
}
