//! Interactive command-line shell for the JavaScript engine.

use std::io::{self, Write};
use std::sync::Arc;

use js::{Realm, Value};

fn main() -> io::Result<()> {
    let mut realm = Realm::new();
    realm
        .register_global_function(
            "print",
            Arc::new(|arguments| {
                println!(
                    "{}",
                    arguments
                        .iter()
                        .map(Value::to_string)
                        .collect::<Vec<_>>()
                        .join(" ")
                );
                Ok(Value::Undefined)
            }),
        )
        .expect("print is a valid global function name");

    println!("BYUI JavaScript shell");
    println!("Type JavaScript, or `exit`/`quit` to leave.");

    let stdin = io::stdin();
    let mut input = String::new();
    loop {
        print!("js> ");
        io::stdout().flush()?;

        input.clear();
        if stdin.read_line(&mut input)? == 0 {
            break;
        }

        let source = input.trim();
        if source.is_empty() {
            continue;
        }
        if matches!(source, "exit" | "quit") {
            break;
        }

        match realm.evaluate_script(source) {
            Ok(value) if value != Value::Undefined => println!("{value}"),
            Ok(_) => {}
            Err(error) => eprintln!("{:?}: {error}", error.category),
        }
    }

    Ok(())
}
