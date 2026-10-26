//! Building projects through the .NET command line.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

mod output;
pub use output::{run, run_while};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub severity: Severity,
    pub message: String,
    pub code: String,
    pub path: Option<PathBuf>,
    pub project: Option<PathBuf>,
    pub line: usize,
    pub column: usize,
}

pub struct Parser {
    folder: PathBuf,
    problems: Vec<Problem>,
    continuing: bool,
}

impl Parser {
    pub fn new(folder: PathBuf) -> Self {
        Self { folder, problems: Vec::new(), continuing: false }
    }

    /// Accepts a complete output line, including an unterminated final line.
    pub fn push(&mut self, line: &str) {
        static DIAGNOSTIC: LazyLock<regex::Regex> = LazyLock::new(|| {
            regex::Regex::new(concat!(
                r"^\s*(?:(?<file>.*?)(?:\((?<line>\d+)(?:,(?<column>\d+))?(?:[-,]\d+)*\))?\s*:\s*)?",
                r"(?:[A-Za-z][\w.]*\s+)?(?<severity>error|warning)\s+(?<code>[A-Za-z]+\d+)\s*:",
                r"(?<gap>[^\S\r\n]*)(?<message>.*?)(?:\s+\[(?<project>[^\]]+)\])?\s*$"
            ))
            .unwrap()
        });
        let Some(captures) = DIAGNOSTIC.captures(line.trim_end_matches(['\r', '\n'])) else {
            self.finalize();
            return;
        };
        let project = captures.name("project").map(|p| resolve(p.as_str(), &self.folder));
        let base = project.as_ref().and_then(|p| p.parent()).unwrap_or(&self.folder);
        let path = captures
            .name("file")
            .map(|m| m.as_str().trim())
            .filter(|p| !p.is_empty() && !p.starts_with("MSBUILD") && !matches!(*p, "CSC" | "FSC" | "VBC"))
            .map(|p| resolve(p, base));
        let position =
            |key| captures.name(key).and_then(|p| p.as_str().parse::<usize>().ok()).unwrap_or(1).saturating_sub(1);
        let problem = Problem {
            severity: if &captures["severity"] == "error" { Severity::Error } else { Severity::Warning },
            message: captures["message"].trim().to_string(),
            code: captures["code"].to_string(),
            path,
            project,
            line: position("line"),
            column: position("column"),
        };
        let gap = &captures["gap"];
        if self.continuing && (gap.len() > 1 || problem.message.is_empty()) {
            let before = self.problems.last_mut().unwrap();
            let same = problem.severity == before.severity
                && problem.code == before.code
                && problem.path == before.path
                && problem.project == before.project
                && problem.line == before.line
                && problem.column == before.column;
            if same {
                before.message.push('\n');
                before.message.push_str(gap.get(1..).unwrap_or_default());
                before.message.push_str(captures["message"].trim_end());
                return;
            }
        }
        self.finalize();
        self.problems.push(problem);
        self.continuing = true;
    }

    pub fn problems(&self) -> &[Problem] {
        &self.problems
    }

    pub fn finish(mut self) -> Vec<Problem> {
        self.finalize();
        self.problems
    }

    fn finalize(&mut self) {
        if self.continuing {
            let last = self.problems.len() - 1;
            if self.problems[..last].contains(&self.problems[last]) {
                self.problems.pop();
            }
        }
        self.continuing = false;
    }
}

fn resolve(path: &str, folder: &Path) -> PathBuf {
    let path = path.replace('\\', "/");
    let windows_drive = path.as_bytes().get(1) == Some(&b':');
    let path = Path::new(&path);
    crate::paths::canonical(&if path.is_absolute() || windows_drive { path.to_path_buf() } else { folder.join(path) })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Build,
    Rebuild,
    Clean,
}

pub fn arguments(target: &Path, kind: Kind, windows_targeting: bool) -> Vec<OsString> {
    let mut arguments =
        vec![OsString::from(if kind == Kind::Clean { "clean" } else { "build" }), target.as_os_str().to_owned()];
    if kind == Kind::Rebuild {
        arguments.push("--no-incremental".into());
    }
    if windows_targeting {
        arguments.push("-p:EnableWindowsTargeting=true".into());
    }
    arguments.extend(["-nologo", "-tl:off", "-clp:NoSummary;ForceNoAlign"].map(OsString::from));
    arguments
}

/// A program's output as a person reads it, without the colour and cursor codes it wrote for a terminal. The codes
/// may be split across the chunks a program's writes arrive in, so the reader keeps its state between chunks.
#[derive(Default)]
pub struct Ansi {
    state: u8,
}

