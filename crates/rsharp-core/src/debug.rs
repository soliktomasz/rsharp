//! A folder's breakpoints, kept with it as Rider keeps them: the lines of each file and what each asks of the debugger
//! (its condition, its hit count, its log message, whether it is on), the exception breakpoints, and Mute Breakpoints.
//! Data alone; a session in `rsharp-dap` sends them.

use crate::state::StateDocument;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Rider's "Suspend if" for an exception breakpoint, as the protocol names each case.
pub const SCENARIOS: [&str; 5] =
    ["thrownInUserCode", "thrownInExternalCode", "handledInUserCode", "handledInExternalCode", "unhandled"];

/// One line breakpoint: where it is, and what it asks of the debugger at a hit.
// shortcut: a breakpoint follows its line by number, so an edit above it moves it off its statement; carry it through
// the buffer's changes, as Rider does, once that is missed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LineBreakpoint {
    /// From 0.
    pub line: usize,
    /// From 0, for one call on a line several share (a lambda's body, say); usually none, as a line breakpoint is.
    pub column: Option<usize>,
    pub enabled: bool,
    /// A C# expression the debugger evaluates where it stopped: it stops only when this is true.
    pub condition: String,
    /// Rider's "Hit count": a number, `> 5`, `% 3` and the like, as the debugger's own condition reads it.
    pub hit_count: String,
    /// A message logged where the program would stop, in `{expression}` braces; empty for one that stops.
    pub log_message: String,
}

impl Default for LineBreakpoint {
    fn default() -> Self {
        Self {
            line: 0,
            column: None,
            enabled: true,
            condition: String::new(),
            hit_count: String::new(),
            log_message: String::new(),
        }
    }
}

/// Rider's CLR exception breakpoint: the exception types it is about (none for "Any exception"), when it stops, and
/// what a hit asks besides.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExceptionBreakpoint {
    /// The exception's full names, as CLR spells them; none for every exception, Rider's "Any exception".
    pub types: Vec<String>,
    pub enabled: bool,
    /// Which of `SCENARIOS` hold; one of each half must, as Rider's own "Suspend if" says.
    pub scenarios: Vec<String>,
    pub condition: String,
    pub hit_count: String,
    pub log_message: String,
}

impl Default for ExceptionBreakpoint {
    fn default() -> Self {
        Self {
            types: Vec::new(),
            enabled: true,
            scenarios: vec!["unhandled".into()],
            condition: String::new(),
            hit_count: String::new(),
            log_message: String::new(),
        }
    }
}

impl ExceptionBreakpoint {
    /// Rider's "Any exception": every exception the program's own code leaves unhandled.
    pub fn any() -> Self {
        Self::default()
    }

    pub fn is_any(&self) -> bool {
        self.types.is_empty()
    }

    /// The breakpoint as the protocol names it: its type path, and which of Rider's cases stop it.
    pub fn path(&self) -> Vec<String> {
        let mut path = vec!["CLR".to_string()];
        path.extend(self.types.iter().filter(|kind| !kind.trim().is_empty()).cloned());
        path
    }
}

/// Rider's function breakpoint: a method's name as `Namespace.Type.Method`, with `*` at its end for a suffix that
/// matches any, and what a hit asks of the debugger.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FunctionBreakpoint {
    pub name: String,
    pub enabled: bool,
    pub condition: String,
    pub hit_count: String,
    pub log_message: String,
}

impl Default for FunctionBreakpoint {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            condition: String::new(),
            hit_count: String::new(),
            log_message: String::new(),
        }
    }
}

/// The folder's breakpoints: each file's lines, the function breakpoints, the exception breakpoints ("Any exception"
/// never removed) and whether every breakpoint is muted.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Breakpoints {
    files: BTreeMap<PathBuf, Vec<LineBreakpoint>>,
    /// The method names the debugger stops at, wherever they are.
    functions: Vec<FunctionBreakpoint>,
    /// Rider's "Any exception", which every folder has and none removes.
    any_exception: ExceptionBreakpoint,
    /// The other CLR exception breakpoints, by type.
    exceptions: Vec<ExceptionBreakpoint>,
    muted: bool,
}

