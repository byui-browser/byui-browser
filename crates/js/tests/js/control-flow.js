let total = 0;
let value = 1;
while (value <= 5) {
    total = total + value;
    value = value + 1;
}
if (total > 10) {
    total = total + 5;
}
{
    let adjustment = 0;
    if (total == 20) {
        adjustment = 1;
    }
    total = total + adjustment - 1;
}
total;
