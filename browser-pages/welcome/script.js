// Welcome page behavior for BYUI Browser.

// Plain <a href> links do not navigate in BYUI Browser yet, so each
// quick-link tile navigates to its data-href from a click listener.
const linkTile = (id) => {
  const tile = document.getElementById(id);
  tile.addEventListener('click', () => {
    location.href = tile.getAttribute('data-href');
  });
};
linkTile('tile-byui');
linkTile('tile-canvas');
linkTile('tile-library');
linkTile('tile-email');
linkTile('tile-code');
linkTile('tile-ladder');

// Oak color toggle. style.css shows the oak named by #page's tree-* class,
// and the browser lays the page out again after a click changes it.
const pageRoot = document.getElementById('page');
const showOak = (color) => {
  pageRoot.setAttribute('class', 'page tree-' + color);
};
document.getElementById('pick-green').addEventListener('click', () => {
  showOak('green');
});
document.getElementById('pick-blue').addEventListener('click', () => {
  showOak('blue');
});