impl Breakpoints {
    /// The enabled lines of a file, from 0, sorted.
    pub fn lines(&self, path: &Path) -> Vec<usize> {
        if self.muted {
            return Vec::new();
        }
        let mut lines: Vec<usize> = self
            .files
            .iter()
            .filter(|(file, _)| crate::paths::same(file, path))
            .flat_map(|(_, breakpoints)| breakpoints.iter())
            .filter(|breakpoint| breakpoint.enabled)
            .map(|breakpoint| breakpoint.line)
            .collect();
        lines.sort_unstable();
        lines.dedup();
        lines
    }

    /// Every breakpoint of a file, on or not, in line order.
    pub fn of(&self, path: &Path) -> Vec<LineBreakpoint> {
        let mut breakpoints = self
            .files
            .iter()
            .filter(|(file, _)| crate::paths::same(file, path))
            .flat_map(|(_, breakpoints)| breakpoints.clone())
            .collect::<Vec<_>>();
        breakpoints.sort_by_key(|breakpoint| breakpoint.line);
        breakpoints
    }

    /// Sets or clears a line, as F9 does: the one there goes, or a new one stands in its place.
    pub fn toggle(&mut self, path: &Path, line: usize) {
        if self.has(path, line) {
            self.remove(path, line);
            return;
        }
        let key = self.files.keys().find(|file| crate::paths::same(file, path)).cloned();
        let breakpoints = match key {
            Some(key) => self.files.get_mut(&key).expect("the key came from the map"),
            None => self.files.entry(path.to_path_buf()).or_default(),
        };
        breakpoints.push(LineBreakpoint { line, ..LineBreakpoint::default() });
    }

    pub fn remove(&mut self, path: &Path, line: usize) {
        let key = self.files.keys().find(|file| crate::paths::same(file, path)).cloned();
        if let Some(key) = key
            && let Some(breakpoints) = self.files.get_mut(&key)
        {
            breakpoints.retain(|breakpoint| breakpoint.line != line);
            if breakpoints.is_empty() {
                self.files.remove(&key);
            }
        }
    }

    /// Whether a line holds a breakpoint, on or not.
    pub fn has(&self, path: &Path, line: usize) -> bool {
        self.files
            .iter()
            .filter(|(file, _)| crate::paths::same(file, path))
            .any(|(_, breakpoints)| breakpoints.iter().any(|breakpoint| breakpoint.line == line))
    }

    pub fn breakpoint(&self, path: &Path, line: usize) -> Option<&LineBreakpoint> {
        self.files
            .iter()
            .filter(|(file, _)| crate::paths::same(file, path))
            .flat_map(|(_, breakpoints)| breakpoints)
            .find(|breakpoint| breakpoint.line == line)
    }

    /// Edits a line's breakpoint as the dialog's fields do.
    pub fn edit(&mut self, path: &Path, line: usize, change: impl FnOnce(&mut LineBreakpoint)) {
        let key = self.files.keys().find(|file| crate::paths::same(file, path)).cloned();
        if let Some(key) = key
            && let Some(breakpoints) = self.files.get_mut(&key)
            && let Some(breakpoint) = breakpoints.iter_mut().find(|breakpoint| breakpoint.line == line)
        {
            change(breakpoint);
        }
    }

    /// Every enabled line breakpoint there is, by file, as a session is told of them. Muted, there are none.
    pub fn enabled(&self) -> Vec<(PathBuf, Vec<LineBreakpoint>)> {
        if self.muted {
            return Vec::new();
        }
        self.files
            .iter()
            .filter_map(|(file, breakpoints)| {
                let enabled: Vec<LineBreakpoint> =
                    breakpoints.iter().filter(|breakpoint| breakpoint.enabled).cloned().collect();
                (!enabled.is_empty()).then(|| (file.clone(), enabled))
            })
            .collect()
    }

