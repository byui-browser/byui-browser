// Catches a runtime error and evaluates to the caught error object.
try {
    missing();
} catch (error) {
    error;
}