impl Ansi {
    /// The text with the codes taken out, and the state a code left open kept for the next chunk.
    pub fn feed(&mut self, text: &str) -> String {
        let mut plain = String::with_capacity(text.len());
        for c in text.chars() {
            match self.state {
                0 => {
                    if c == '\u{1b}' {
                        self.state = 1;
                    } else {
                        plain.push(c);
                    }
                }
                1 => match c {
                    '[' => self.state = 2,
                    ']' => self.state = 3,
                    _ => self.state = 0,
                },
                2 => {
                    if ('@'..='~').contains(&c) {
                        self.state = 0;
                    }
                }
                3 => match c {
                    '\u{7}' => self.state = 0,
                    '\u{1b}' => self.state = 4,
                    _ => {}
                },
                4 => match c {
                    '\\' => self.state = 0,
                    _ => self.state = 3,
                },
                _ => self.state = 0,
            }
        }
        plain
    }
}

/// The codes taken out of a whole text: an output that arrived in one piece.
pub fn strip_ansi(text: &str) -> String {
    Ansi::default().feed(text)
}

#[cfg(test)]
mod tests {
    #[test]
    fn ansi_codes_are_taken_out_even_when_split_across_chunks() {
        let mut ansi = super::Ansi::default();
        assert_eq!(ansi.feed("\u{1b}"), "", "an escape alone is kept for what follows");
        assert_eq!(ansi.feed("[31mred\u{1b}[0m plain"), "red plain");
        assert_eq!(super::strip_ansi("\u{1b}]0;title\u{7}name"), "name");
    }

    use super::*;

    #[test]
    fn a_rebuild_keeps_the_target_literal_and_disables_incremental_and_animated_output() {
        assert_eq!(
            arguments(Path::new("My App/App.csproj"), Kind::Rebuild, true),
            [
                "build",
                "My App/App.csproj",
                "--no-incremental",
                "-p:EnableWindowsTargeting=true",
                "-nologo",
                "-tl:off",
                "-clp:NoSummary;ForceNoAlign",
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn ordinary_build_and_clean_use_only_their_requested_operation() {
        for (kind, operation) in [(Kind::Build, "build"), (Kind::Clean, "clean")] {
            assert_eq!(
                arguments(Path::new("App.slnx"), kind, false),
                [operation, "App.slnx", "-nologo", "-tl:off", "-clp:NoSummary;ForceNoAlign"].map(OsString::from)
            );
        }
    }

    #[test]
    fn diagnostics_resolve_from_the_emitting_project_and_keep_parentheses_and_ranges() {
        let mut parser = Parser::new(PathBuf::from("/work"));
        parser.push("src/Program (copy).cs(12,5,12,9): error CS0103: No such name [app/App.csproj]");
        let problems = parser.finish();
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].path, Some(PathBuf::from("/work/app/src/Program (copy).cs")));
        assert_eq!(problems[0].project, Some(PathBuf::from("/work/app/App.csproj")));
        assert_eq!((problems[0].line, problems[0].column), (11, 4));
    }

    #[test]
    fn windows_drives_subcategories_and_locationless_messages_are_not_source_files() {
        let mut parser = Parser::new(PathBuf::from("/work"));
        parser
            .push(r"C:\Program Files (x86)\App\View.axaml(8): Avalonia error AVLN2000: Bad markup [C:\App\App.csproj]");
        parser.push("MSBUILD : error MSB1009: Project file does not exist.");
        parser.push("warning NU1902: Package is vulnerable [app/App.csproj]");
        let problems = parser.finish();
        assert_eq!(problems.len(), 3);
        assert_eq!(problems[0].path, Some(PathBuf::from("C:/Program Files (x86)/App/View.axaml")));
        assert_eq!((problems[0].line, problems[0].column), (7, 0));
        assert_eq!(problems[1].path, None);
        assert_eq!(problems[2].path, None);
        assert_eq!(problems[2].severity, Severity::Warning);
    }

    #[test]
    fn multiline_diagnostics_are_joined_and_repeated_summaries_are_deduplicated() {
        let mut parser = Parser::new(PathBuf::from("/work"));
        for _ in 0..2 {
            parser.push("View.axaml(4,2): error AVLN2000: Bad markup [App.csproj]");
            parser.push("View.axaml(4,2): error AVLN2000:   at Compiler.Parse() [App.csproj]");
            parser.push("Build FAILED.");
        }
        let problems = parser.finish();
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].message, "Bad markup\n  at Compiler.Parse()");
    }
}
