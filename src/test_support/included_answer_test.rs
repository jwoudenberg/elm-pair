// A helper for defining tests where the test input and expected output are
// included in the same file. These are like golden tests, in the sense that the
// expected output will be appended to files automatically if they're missing,
// and asserted against if present.

use std::io::Write;
use std::path::Path;

pub fn assert_eq_answer_in(output: &str, path: &Path) {
    let prefix = "-- ";
    let separator = prefix.to_owned() + "=== expected output below ===\n";
    let contents = assert_ok(std::fs::read_to_string(path));
    match contents.split_once(&separator) {
        None => {
            let mut file =
                assert_ok(std::fs::OpenOptions::new().append(true).open(path));
            assert_ok(file.write_all(separator.as_bytes()));
            for line in output.lines() {
                assert_ok(file.write_all(prefix.as_bytes()));
                assert_ok(file.write_all(line.as_bytes()));
                assert_ok(file.write_all("\n".as_bytes()));
            }
        }
        Some((_, expected_output_prefixed)) => {
            let expected_output = expected_output_prefixed
                .lines()
                .map(|x| {
                    x.strip_prefix(&prefix)
                        .or_else(|| x.strip_prefix(&prefix.trim_end()))
                        .unwrap_or(x)
                        .trim_end()
                })
                .collect::<Vec<&str>>()
                .join("\n");
            let actual_output = output
                .lines()
                .map(|x| x.trim_end())
                .collect::<Vec<&str>>()
                .join("\n");
            if actual_output != expected_output {
                eprintln!("Actual output does not match expected");
                eprintln!();
                eprintln!("--- actual ----");
                eprintln!("{}", actual_output);
                eprintln!();
                eprintln!("--- expected ----");
                eprintln!("{}", expected_output);
                eprintln!();
                panic!()
            }
        }
    }
}

pub fn snake_to_camel(str: &str) -> String {
    str.split('_')
        .map(|word| {
            let (first, rest) = word.split_at(1);
            first.to_uppercase() + rest
        })
        .collect::<Vec<String>>()
        .join("")
}

fn assert_ok<A, E: std::fmt::Debug>(result: Result<A, E>) -> A {
    match result {
        Err(err) => panic!("{:?}", err),
        Ok(x) => x,
    }
}
