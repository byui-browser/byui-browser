import { DevTools } from "./devtools-base.js";

const MIRRORED_CONSOLE_METHODS = [
    "log", "info", "debug", "error", "warn", "dir", "table", "trace",
];
const originalConsole = Object.fromEntries(
    [
        ...MIRRORED_CONSOLE_METHODS,
        "assert", "clear", "group", "groupCollapsed", "groupEnd",
    ].map((method) => [
        method,
        typeof console[method] === "function" ? console[method].bind(console) : null,
    ]),
);

export class DevConsole extends DevTools {
    constructor() {
        super("#console-panel");

        this.input = this.require("#dev-console-input");
        this.output = this.require("#dev-console-output");

        this.history = [];
        this.historyIndex = -1;
        this.resetContext(false);

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
                this.print(`> ${code}`, "command");
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
        if (value && typeof value === "object" && typeof value.nodeType === "number") {
            return value.outerHTML || value.textContent || value.nodeName;
        }

        if (typeof value === "object" && value !== null) {
            const seen = new WeakSet();
            try {
                return JSON.stringify(value, (_key, nestedValue) => {
                    if (nestedValue && typeof nestedValue === "object") {
                        if (seen.has(nestedValue)) return "[Circular]";
                        seen.add(nestedValue);
                    }
                    return nestedValue;
                }, 2);
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
            const source = this.persistSimpleDeclarations(code);
            const result = this.evaluateInPageContext(source);
            if (result && typeof result.then === "function") {
                result.then(
                    (resolved) => {
                        if (resolved !== undefined) this.print(resolved, "result");
                    },
                    (error) => this.print(error, "error"),
                );
            } else if (result !== undefined) {
                this.print(result, "result");
            }
        } catch (error) {
            this.print(error, "error");
        }
    }

    createContext() {
        const values = { __devtoolsContext: null };
        const context = new Proxy(values, {
            has: (target, property) => {
                if (property === Symbol.unscopables) return false;
                return Reflect.has(target, property) ||
                    (typeof property === "string" && property in window);
            },
            get: (target, property) => {
                if (property === Symbol.unscopables) return undefined;
                if (Reflect.has(target, property)) return Reflect.get(target, property);
                return window[property];
            },
            set: (target, property, value) => {
                Reflect.set(target, property, value);
                return true;
            },
        });
        values.__devtoolsContext = context;
        return context;
    }

    persistSimpleDeclarations(code) {
        // Store simple top-level let/const/var declarations in the console context.
        // This keeps values available for later commands without changing page globals.
        return code.replace(
            /(^|[;\n]\s*)(?:let|const|var)\s+([A-Za-z_$][\w$]*)\s*(?:=\s*([^;\n]+))?(?=[;\n]|$)/g,
            (_declaration, prefix, name, initializer) =>
                `${prefix}__devtoolsContext[${JSON.stringify(name)}] = ${initializer ?? "undefined"}`,
        );
    }

    evaluateInPageContext(source) {
        const evaluate = new Function(
            "environment",
            "source",
            "with (environment) { return eval(source); }",
        );
        return evaluate(this.context, source);
    }

    resetContext(showMessage = true) {
        this.context = this.createContext();
        if (showMessage && this.output) this.print("Console variables reset", "command");
    }

    clear() {
        this.output.innerHTML = "";
    }

    focusInput() {
        this.input.focus();
    }

    // Overrides of the parent's hooks

    onActivate() {
        for (const method of MIRRORED_CONSOLE_METHODS) {
            console[method] = (...args) => this.write(method, args);
        }
        console.assert = (condition, ...args) => {
            originalConsole.assert?.(condition, ...args);
            if (!condition) {
                this.print(["Assertion failed:", ...args].map((value) => this.format(value)).join(" "), "error");
            }
        };
        console.clear = () => {
            originalConsole.clear?.();
            this.clear();
        };
        for (const method of ["group", "groupCollapsed"]) {
            console[method] = (...args) => {
                originalConsole[method]?.(...args);
                if (args.length) this.write("log", args);
            };
        }
        console.groupEnd = () => originalConsole.groupEnd?.();

        this.onError = (event) => {
            const detail = event.error || event.message || "Unknown application error";
            this.print(`Uncaught ${this.format(detail)}`, "error");
        };
        this.onRejection = (event) =>
            this.print(`Uncaught (in promise) ${this.format(event.reason)}`, "error");
        window.addEventListener("error", this.onError);
        window.addEventListener("unhandledrejection", this.onRejection);
    }

    onDeactivate() {
        for (const [method, original] of Object.entries(originalConsole)) {
            if (original) console[method] = original;
        }

        window.removeEventListener("error", this.onError);
        window.removeEventListener("unhandledrejection", this.onRejection);
    }

    write(level, args) {
        originalConsole[level]?.(...args);
        this.print(args.map((value) => this.format(value)).join(" "), level);
    }

     onOpen() {
        this.focusInput();
        this.output.scrollTop = this.output.scrollHeight;
    }

    onClose() {
        this.input.blur();
    }
}
