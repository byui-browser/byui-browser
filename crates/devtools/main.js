import { Devtools } from "./devtools.js";

const devtools = new Devtools();
window.devtools = devtools;
window.devConsole = devtools.tools.console;
