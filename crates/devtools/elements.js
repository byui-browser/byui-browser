import { DevTools } from "./devtools-base.js";

/** Elements panel placeholder; DOM inspection will be added in a later change. */
export class Elements extends DevTools {
    constructor() {
        super("#elements-panel");
    }
}
