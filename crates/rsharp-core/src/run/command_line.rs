//! A command line as a program's arguments, split the way .NET splits one on every platform: white space
//! separates, a double quote groups, and a backslash escapes only a quote it precedes.

/// Splits a command line into the arguments it passes, as `Environment.GetCommandLineArgs` would.
pub fn split(line: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let mut run = 1;
                while chars.peek() == Some(&'\\') {
                    chars.next();
                    run += 1;
                }
                if chars.peek() == Some(&'"') {
                    // An even run leaves the quote its meaning; an odd one makes half the run and the quote.
                    current.extend(std::iter::repeat_n('\\', run / 2));
                    if run % 2 == 1 {
                        current.push('"');
                    } else {
                        quoted = !quoted;
                    }
                    chars.next();
                } else {
                    current.extend(std::iter::repeat_n('\\', run));
                }
                started = true;
            }
            '"' => {
                // Two quotes inside a quoted run are one literal quote, as .NET reads them.
                if quoted && chars.peek() == Some(&'"') {
                    current.push('"');
                    chars.next();
                } else {
                    quoted = !quoted;
                }
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    arguments.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        arguments.push(current);
    }
    arguments
}

#[cfg(test)]
mod tests {
    use super::split;

    #[test]
    fn a_command_line_splits_as_dotnet_splits_one() {
        // A profile's arguments reach the program as the person wrote them, however they are quoted.
        assert!(split("").is_empty());
        assert!(split("   ").is_empty(), "spaces alone are no arguments");
        assert_eq!(split("--urls http://localhost:5000"), ["--urls", "http://localhost:5000"]);
        assert_eq!(split(r#"a "b c" d"#), ["a", "b c", "d"]);
        assert_eq!(split(r#""a b""#), ["a b"], "quotes alone still make the argument they hold");
        assert_eq!(split("a \"\" b"), ["a", "", "b"], "an empty quoted argument is kept, as .NET keeps it");
        assert_eq!(split(r#"c:\a\b"#), [r"c:\a\b"], "a backslash not before a quote is itself");
        assert_eq!(split(r#""c:\a b\d""#), [r"c:\a b\d"]);
        assert_eq!(split(r#""a\"b""#), [r#"a"b"#], "a backslash before a quote makes the quote literal");
        assert_eq!(split(r#""a\\""#), [r"a\"], "an even run leaves the quote grouping and halves the run");
        assert_eq!(split(r#" ""a"" "#), ["a"], "two quotes inside a quoted run are one");
    }
}
