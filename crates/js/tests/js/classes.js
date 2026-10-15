// TODO(js): Classes and `new` are not implemented yet.
class Counter {
    constructor() {
        this.value = 0;
    }

    increment() {
        this.value = this.value + 1;
    }
}

let counter = new Counter();
counter.increment();
counter.value;
