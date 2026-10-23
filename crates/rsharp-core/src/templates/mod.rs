//! dotnet new's installed templates and Rider's project families, with the options an IDE asks for.

pub mod catalog;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateCatalog {
    pub sdk: String,
    pub families: Vec<TemplateFamily>,
    pub solution_formats: Vec<String>,
    pub solution_format: String,
}

impl TemplateCatalog {
    pub fn templates(&self) -> Vec<&ProjectTemplate> {
        self.families.iter().flat_map(|f| &f.types).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateFamily {
    pub name: String,
    pub types: Vec<ProjectTemplate>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ProjectTemplate {
    pub name: String,
    pub short_names: Vec<String>,
    pub author: String,
    pub classifications: Vec<String>,
    pub default_name: String,
    pub variants: Vec<TemplateVariant>,
    pub family: String,
    pub type_name: String,
    pub is_solution: bool,
    pub pack: Option<TemplatePack>,
}

impl ProjectTemplate {
    pub fn short_name(&self) -> &str {
        &self.short_names[0]
    }

    pub fn languages(&self) -> Vec<String> {
        let mut languages: Vec<_> = self.variants.iter().filter_map(|v| v.language.clone()).collect();
        languages.sort_by(|a, b| language_order(a).cmp(&language_order(b)).then(a.cmp(b)));
        languages.dedup();
        languages
    }

    pub fn frameworks(&self, language: Option<&str>) -> Vec<String> {
        let mut frameworks = Vec::new();
        for framework in self.variants.iter().filter(|v| v.language.as_deref() == language).flat_map(|v| &v.frameworks)
        {
            if !frameworks.contains(framework) {
                frameworks.push(framework.clone());
            }
        }
        frameworks
    }

    pub fn variant(&self, language: Option<&str>, framework: Option<&str>) -> &TemplateVariant {
        self.variants
            .iter()
            .find(|v| {
                v.language.as_deref() == language
                    && framework.is_none_or(|f| v.frameworks.is_empty() || v.frameworks.iter().any(|v| v == f))
            })
            .or_else(|| self.variants.iter().find(|v| v.language.as_deref() == language))
            .unwrap_or(&self.variants[0])
    }
}

pub(crate) fn language_order(language: &str) -> usize {
    match language {
        "C#" => 0,
        "F#" => 1,
        "VB" => 2,
        _ => 3,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct TemplateVariant {
    pub identity: String,
    pub language: Option<String>,
    pub precedence: i64,
    pub description: String,
    pub framework_option: Option<String>,
    pub frameworks: Vec<String>,
    pub default_framework: Option<String>,
    pub options: Vec<TemplateOption>,
    pub skip_restore_option: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateOptionKind {
    Bool,
    Choice,
    Text,
    Integer,
    Float,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateOption {
    pub name: String,
    pub option: String,
    pub label: String,
    pub description: Option<String>,
    pub kind: TemplateOptionKind,
    pub default: String,
    pub choices: Vec<TemplateChoice>,
    pub inverted: bool,
    pub required: bool,
    pub enabled_when: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateChoice {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplatePack {
    pub id: String,
    pub offer: String,
    pub version: String,
    pub author: String,
    pub kinds: Vec<PackKind>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackKind {
    pub name: String,
    pub short_name: String,
    pub is_solution: bool,
    pub languages: Vec<String>,
    pub description: String,
    pub classifications: Vec<String>,
    pub default_name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplatePlatform {
    MacOS,
    Windows,
    Linux,
}

impl TemplatePlatform {
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOS
        } else {
            Self::Linux
        }
    }
}
