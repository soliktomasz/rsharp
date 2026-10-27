//! The window's side of the code help in XAML files: each open Avalonia or WPF XAML file given the XAML host and the
//! project it is read against, an element's name found and renamed with its code-behind's uses through Roslyn, the build's XAML problems marked in it, and the host told what changed on disk.

use super::*;

impl MainWindow {
    /// Gives a XAML file's buffer the XAML host, which starts with the first.
    pub(super) fn attach_xaml(&mut self, buffer: &Entity<Buffer>, cx: &mut Context<Self>) {
        let path = buffer.read(cx).path.clone();
        if !rsharp_lsp::xaml::is_xaml(&path) || !cx.global::<crate::state::Ide>().servers {
            return;
        }
        if self.xaml.is_none() {
            self.xaml = rsharp_lsp::xaml::XamlHost::carried();
        }
        let Some(host) = self.xaml.clone() else { return };
        let project = self.msbuild_project(&path);
        buffer.update(cx, |b, cx| b.attach_xaml(host, project, cx));
    }

    /// The build that just ended: its XAML compiler's problems marked in each open XAML file, the last build's gone.
    pub(super) fn xaml_built(&self, cx: &mut Context<Self>) {
        for buffer in self.workspace_buffers(cx) {
            if rsharp_lsp::xaml::is_xaml(&buffer.read(cx).path) {
                buffer.update(cx, |b, cx| b.built(&self.build_problems, cx));
            }
        }
    }

    /// Files another program wrote: the projects that hold them read again, a project file's read afresh.
    // shortcut: the watcher passes over obj/, so a restore landing while a XAML file is open is not seen until the
    // project file changes; report project.assets.json apart, as Passenger's watcher does, once restores are run here.
    pub(super) fn xaml_files_changed(&self, paths: &[PathBuf], cx: &mut Context<Self>) {
        let Some(host) = self.xaml.clone() else { return };
        let paths = paths.to_vec();
        cx.background_executor()
            .spawn(async move {
                if let Err(failed) = host.changed(&paths) {
                    log::info!("XAML code help: {failed}");
                }
            })
            .detach();
    }

