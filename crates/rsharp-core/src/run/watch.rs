//! What `dotnet watch` says it made of a save, in Rider's and Passenger's words: its own lines name the outcome, and the
//! person reads a sentence about their program instead.

/// What a save came to, as `dotnet watch` said it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The runtime took the change and the program runs on with it.
    Applied,
    /// Nothing in the change was code the runtime could apply (a comment, a resource the program does not read).
    NothingToApply,
    /// The runtime refused the change: the program keeps running the code it has until it is restarted.
    NotApplied,
    /// The change cannot be applied, so the program is restarted with it.
    Restarting,
    /// The change does not compile, so nothing was applied.
    Errors,
}

impl Verdict {
    /// The person's sentence for it, as the status bar says it.
    pub fn said(self) -> &'static str {
        match self {
            Verdict::Applied => "Hot Reload: dotnet watch applied the changes",
            Verdict::NothingToApply => "Hot Reload: no managed code changes to apply",
            Verdict::NotApplied => "Hot Reload: the runtime cannot apply the change; restart the program to run it",
            Verdict::Restarting => "Hot Reload: dotnet watch is restarting the application to apply the changes",
            Verdict::Errors => "Hot Reload: dotnet watch could not apply the changes: the code does not compile",
        }
    }
}

/// The outcome a line of `dotnet watch`'s output carries, if it carries one: the phrases the .NET SDK writes as it
/// applies a save, each as its own version spells them. A phrase that says nothing was applied is read before one that
/// says something was, since it holds the same words: "no changes applied" is not "changes applied".
pub fn verdict(line: &str) -> Option<Verdict> {
    const PHRASES: &[(&str, Verdict)] = &[
        ("No changes applied to client because they are not supported by the runtime", Verdict::NotApplied),
        ("Hot reload of changes succeeded", Verdict::Applied),
        ("changes applied", Verdict::Applied),
        ("No managed code changes to apply", Verdict::NothingToApply),
        ("Restart is needed to apply the changes", Verdict::Restarting),
        // The program called a method whose signature an update deleted: watch restarts it.
        ("Attempted to invoke a deleted method implementation", Verdict::Restarting),
        ("Restarting application", Verdict::Restarting),
        ("Unable to apply changes due to compilation errors", Verdict::Errors),
    ];
    PHRASES.iter().find(|(words, _)| line.contains(words)).map(|(_, verdict)| *verdict)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotnet_watchs_lines_are_read_as_the_person_reads_them() {
        // Each way watch answers a save is said in one sentence, whatever the SDK version's own wording is.
        let said = |line: &str| verdict(line).map(Verdict::said);
        assert_eq!(
            said("dotnet watch: Hot reload of changes succeeded: 2 change(s) applied"),
            Some("Hot Reload: dotnet watch applied the changes")
        );
        assert_eq!(said("No managed code changes to apply"), Some(Verdict::NothingToApply.said()));
        assert_eq!(said("Restart is needed to apply the changes"), Some(Verdict::Restarting.said()));
        assert_eq!(
            said("Attempted to invoke a deleted method implementation in 'Shop.Program.Main'"),
            Some(Verdict::Restarting.said()),
            "a call to a method an update deleted restarts the program"
        );
        assert_eq!(said("Restarting application"), Some(Verdict::Restarting.said()));
        assert_eq!(
            said("Unable to apply changes due to compilation errors. Please revert the changes or fix the errors"),
            Some("Hot Reload: dotnet watch could not apply the changes: the code does not compile")
        );
        assert_eq!(
            said("No changes applied to client because they are not supported by the runtime"),
            Some("Hot Reload: the runtime cannot apply the change; restart the program to run it"),
            "a change the runtime refused is not a change that was applied"
        );
        assert_ne!(
            verdict("No changes applied to client because they are not supported by the runtime"),
            Some(Verdict::Applied),
            "the negative form holds the words of the success one"
        );
        assert_eq!(said("dotnet watch 🔥 Hot reload enabled"), None, "watch starting says nothing of a save");
        assert_eq!(said("Now listening on: http://localhost:5000"), None, "nor does the program's own output");
        assert_eq!(verdict(""), None);
    }
}
