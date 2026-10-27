// Classes and `new` run only through the bytecode VM (see tests/bytecode_classes.rs).
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
