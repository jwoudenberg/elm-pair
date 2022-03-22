// A helper for defining tests where the test input and expected output are
// included in the same file. These are like golden tests, in the sense that the
// expected output will be appended to files automatically if they're missing,
// and asserted against if present.

use crate::lib::intersperse::Intersperse;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use std::io::Write;
use std::path::Path;

pub fn for_file<F>(path: &Path, f: F)
where
    F: Fn(&str) -> String,
{
    let file = assert_ok(
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path),
    );
    let mut buffered = BufReader::new(file);
    let separator = "=== expected output below ===";
    let mut prefix = "".to_string();
    let mut lines = buffered.by_ref().lines();
    let mut found_separator = false;
    let input: String = lines
        .by_ref()
        .map(|line| line.unwrap())
        .take_while(|line| {
            if let Some(prefix_) = line.strip_suffix(separator) {
                found_separator = true;
                prefix = prefix_.to_string();
            }
            !found_separator
        })
        .my_intersperse("\n".to_string())
        .collect();

    let output = f(&input);

    let actual_output: String = output
        .lines()
        .map(|line| line.trim_end())
        .my_intersperse("\n")
        .collect();

    if found_separator {
        let expected_output: String = lines
            .map(|opt_line| {
                let line = opt_line.unwrap();
                line.strip_prefix(&prefix)
                    .or_else(|| line.strip_prefix(&prefix.trim_end()))
                    .unwrap_or(&line)
                    .trim_end()
                    .to_string()
            })
            .my_intersperse("\n".to_string())
            .collect();
        if actual_output.trim_end() != expected_output.trim_end() {
            eprintln!("Actual output does not match expected");
            eprintln!();
            eprintln!("[actual]");
            eprintln!("{}", actual_output);
            eprintln!();
            eprintln!("[expected]");
            eprintln!("{}", expected_output);
            eprintln!();
            panic!()
        }
    } else {
        let mut file_for_writing = buffered.into_inner();
        assert_ok(file_for_writing.write_all(prefix.as_bytes()));
        assert_ok(file_for_writing.write_all(separator.as_bytes()));
        assert_ok(file_for_writing.write_all("\n".as_bytes()));

        for line in output.lines() {
            assert_ok(file_for_writing.write_all(prefix.as_bytes()));
            assert_ok(file_for_writing.write_all(line.as_bytes()));
            assert_ok(file_for_writing.write_all("\n".as_bytes()));
        }
    }
}

fn assert_ok<A, E: std::fmt::Debug>(result: Result<A, E>) -> A {
    match result {
        Err(err) => panic!("{:?}", err),
        Ok(x) => x,
    }
}