    /// Every file with a breakpoint, on or off, as a session must be told of even an emptied one.
    pub fn files(&self) -> Vec<PathBuf> {
        self.files.keys().cloned().collect()
    }

    /// How many breakpoints there are, on or off: what the window says.
    pub fn count(&self) -> usize {
        self.files.values().map(Vec::len).sum::<usize>() + self.functions.len() + self.exceptions.len() + 1
    }

    pub fn any_exception(&self) -> &ExceptionBreakpoint {
        &self.any_exception
    }

    pub fn any_exception_mut(&mut self) -> &mut ExceptionBreakpoint {
        &mut self.any_exception
    }

    pub fn functions(&self) -> &[FunctionBreakpoint] {
        &self.functions
    }

    pub fn function_mut(&mut self, index: usize) -> Option<&mut FunctionBreakpoint> {
        self.functions.get_mut(index)
    }

    /// Adds a function breakpoint, as the dialog's Add does: a name to be typed, stopping where the method does.
    pub fn add_function(&mut self) {
        self.functions.push(FunctionBreakpoint::default());
    }

    pub fn remove_function(&mut self, index: usize) {
        if index < self.functions.len() {
            self.functions.remove(index);
        }
    }

    /// Every function breakpoint that is on and names a method, as a session is told of them. Muted, there are none.
    pub fn enabled_functions(&self) -> Vec<FunctionBreakpoint> {
        if self.muted {
            return Vec::new();
        }
        self.functions
            .iter()
            .filter(|breakpoint| breakpoint.enabled && !breakpoint.name.trim().is_empty())
            .cloned()
            .collect()
    }

    pub fn exceptions(&self) -> &[ExceptionBreakpoint] {
        &self.exceptions
    }

    pub fn exceptions_mut(&mut self) -> &mut Vec<ExceptionBreakpoint> {
        &mut self.exceptions
    }

    /// Adds a CLR exception breakpoint, as the dialog's Add does: a type to be edited, stopping as Rider's new ones do.
    pub fn add_exception(&mut self) {
        self.exceptions.push(ExceptionBreakpoint {
            types: vec!["System.Exception".into()],
            scenarios: vec!["thrownInUserCode".into(), "unhandled".into()],
            ..ExceptionBreakpoint::default()
        });
    }

    pub fn remove_exception(&mut self, index: usize) {
        if index < self.exceptions.len() {
            self.exceptions.remove(index);
        }
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    /// Sets whether every breakpoint of a file is on (Mute Breakpoints asks for all of them).
    pub fn set_enabled(&mut self, path: &Path, line: usize, enabled: bool) {
        if let Some(breakpoints) =
            self.files.iter_mut().find(|(file, _)| crate::paths::same(file, path)).map(|(_, breakpoints)| breakpoints)
            && let Some(breakpoint) = breakpoints.iter_mut().find(|breakpoint| breakpoint.line == line)
        {
            breakpoint.enabled = enabled;
        }
    }

    /// The document with each file relative to the folder, so it survives the folder being moved.
    pub fn for_store(&self, folder: &Path) -> Self {
        let folder = crate::paths::canonical(folder);
        Self {
            files: self
                .files
                .iter()
                .map(|(file, breakpoints)| {
                    let relative = file.strip_prefix(&folder).map(Path::to_path_buf).unwrap_or_else(|_| file.clone());
                    (relative, breakpoints.clone())
                })
                .collect(),
            ..self.clone()
        }
    }

    /// The document with each file absolute again, as a session needs it.
    pub fn resolved(&self, folder: &Path) -> Self {
        let folder = crate::paths::canonical(folder);
        Self {
            files: self
                .files
                .iter()
                .map(|(file, breakpoints)| {
                    let full = if file.is_absolute() { file.clone() } else { folder.join(file) };
                    (crate::paths::canonical(&full), breakpoints.clone())
                })
                .collect(),
            ..self.clone()
        }
    }
}

impl StateDocument for Breakpoints {
    const FILE: &'static str = "breakpoints.json";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("rsharp-breakpoints-{name}-{}", std::process::id()));
        std::fs::create_dir_all(folder.join("App")).unwrap();
        crate::paths::canonical(&folder)
    }

