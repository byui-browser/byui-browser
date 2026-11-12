function callback() {
    alert('Clicked')
}

const btn = document.getElementById('btn')
btn.addEventListener('click', callback);

let num = 1.232890723;
console.log(Math.round(num))