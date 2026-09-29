// How to bind keys to shortcuts
// if i press Ctrl + D (you can change that) them i trigger a console.log saying It worked
// ctrl + shift + h
const devtools_div = document.querySelector("#devtools");

document.addEventListener("keydown", (event) => {
  if (event.ctrlKey && event.shiftKey && event.key.toLowerCase() === "h") {
    devtools_div.innerHTML = `
      <h1>Welcome to devtools!</h1>
      <button id="console-btn">Console</button>
      <button id="element-btn">Elements</button>
      <button id="hover-btn">Hover</button>
      <button id="network-button">Network</button>
      <ul id="network-options" hidden>
        <li><button>Doc</button></li>
        <li><button>CSS</button></li>
        <li><button>JS</button></li>
        <li><button>Img</button></li>
      </ul>
    `;

    document.querySelector("#network-button").addEventListener("click", () => {
      document.querySelector("#network-options").hidden = false;
    
    });
  }
});