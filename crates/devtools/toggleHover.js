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

//set devtool button to be toggleable
const toggleButton = document.getElementById("hover-inspect");
let toggleButtonState = false;
toggleButton.addEventListener('click', () => {
    toggleButtonState = !toggleButtonState;
    console.log(`toggleButtonState = ${toggleButtonState}`);
});

//on hover: highlight element
let currentHovered = null;
document.body.addEventListener('mouseover', (e) => {
    // Add highlight to current element
    if (toggleButtonState){
        currentHovered = e.target;
        currentHovered.classList.add('hover-label');
    }
    
});

//clean up/remove hover effects
document.body.addEventListener('mouseout', (e) => {
    //remove highlight
    if (currentHovered) {
        currentHovered.classList.remove('hover-label');
        currentHovered = null;
    }
});