    #[test]
    fn a_line_is_toggled_and_kept_relative_to_the_folder() {
        let folder = folder("lines");
        let file = folder.join("App").join("Program.cs");
        std::fs::write(&file, "class Program {}\n").unwrap();
        let mut breakpoints = Breakpoints::default();
        breakpoints.toggle(&file, 4);
        breakpoints.toggle(&file, 2);
        assert_eq!(breakpoints.lines(&file), vec![2, 4], "the lines a session is told of are in order");
        assert!(breakpoints.has(&file, 4) && !breakpoints.has(&file, 3));
        assert_eq!(breakpoints.count(), 3, "two lines and Any exception");

        let stored = breakpoints.for_store(&folder);
        assert_eq!(
            stored.files.keys().collect::<Vec<_>>(),
            vec![&PathBuf::from("App/Program.cs")],
            "a breakpoint inside the folder is kept relative"
        );
        assert_eq!(stored.clone().resolved(&folder), breakpoints, "and read back absolute");

        breakpoints.toggle(&file, 4);
        assert_eq!(breakpoints.lines(&file), vec![2], "the second F9 clears it");
        breakpoints.set_enabled(&file, 2, false);
        assert!(breakpoints.lines(&file).is_empty(), "a muted breakpoint is not sent");
        assert!(breakpoints.enabled().is_empty(), "nothing is sent at all");
        assert_eq!(breakpoints.count(), 2, "but it is still there");
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_breakpoint_keeps_its_condition_hit_count_and_log_message() {
        let folder = folder("details");
        let file = folder.join("App").join("Program.cs");
        std::fs::write(&file, "class Program {}\n").unwrap();
        let mut breakpoints = Breakpoints::default();
        breakpoints.toggle(&file, 6);
        breakpoints.edit(&file, 6, |breakpoint| {
            breakpoint.condition = "a > 3".into();
            breakpoint.hit_count = "> 2".into();
            breakpoint.log_message = "a is {a}".into();
        });
        let kept = breakpoints.of(&file);
        assert_eq!(kept[0].condition, "a > 3");
        assert_eq!(kept[0].hit_count, "> 2");
        assert_eq!(kept[0].log_message, "a is {a}");
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_process_is_named_by_the_dll_dotnet_runs() {
        let hosted = DotnetProcess {
            id: 42,
            name: "dotnet".into(),
            command_line: "/Users/me/.dotnet/dotnet /work/Shop.Api.dll --port 5000".into(),
            user: "me".into(),
            is_mine: true,
        };
        assert_eq!(hosted.program(), "Shop.Api", "the dll the host runs names it");
        let apphost =
            DotnetProcess { name: "Shop.Api".into(), command_line: "/work/Shop.Api".into(), ..hosted.clone() };
        assert_eq!(apphost.program(), "Shop.Api", "an apphost names it without its extension");
        let exe = DotnetProcess { name: "Shop.exe".into(), ..hosted.clone() };
        assert_eq!(exe.program(), "Shop", "and Windows' .exe is dropped");
        assert!(
            hosted.matches("shop") && hosted.matches("42") && hosted.matches("--port"),
            "search finds name, id and command line"
        );
        assert!(!hosted.matches("passenger"));

        assert_eq!(transport_pid("clr-debug-pipe-123-456789-in"), Some(123));
        assert_eq!(transport_pid("clr-debug-pipe-123-456789-out"), None);
        assert_eq!(transport_pid("dotnet-diagnostic-123"), None);

        let kept = DebugSessions { recent_attached: Some(hosted) };
        let json = serde_json::to_string(&kept).unwrap();
        assert_eq!(serde_json::from_str::<DebugSessions>(&json).unwrap(), kept, "the recent attach is kept as JSON");
        assert_eq!(<DebugSessions as StateDocument>::FILE, "debug.json");
    }

    #[test]
    fn any_exception_is_always_there_and_mute_silences_every_breakpoint() {
        let folder = folder("exceptions");
        let file = folder.join("App").join("Program.cs");
        std::fs::write(&file, "class Program {}\n").unwrap();
        let mut breakpoints = Breakpoints::default();
        assert!(breakpoints.any_exception().is_any(), "a folder begins with Rider's Any exception");
        assert_eq!(breakpoints.any_exception().path(), vec!["CLR"], "it is every exception");
        breakpoints.toggle(&file, 4);
        breakpoints.set_muted(true);
        assert!(breakpoints.lines(&file).is_empty(), "Mute Breakpoints sends none");
        assert!(breakpoints.enabled().is_empty());
        breakpoints.set_muted(false);
        assert_eq!(breakpoints.lines(&file), vec![4]);

        breakpoints.add_exception();
        assert_eq!(breakpoints.exceptions().len(), 1);
        assert_eq!(breakpoints.exceptions()[0].path(), vec!["CLR", "System.Exception"]);
        breakpoints.exceptions_mut()[0].types = vec!["System.InvalidOperationException".into()];
        assert_eq!(
            breakpoints.exceptions()[0].path(),
            vec!["CLR", "System.InvalidOperationException"],
            "a typed exception keeps its full name"
        );
        breakpoints.remove_exception(0);
        assert!(breakpoints.exceptions().is_empty());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_function_breakpoint_keeps_its_name_and_asks() {
        let mut breakpoints = Breakpoints::default();
        breakpoints.add_function();
        assert_eq!(breakpoints.count(), 2, "a function breakpoint and Any exception");
        assert!(breakpoints.enabled_functions().is_empty(), "an unnamed one is not sent");
        let function = breakpoints.function_mut(0).expect("the one just added");
        function.name = "Program.Double".into();
        function.condition = "x > 0".into();
        function.hit_count = "> 2".into();
        function.log_message = "x is {x}".into();
        let sent = breakpoints.enabled_functions();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].name, "Program.Double");
        assert_eq!(sent[0].condition, "x > 0");

        breakpoints.function_mut(0).unwrap().enabled = false;
        assert!(breakpoints.enabled_functions().is_empty(), "a disabled one is not sent");
        breakpoints.function_mut(0).unwrap().enabled = true;
        breakpoints.set_muted(true);
        assert!(breakpoints.enabled_functions().is_empty(), "Mute Breakpoints silences it too");
        breakpoints.set_muted(false);

        let json = serde_json::to_string(&breakpoints.for_store(Path::new("/work"))).unwrap();
        let kept: Breakpoints = serde_json::from_str(&json).unwrap();
        assert_eq!(kept.functions()[0].name, "Program.Double", "it is kept with the folder");

        breakpoints.remove_function(0);
        assert!(breakpoints.functions().is_empty());
        assert_eq!(breakpoints.count(), 1);
    }
}

/// A .NET program a debugger can attach to: its process, its executable's name, its command line and the user it runs
/// as, [`DotnetProcess::is_mine`] when that is the person running RSharp.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DotnetProcess {
    pub id: u32,
    pub name: String,
    pub command_line: String,
    pub user: String,
    pub is_mine: bool,
}

