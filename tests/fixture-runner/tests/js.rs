//! JS team fixtures: `input.js` -> completion value, or the error category.
use fixture_runner::{read, run_group};

#[test]
fn js_fixtures() {
    run_group("js", |case| match js::eval(&read(case, "input.js")) {
        Ok(value) => format!("Ok: {value}\n"),
        Err(error) => format!("Err: {:?}\n", error.category),
    });
}
