use js::{JsError, Value, Vm, bytecode::compile, parse, runtime::evaluate_program};

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Value(String),
    Error {
        category: String,
        message: String,
        context: Option<String>,
    },
}

fn error_outcome(error: JsError) -> Outcome {
    Outcome::Error {
        category: format!("{:?}", error.category),
        message: error.message,
        context: error.context,
    }
}

fn value_outcome(value: Value) -> Outcome {
    // Display is stable for the currently supported primitive subset and also
    // canonicalizes NaN, which is not equal to itself under f64 equality.
    Outcome::Value(value.to_string())
}

pub fn outcomes(source: &str) -> (Outcome, Outcome) {
    let program =
        parse(source).unwrap_or_else(|error| panic!("invalid test source {source:?}: {error}"));
    let tree = evaluate_program(&program)
        .map(value_outcome)
        .unwrap_or_else(error_outcome);
    let vm = match compile(&program) {
        Ok(chunk) => Vm::new()
            .execute(&chunk)
            .map(value_outcome)
            .unwrap_or_else(error_outcome),
        Err(error) => panic!("test source is outside VM subset {source:?}: {error}"),
    };
    (tree, vm)
}

/// Shrinks a failing program by removing source ranges while preserving a
/// parseable, compilable evaluator/VM mismatch.
pub fn minimize(source: &str) -> String {
    let mut best = source.to_owned();
    let mut granularity = 2;
    while best.chars().count() >= 2 {
        let characters: Vec<char> = best.chars().collect();
        let chunk_size = characters.len().div_ceil(granularity);
        let mut reduced = false;
        for start in (0..characters.len()).step_by(chunk_size) {
            let end = (start + chunk_size).min(characters.len());
            let candidate: String = characters[..start]
                .iter()
                .chain(&characters[end..])
                .collect();
            if candidate.trim().is_empty() {
                continue;
            }
            let Ok(program) = parse(&candidate) else {
                continue;
            };
            let Ok(chunk) = compile(&program) else {
                continue;
            };
            let tree = evaluate_program(&program)
                .map(value_outcome)
                .unwrap_or_else(error_outcome);
            let vm = Vm::new()
                .execute(&chunk)
                .map(value_outcome)
                .unwrap_or_else(error_outcome);
            if tree != vm {
                best = candidate;
                granularity = 2;
                reduced = true;
                break;
            }
        }
        if !reduced {
            if granularity >= best.chars().count() {
                break;
            }
            granularity = (granularity * 2).min(best.chars().count());
        }
    }
    best
}

pub fn assert_equivalent(source: &str) {
    let (tree, vm) = outcomes(source);
    assert_eq!(
        tree,
        vm,
        "differential mismatch\nminimized source: {}\ntree-walk: {tree:#?}\nbytecode VM: {vm:#?}",
        minimize(source)
    );
}