impl DotnetProcess {
    /// What the program is to a person: the .dll the dotnet host runs ("Shop.Api" for "dotnet Shop.Api.dll"), else the
    /// executable's name without its extension.
    pub fn program(&self) -> String {
        let executable = self.name.strip_suffix(".exe").unwrap_or(&self.name);
        if executable.eq_ignore_ascii_case("dotnet")
            && let Some(dll) = self
                .command_line
                .split_whitespace()
                .skip(1)
                .map(|word| word.trim_matches('"'))
                .find(|word| word.to_ascii_lowercase().ends_with(".dll"))
            && let Some(stem) = Path::new(dll).file_stem()
        {
            return stem.to_string_lossy().into_owned();
        }
        executable.to_string()
    }

    /// Whether a search typed into Attach to Process finds it: in its name, its process id, its user or its command line.
    pub fn matches(&self, search: &str) -> bool {
        let search = search.trim();
        let lower = search.to_ascii_lowercase();
        search.is_empty()
            || self.name.to_ascii_lowercase().contains(&lower)
            || self.id.to_string().contains(search)
            || self.user.to_ascii_lowercase().contains(&lower)
            || self.command_line.to_ascii_lowercase().contains(&lower)
    }
}

/// The .NET programs on this machine a debugger can attach to, as Rider's Attach to Process lists them: CoreCLR's alone,
/// found by the debugger transport its runtime opened (a `clr-debug-pipe-<pid>-<started>-in` in the temporary folder).
/// RSharp's own process is left out. Each one's user and command line are read with `ps`, off the UI thread.
// shortcut: Windows lists no programs yet, its named pipes not being read; list them from `\\.\pipe\` when a Windows
// attach is wanted.
#[cfg(not(windows))]
pub fn processes() -> Vec<DotnetProcess> {
    let mut found: Vec<DotnetProcess> = candidate_ids().into_iter().filter_map(read_process).collect();
    found.sort_by(|a, b| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()).then(a.id.cmp(&b.id)));
    found.dedup_by_key(|process| process.id);
    found
}

