// TODO(js): `for` statements are not implemented yet.
let total = 0;
for (let index = 0; index < 3; index = index + 1) {
    total = total + index;
}
if (total == 3) {
    total = total + 1;
}
total;
