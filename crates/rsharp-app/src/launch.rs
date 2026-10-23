//! The command line. A malformed flag or value is fatal, never a silent default.

use std::path::PathBuf;

#[derive(Debug, Default, PartialEq)]
pub struct LaunchOptions {
    pub path: Option<PathBuf>,
    pub file: Option<PathBuf>,
    /// Line and column, counted from 1 as the person writes them.
    pub caret: Option<(usize, usize)>,
    /// What is pressed and typed through the window's real input once it has opened, in order.
    pub steps: Vec<Step>,
    /// Measure the start, the solution's load, memory and typing, print them as JSON and quit (`bench.rs`).
    pub bench: bool,
    /// Serve MCP on standard input and output over this folder instead of opening a window (`rsharp-mcp`).
    pub mcp: Option<PathBuf>,
}

#[derive(Debug, PartialEq, Clone)]
pub enum Step {
    /// Keys as gpui writes them, a chord's keys apart: `cmd-shift-a`, `ctrl-k ctrl-s`.
    Key(String),
    /// Text, a key at a time.
    Type(String),
}

impl LaunchOptions {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut options = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--file" => options.file = Some(args.next().ok_or("--file needs a file")?.into()),
                "--caret" => {
                    let value = args.next().ok_or("--caret needs LINE:COLUMN")?;
                    let parsed = value.split_once(':').and_then(|(l, c)| Some((l.parse().ok()?, c.parse().ok()?)));
                    match parsed {
                        Some((line, column)) if line >= 1 && column >= 1 => options.caret = Some((line, column)),
                        _ => return Err(format!("--caret takes LINE:COLUMN, each from 1, not {value:?}")),
                    }
                }
                "--key" => options.steps.push(Step::Key(args.next().ok_or("--key needs keys, such as cmd-shift-a")?)),
                "--bench" => options.bench = true,
                "--mcp" => options.mcp = Some(PathBuf::new()),
                "--type" => options.steps.push(Step::Type(args.next().ok_or("--type needs text")?)),
                flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
                path if options.path.is_none() => options.path = Some(path.into()),
                extra => return Err(format!("one folder or solution at a time, not also {extra:?}")),
            }
        }
        // --mcp's folder is the one named, else the one it runs in.
        if options.mcp.is_some() {
            let folder = match options.path.take() {
                Some(folder) => folder,
                None => std::env::current_dir().map_err(|e| format!("--mcp has no folder: {e}"))?,
            };
            if !folder.is_dir() {
                return Err(format!("--mcp takes a folder, not {}", folder.display()));
            }
            if options.file.is_some() || !options.steps.is_empty() || options.bench {
                return Err("--mcp opens no window, so it takes no --file, --key, --type or --bench".into());
            }
            options.mcp = Some(folder);
        }
        if options.caret.is_some() && options.file.is_none() {
            return Err("--caret needs --file".into());
        }
        Ok(options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<LaunchOptions, String> {
        LaunchOptions::parse(args.iter().map(|a| a.to_string()))
    }

    #[test]
    fn a_folder_file_and_caret_are_read_and_anything_malformed_refused() {
        assert_eq!(
            parse(&[".", "--file", "Program.cs", "--caret", "12:5"]),
            Ok(LaunchOptions {
                path: Some(".".into()),
                file: Some("Program.cs".into()),
                caret: Some((12, 5)),
                steps: vec![],
                bench: false,
                mcp: None,
            })
        );
        assert!(parse(&["--caret", "12:5"]).is_err(), "a caret with no file");
        assert!(parse(&["--file", "a", "--caret", "0:1"]).is_err(), "a line counted from 0");
        assert!(parse(&["--file", "a", "--caret", "x"]).is_err());
        assert!(parse(&["--theme", "dark"]).is_err(), "an option RSharp does not know");
        assert!(parse(&["a", "b"]).is_err(), "two folders");
        assert_eq!(
            parse(&["--key", "cmd-,", "--type", "font"]).unwrap().steps,
            [Step::Key("cmd-,".into()), Step::Type("font".into())],
            "steps were not kept in the order given"
        );
    }

    #[test]
    fn mcp_serves_the_folder_named_and_refuses_what_only_a_window_takes() {
        let folder = std::env::temp_dir();
        let named = parse(&["--mcp", folder.to_str().unwrap()]).unwrap();
        assert_eq!((named.mcp, named.path), (Some(folder.clone()), None));
        assert!(parse(&["--mcp", folder.to_str().unwrap(), "--file", "A.cs"]).is_err());
        assert!(parse(&["--mcp", "/no/such/folder"]).is_err());
    }
}