#[cfg(windows)]
pub fn processes() -> Vec<DotnetProcess> {
    Vec::new()
}

/// One program by its process id, when it is one a debugger can attach to.
pub fn find_process(id: u32) -> Option<DotnetProcess> {
    processes().into_iter().find(|process| process.id == id)
}

/// The process each of the runtime's transport pipes names, RSharp's own left out.
#[cfg(not(windows))]
fn candidate_ids() -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else { return Vec::new() };
    let mut ids: Vec<u32> = entries
        .flatten()
        .filter_map(|entry| transport_pid(&entry.file_name().to_string_lossy()))
        .filter(|id| *id != std::process::id())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

#[cfg(windows)]
fn candidate_ids() -> Vec<u32> {
    Vec::new()
}

/// The process id a runtime transport's pipe names, or none for another file.
fn transport_pid(name: &str) -> Option<u32> {
    let rest = name.strip_prefix("clr-debug-pipe-")?.strip_suffix("-in")?;
    // <pid>-<started at>
    rest.split('-').next()?.parse().ok()
}

/// A process as the system describes it, or none when it has gone or is not one the pipes named: macOS asks
/// `proc_pidpath` for the executable and `sysctl` for its arguments, as Passenger reads it.
#[cfg(target_os = "macos")]
fn read_process(id: u32) -> Option<DotnetProcess> {
    use std::ffi::c_void;
    let pid = id as libc::c_int;
    let mut path = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let length = unsafe { libc::proc_pidpath(pid, path.as_mut_ptr() as *mut c_void, path.len() as u32) };
    if length <= 0 {
        return None;
    }
    path.truncate(length as usize);
    let executable = String::from_utf8_lossy(&path).into_owned();
    let arguments = mac_arguments(pid)?;
    let command_line = if arguments.is_empty() { executable.clone() } else { arguments.join(" ") };
    let name = Path::new(&executable).file_name()?.to_string_lossy().into_owned();
    let uid = mac_uid(pid)?;
    let user = user_name(uid).unwrap_or_else(|| uid.to_string());
    Some(DotnetProcess { id, name, command_line, user, is_mine: uid == unsafe { libc::getuid() } })
}