    /// Find Usages or Rename of an element's name: where its XAML names it, and where its code-behind uses the field
    /// made for it, which Roslyn's references give from a mention whose definition is that generated field.
    // shortcut: a rename's files are each one undo step, not one command undone whole as Passenger's is; make them one
    // with global undo, as C#'s renames need too.
    pub(super) fn code_behind(&mut self, asked: &editor::CodeBehind, window: &mut Window, cx: &mut Context<Self>) {
        let named = asked.named.clone();
        if let Some(name) = asked.rename.as_deref().filter(|n| !is_identifier(n)) {
            self.said = Some(format!("'{name}' cannot name a field in the code-behind").into());
            cx.notify();
            return;
        }
        let xaml: Vec<rsharp_lsp::Location> =
            named.uses.iter().map(|span| rsharp_lsp::Location { path: asked.xaml.clone(), span: *span }).collect();
        let texts: Vec<(PathBuf, String)> = named
            .files
            .iter()
            .filter_map(|path| match crate::buffer::find(path, cx) {
                Some(open) => Some((path.clone(), open.read(cx).document.text().to_string())),
                None => std::fs::read_to_string(path).ok().map(|text| (path.clone(), text)),
            })
            .collect();
        let server = self.server.clone();
        let rename = asked.rename.clone();
        if rename.is_some() && server.is_none() {
            self.said = Some("Cannot rename the XAML element while Roslyn is unavailable".into());
            cx.notify();
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            let code = match &server {
                Some(server) => code_behind_uses(server, texts, &named.name).await,
                None => Vec::new(),
            };
            let _ = this.update_in(cx, |w, window, cx| {
                let places: Vec<_> = xaml.into_iter().chain(code).collect();
                let Some(name) = rename else {
                    let go = editor::Navigate { title: format!("Usages of {}", named.name), to: places, list: true };
                    w.navigate(&go, window, cx);
                    return;
                };
                let mut files: Vec<(PathBuf, Vec<rsharp_lsp::TextEdit>)> = Vec::new();
                for place in places {
                    let edit = rsharp_lsp::TextEdit { span: place.span, text: name.clone() };
                    match files.iter_mut().find(|(path, _)| *path == place.path) {
                        Some((_, edits)) => edits.push(edit),
                        None => files.push((place.path, vec![edit])),
                    }
                }
                let count = files.len();
                let edit = rsharp_lsp::WorkspaceEdit { files, ..Default::default() };
                w.said = Some(match crate::language::apply_workspace_edit(&edit, server.as_deref(), cx) {
                    Ok(()) if count > 1 => format!("Renamed in {count} files").into(),
                    Ok(()) => "Renamed".into(),
                    Err(why) => why.into(),
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// Go to Declaration from C# on a named element's generated field: where its class's XAML names it.
    pub(super) fn open_xaml_field(
        &mut self,
        field: &editor::XamlField,
        from: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.xaml.is_none() && cx.global::<crate::state::Ide>().servers {
            self.xaml = rsharp_lsp::xaml::XamlHost::carried();
        }
        let Some(host) = self.xaml.clone() else { return };
        let project = self.msbuild_project(from);
        let (class, name) = (field.class.clone(), field.name.clone());
        let found = cx.background_executor().spawn(async move { host.element(&project, &class, &name) });
        cx.spawn_in(window, async move |this, cx| {
            let found = found.await.ok().flatten();
            let _ = this.update_in(cx, |w, window, cx| match found {
                Some(to) => w.open_file(&to.path, Some((to.span.start.line, to.span.start.character)), window, cx),
                None => {
                    w.said = Some("Cannot find declaration to go to".into());
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

/// Whether a name can be a C# field's: a letter or underscore, then letters, digits and underscores.
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_') && chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// The code-behind's uses of a named element's field: Roslyn's references from the first mention of the name whose
/// definition is the generated field, the generated places left out (Roslyn gives them as non-file URIs, or WPF's
/// `obj/…/File.g.cs`).
async fn code_behind_uses(
    server: &rsharp_lsp::LanguageServer,
    files: Vec<(PathBuf, String)>,
    name: &str,
) -> Vec<rsharp_lsp::Location> {
    for (path, text) in files {
        let uri = rsharp_core::paths::to_uri(&path);
        let rope = rsharp_text::Rope::from_str(&text);
        for at in mentions(&text, name) {
            let here = rsharp_lsp::at(&uri, crate::language::point(&rope, at));
            let Ok(declared) = server.ask("textDocument/definition", here.clone()).await else { continue };
            let generated = crate::editor::generated_field(&declared).is_some()
                || rsharp_lsp::locations(&declared).iter().any(|l| is_generated(&l.path));
            if !generated {
                continue;
            }
            let mut params = here;
            params["context"] = serde_json::json!({ "includeDeclaration": false });
            let found = server.ask("textDocument/references", params).await.unwrap_or_default();
            return rsharp_lsp::locations(&found).into_iter().filter(|l| !is_generated(&l.path)).collect();
        }
    }
    Vec::new()
}

/// Where `name` stands as a whole word, unqualified or qualified by `this.`.
fn mentions<'a>(text: &'a str, name: &'a str) -> impl Iterator<Item = usize> + 'a {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    text.match_indices(name).map(|(at, _)| at).filter(move |&at| {
        let before = text[..at].strip_suffix("this.").unwrap_or(&text[..at]);
        !before.chars().next_back().is_some_and(|c| word(c) || c == '.')
            && !text[at + name.len()..].chars().next().is_some_and(word)
    })
}

/// WPF's MarkupCompile output for a XAML file, `obj/…/File.g.cs` (`.g.i.cs` at design time).
fn is_generated(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    (name.ends_with(".g.cs") || name.ends_with(".g.i.cs")) && path.components().any(|c| c.as_os_str() == "obj")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn a_rename_without_roslyn_keeps_the_xaml_unchanged(cx: &mut gpui::TestAppContext) {
        let root = std::env::temp_dir().join(format!("rsharp-xaml-rename-no-roslyn-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("MainWindow.axaml");
        let text = "<Button Name=\"Go\" />";
        std::fs::write(&path, text).unwrap();
        cx.update(|cx| {
            crate::init(root.join("state"), cx);
            cx.global_mut::<crate::state::Ide>().servers = false;
            cx.global_mut::<crate::templates::Templates>().enabled = false;
        });
        let (main, cx) = cx.add_window_view(MainWindow::new);
        main.update_in(cx, |main, window, cx| {
            main.open_file(&path, None, window, cx);
            let asked = editor::CodeBehind {
                xaml: path.clone(),
                named: rsharp_lsp::xaml::XamlNamed {
                    name: "Go".into(),
                    uses: vec![rsharp_lsp::Span {
                        start: rsharp_lsp::Point { line: 0, character: 14 },
                        end: rsharp_lsp::Point { line: 0, character: 16 },
                    }],
                    files: Vec::new(),
                },
                rename: Some("Start".into()),
            };
            main.code_behind(&asked, window, cx);
        });
        cx.run_until_parked();
        main.read_with(cx, |main, cx| {
            assert_eq!(
                main.editor().unwrap().read(cx).text().to_string(),
                text,
                "the XAML was renamed without its code-behind"
            );
            assert_eq!(main.said(), Some("Cannot rename the XAML element while Roslyn is unavailable"));
        });
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            text,
            "the XAML on disk was renamed without its code-behind"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_mention_is_a_whole_word_not_after_a_dot() {
        let text = "Go.Content = x.Go; GoOn(); var go = Go;";
        assert_eq!(super::mentions(text, "Go").collect::<Vec<_>>(), [0, 36]);
    }

    #[test]
    fn a_this_qualified_mention_is_allowed_but_other_objects_are_excluded() {
        for text in ["Go", "this.Go", "(this.Go)", "this.Go.Content"] {
            assert_eq!(
                super::mentions(text, "Go").collect::<Vec<_>>(),
                [text.find("Go").unwrap()],
                "missing field in {text}"
            );
        }
        for text in ["x.Go", "x.this.Go", "notthis.Go", "_this.Go", "this.GoOn", "this.Go_", "this.Go1", "éGo", "Goé"]
        {
            assert!(super::mentions(text, "Go").next().is_none(), "accepted a different field or word in {text}");
        }
    }
}
