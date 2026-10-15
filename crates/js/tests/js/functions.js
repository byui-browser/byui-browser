function double(value) {
    return value + value;
}

function add(left, right) {
    return left + right;
}

function twice(value) {
    let once = double(value);
    return double(once);
}

let first = twice(3);
let second = add(first, 3);
second;