/// A process as the system describes it: Linux's `/proc` holds its arguments and its uid.
#[cfg(target_os = "linux")]
fn read_process(id: u32) -> Option<DotnetProcess> {
    let bytes = std::fs::read(format!("/proc/{id}/cmdline")).ok()?;
    let arguments: Vec<String> = bytes
        .split(|byte| *byte == 0)
        .filter(|argument| !argument.is_empty())
        .map(|argument| String::from_utf8_lossy(argument).into_owned())
        .collect();
    let first = arguments.first()?;
    let name = Path::new(first).file_name()?.to_string_lossy().into_owned();
    let status = std::fs::read_to_string(format!("/proc/{id}/status")).ok()?;
    let uid: u32 =
        status.lines().find_map(|line| line.strip_prefix("Uid:")?.split_whitespace().next()?.parse().ok())?;
    let user = user_name(uid).unwrap_or_else(|| uid.to_string());
    Some(DotnetProcess { id, name, command_line: arguments.join(" "), user, is_mine: uid == unsafe { libc::getuid() } })
}

#[cfg(windows)]
fn read_process(_: u32) -> Option<DotnetProcess> {
    None
}

/// A process's arguments, as `sysctl`'s `KERN_PROCARGS2` holds them: the count, the executable's path, then the
/// arguments, each NUL-terminated, with padding between the path and the first.
#[cfg(target_os = "macos")]
fn mac_arguments(pid: libc::c_int) -> Option<Vec<String>> {
    use std::ffi::c_void;
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut size = 0usize;
    if unsafe { libc::sysctl(mib.as_mut_ptr(), 3, std::ptr::null_mut(), &mut size, std::ptr::null_mut(), 0) } != 0
        || size < 4
    {
        return None;
    }
    let mut buffer = vec![0u8; size];
    if unsafe {
        libc::sysctl(mib.as_mut_ptr(), 3, buffer.as_mut_ptr() as *mut c_void, &mut size, std::ptr::null_mut(), 0)
    } != 0
    {
        return None;
    }
    buffer.truncate(size);
    let count = i32::from_ne_bytes(buffer.get(..4)?.try_into().ok()?) as usize;
    // The executable's path, the NUL after it, then zeros up to the first argument.
    let mut at = 4;
    while at < buffer.len() && buffer[at] != 0 {
        at += 1;
    }
    while at < buffer.len() && buffer[at] == 0 {
        at += 1;
    }
    let mut arguments = Vec::with_capacity(count);
    for _ in 0..count {
        if at >= buffer.len() {
            break;
        }
        let end = at + buffer[at..].iter().position(|byte| *byte == 0)?;
        arguments.push(String::from_utf8_lossy(&buffer[at..end]).into_owned());
        at = end + 1;
    }
    Some(arguments)
}

/// A process's uid, from `proc_pidinfo`'s BSD side.
#[cfg(target_os = "macos")]
fn mac_uid(pid: libc::c_int) -> Option<u32> {
    use std::ffi::c_void;
    // proc_bsdinfo: pbi_flags, pbi_status, pbi_xstatus, pbi_pid, pbi_ppid, pbi_uid …
    let mut info = [0u8; 136];
    let read = unsafe {
        libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, info.as_mut_ptr() as *mut c_void, info.len() as i32)
    };
    if read < 24 {
        return None;
    }
    Some(u32::from_ne_bytes(info[20..24].try_into().ok()?))
}

/// A user's name, by their uid.
#[cfg(unix)]
fn user_name(uid: u32) -> Option<String> {
    let passwd = unsafe { libc::getpwuid(uid) };
    if passwd.is_null() {
        return None;
    }
    let name = unsafe { std::ffi::CStr::from_ptr((*passwd).pw_name) };
    Some(name.to_string_lossy().into_owned())
}

/// The process last attached to in a folder, which Reattach to Process asks for: enough to tell it from a program given
/// the same number since.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DebugSessions {
    pub recent_attached: Option<DotnetProcess>,
}

impl StateDocument for DebugSessions {
    const FILE: &'static str = "debug.json";
}
