//! Passenger's family classification and Rider's ordering; a Type is a whole template group.

use super::{ProjectTemplate, TemplateFamily};

const ORDER: &[&str] = &[
    "Console",
    "Class Library",
    "Desktop",
    "Web",
    "AI Extensions",
    "Services",
    "Unit Test",
    "Aspire",
    "Azure Functions",
    "MAUI",
];

impl TemplateFamily {
    pub fn not_installed(&self) -> bool {
        self.types.iter().all(|t| t.pack.is_some())
    }
    pub fn projects(&self) -> Option<Self> {
        let types: Vec<_> = self.types.iter().filter(|t| !t.is_solution).cloned().collect();
        (!types.is_empty()).then(|| Self { name: self.name.clone(), types })
    }
    pub fn matches(&self, words: &[&str]) -> Option<Vec<usize>> {
        let has = |text: &str, word: &str| text.to_lowercase().contains(&word.to_lowercase());
        if words.iter().all(|w| has(&self.name, w)) {
            return Some(Vec::new());
        }
        let found: Vec<_> = self
            .types
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                words.iter().all(|w| {
                    has(&self.name, w)
                        || has(&t.name, w)
                        || has(&t.type_name, w)
                        || t.short_names.iter().any(|s| has(s, w))
                })
            })
            .map(|(i, _)| i)
            .collect();
        (!found.is_empty()).then_some(found)
    }
}

fn kind(t: &ProjectTemplate) -> Option<&'static str> {
    let has = |tag: &str| t.classifications.iter().any(|c| c.eq_ignore_ascii_case(tag));
    if has("AI") {
        Some("AI Extensions")
    } else if has("Aspire") || has(".NET Aspire") {
        Some("Aspire")
    } else if has("Azure Functions") {
        Some("Azure Functions")
    } else if has("Test") {
        Some("Unit Test")
    } else if has("MAUI") {
        Some("MAUI")
    } else if has("WPF") || has("WinForms") {
        Some("Desktop")
    } else if has("Web") && !has("Worker") && !has("Cloud") {
        Some("Web")
    } else if has("Library") {
        Some("Class Library")
    } else if has("Worker") {
        Some("Services")
    } else if has("Console") {
        Some("Console")
    } else {
        None
    }
}

pub fn of(templates: Vec<ProjectTemplate>) -> Vec<TemplateFamily> {
    let mut known: Vec<(&str, Vec<ProjectTemplate>)> = Vec::new();
    let mut rest: Vec<(String, Vec<ProjectTemplate>)> = Vec::new();
    for template in templates {
        if let Some(name) = kind(&template) {
            if let Some((_, group)) = known.iter_mut().find(|(n, _)| *n == name) {
                group.push(template);
            } else {
                known.push((name, vec![template]));
            }
        } else {
            let prefix = template.short_name().split(['.', '-']).next().unwrap_or("").to_lowercase();
            if let Some((_, group)) = rest.iter_mut().find(|(p, _)| *p == prefix) {
                group.push(template);
            } else {
                rest.push((prefix, vec![template]));
            }
        }
    }
    known.sort_by_key(|(name, _)| ORDER.iter().position(|n| n == name).unwrap());
    let mut result: Vec<_> = known
        .into_iter()
        .map(|(name, templates)| {
            let (cut, lead): (&[&str], &[&str]) = match name {
                "Aspire" => (&[".NET Aspire ", "Aspire "], &["aspire-starter", "aspire"]),
                "Unit Test" => (&[" Test Project"], &["xunit", "nunit", "mstest"]),
                "MAUI" => (&[".NET MAUI "], &[]),
                "Web" => (&["ASP.NET Core "], &["webapp", "mvc", "webapi", "web"]),
                "Class Library" => (&[" Class Library"], &["classlib"]),
                _ => (&[], &[]),
            };
            family(name, templates, cut, lead)
        })
        .collect();
    let mut others = Vec::new();
    for (_, templates) in rest {
        let words: Vec<Vec<_>> = templates.iter().map(|t| t.name.split_whitespace().collect()).collect();
        let mut count = 0;
        if templates.len() > 1 {
            while words.iter().all(|w| w.len() > count + 1 && w[count].eq_ignore_ascii_case(words[0][count])) {
                count += 1;
            }
        }
        let shared = words[0][..count].join(" ");
        if shared.is_empty() {
            others.extend(templates.into_iter().map(|t| family(&t.name.clone(), vec![t], &[], &[])));
        } else {
            others.push(family(&shared, templates, &[&format!("{shared} .NET "), &format!("{shared} ")], &[]));
        }
    }
    others.sort_by_key(|f| f.name.to_lowercase());
    result.extend(others);
    result
}

fn family(name: &str, mut types: Vec<ProjectTemplate>, cut: &[&str], lead: &[&str]) -> TemplateFamily {
    for t in &mut types {
        t.family = name.to_string();
        let mut shortened = t.name.clone();
        for cut in cut {
            if shortened.get(..cut.len()).is_some_and(|s| s.eq_ignore_ascii_case(cut)) {
                shortened = shortened[cut.len()..].to_string();
            } else if shortened.len() >= cut.len()
                && shortened.get(shortened.len() - cut.len()..).is_some_and(|s| s.eq_ignore_ascii_case(cut))
            {
                shortened.truncate(shortened.len() - cut.len());
            }
        }
        t.type_name = if shortened.trim().is_empty() { t.name.clone() } else { shortened.trim().into() };
    }
    types.sort_by_key(|t| {
        (
            t.pack.is_some(),
            lead.iter().position(|s| t.short_names.iter().any(|n| n.eq_ignore_ascii_case(s))).unwrap_or(lead.len()),
            t.is_solution,
            t.type_name.to_lowercase(),
        )
    });
    TemplateFamily { name: name.into(), types }
}
