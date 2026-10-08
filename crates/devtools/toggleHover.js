//Tool to hover over elements in on the page and view their HTML tags

//Add required css to the page
const hoverStyles = document.createElement('style');
hoverStyles.textContent = `
.hover-label{
    cursor: crosshair;
    outline: 3px solid #007acc !important;
    background-color: rgba(0, 122, 204, 0.1) !important;
}
`;
document.head.append(hoverStyles);

//add div for element info display
const elementDetails = document.createElement('div');
document.body.append(elementDetails); //note: will need to append to the dev tools window, no the body as it does now

//set devtool button to be toggleable
const toggleButton = document.getElementById("hover-inspect");
let toggleButtonState = false;
toggleButton.addEventListener('click', () => {
    toggleButtonState = !toggleButtonState;
    console.log(`toggleButtonState = ${toggleButtonState}`);
});

//on hover: highlight element and update elementDetails display
let currentHovered = null;
document.body.addEventListener('mouseover', (e) => {
    if (toggleButtonState && e.target != elementDetails){
        currentHovered = e.target;
        currentHovered.classList.add('hover-label');
        elementDetails.textContent = currentHovered.tagName.toLowerCase();
    }
    
});

//remove hover effects [ToDo[and clear elementDetails display]] after mouse stops hovering
document.body.addEventListener('mouseout', (e) => {
    if (currentHovered) {
        currentHovered.classList.remove('hover-label');
        currentHovered = null;
        elementDetails.textContent = '';
    }
});