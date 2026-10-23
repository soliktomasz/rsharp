//! Choices and name-following from Passenger's dialog, independent of rendering and filesystem work.

use rsharp_core::templates::{
    CreateRequest, ProjectTarget, ProjectTemplate, TemplateCatalog, TemplateFamily, TemplateOptionKind,
    TemplateVariant, options,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

pub struct Draft {
    pub directory: PathBuf,
    pub target: Option<ProjectTarget>,
    pub catalog: Option<Arc<TemplateCatalog>>,
    pub authoritative: bool,
    pub query: String,
    pub families: Vec<TemplateFamily>,
    pub listed: Vec<usize>,
    pub family: Option<usize>,
    pub type_index: Option<usize>,
    pub language: Option<String>,
    pub framework: Option<String>,
    pub same_directory: bool,
    pub solution_format: String,
    solution_name: String,
    project_name: String,
    solution_given: String,
    project_given: String,
    values: BTreeMap<String, String>,
    by_person: BTreeMap<String, String>,
    type_of: BTreeMap<String, String>,
    last_chosen: Option<String>,
    last_family: Option<String>,
    occupied: Vec<String>,
}

impl Draft {
    pub fn new(directory: PathBuf, target: Option<ProjectTarget>) -> Self {
        Self {
            directory,
            target,
            catalog: None,
            authoritative: false,
            query: String::new(),
            families: vec![],
            listed: vec![],
            family: None,
            type_index: None,
            language: None,
            framework: None,
            same_directory: false,
            solution_format: "sln".into(),
            solution_name: String::new(),
            project_name: String::new(),
            solution_given: String::new(),
            project_given: String::new(),
            values: BTreeMap::new(),
            by_person: BTreeMap::new(),
            type_of: BTreeMap::new(),
            last_chosen: None,
            last_family: None,
            occupied: vec![],
        }
    }
    pub fn solution_name(&self) -> &str {
        self.solution_name.trim()
    }
    pub fn project_name(&self) -> &str {
        self.project_name.trim()
    }
    pub(super) fn solution_text(&self) -> &str {
        &self.solution_name
    }
    pub(super) fn project_text(&self) -> &str {
        &self.project_name
    }
    pub fn chosen(&self) -> Option<&ProjectTemplate> {
        self.families.get(self.family?)?.types.get(self.type_index?)
    }
    pub fn variant(&self) -> Option<&TemplateVariant> {
        self.chosen().map(|t| t.variant(self.language.as_deref(), self.framework.as_deref()))
    }
    pub fn option_value(&self, symbol: &str) -> Option<&str> {
        self.values.get(symbol).map(String::as_str)
    }
    pub fn option_enabled(&self, symbol: &str) -> bool {
        self.variant()
            .and_then(|v| v.options.iter().find(|o| o.name == symbol))
            .is_some_and(|o| options::is_enabled(o.enabled_when.as_deref(), &self.values))
    }
    pub fn asks_project_name(&self) -> bool {
        self.chosen().is_none_or(|t| !t.is_solution)
    }
    pub fn show_catalog(&mut self, catalog: Arc<TemplateCatalog>, authoritative: bool) {
        self.authoritative = authoritative;
        if self.catalog.is_none() || !catalog.solution_formats.contains(&self.solution_format) {
            self.solution_format = catalog.solution_format.clone();
        }
        self.families = catalog
            .families
            .iter()
            .filter_map(|f| if self.target.is_some() { f.projects() } else { Some(f.clone()) })
            .collect();
        self.catalog = Some(catalog);
        self.filter(false);
    }
    pub fn search(&mut self, query: &str) {
        self.query = query.into();
        self.filter(true);
    }
    fn filter(&mut self, searched: bool) {
        let words: Vec<_> = self.query.split_whitespace().collect();
        let found: Vec<_> =
            self.families.iter().enumerate().filter_map(|(i, f)| f.matches(&words).map(|types| (i, types))).collect();
        self.listed = found.iter().map(|(i, _)| *i).collect();
        let selected = (!searched)
            .then(|| {
                self.last_chosen.as_ref().and_then(|key| {
                    found
                        .iter()
                        .find(|(i, _)| self.families[*i].types.iter().any(|t| t.short_name().eq_ignore_ascii_case(key)))
                        .map(|(i, _)| *i)
                })
            })
            .flatten()
            .or_else(|| {
                self.last_family
                    .as_ref()
                    .and_then(|name| self.listed.iter().copied().find(|i| self.families[*i].name == *name))
            })
            .or_else(|| self.listed.first().copied());
        let Some(index) = selected else {
            self.family = None;
            self.type_index = None;
            self.values.clear();
            return;
        };
        let matched = &found.iter().find(|(i, _)| *i == index).unwrap().1;
        let family = &self.families[index];
        let named = if searched && !matched.is_empty() {
            matched
                .iter()
                .copied()
                .find(|i| {
                    self.type_of
                        .get(&family.name)
                        .is_some_and(|key| family.types[*i].short_name().eq_ignore_ascii_case(key))
                })
                .or_else(|| matched.first().copied())
                .map(|i| family.types[i].short_name().to_string())
        } else {
            None
        };
        let kept = self
            .last_chosen
            .as_ref()
            .filter(|key| family.types.iter().any(|t| t.short_name().eq_ignore_ascii_case(key)))
            .cloned();
        self.select_family(index, named.or(kept).as_deref());
    }
    pub fn choose_family(&mut self, index: usize) {
        if self.listed.contains(&index) {
            self.select_family(index, None);
        }
    }
    fn select_family(&mut self, index: usize, preferred: Option<&str>) {
        let Some(family) = self.families.get(index) else { return };
        if family.types.is_empty() {
            return;
        }
        let key = preferred.or_else(|| self.type_of.get(&family.name).map(String::as_str));
        let pick =
            key.and_then(|key| family.types.iter().position(|t| t.short_name().eq_ignore_ascii_case(key))).unwrap_or(0);
        self.family = Some(index);
        self.type_index = Some(pick);
        self.remember_choice();
        self.retarget();
        self.default_names();
    }
    pub fn choose_type(&mut self, short_name: &str) {
        let Some(family) = self.family.and_then(|i| self.families.get(i)) else { return };
        let Some(pick) = family.types.iter().position(|t| t.short_name().eq_ignore_ascii_case(short_name)) else {
            return;
        };
        self.type_index = Some(pick);
        self.remember_choice();
        self.retarget();
        self.default_names();
    }
    fn remember_choice(&mut self) {
        if let Some(chosen) = self.chosen().cloned() {
            let key = chosen.short_name().to_lowercase();
            self.type_of.insert(chosen.family.clone(), key.clone());
            self.last_family = Some(chosen.family);
            self.last_chosen = Some(key);
        }
    }
    pub fn set_language(&mut self, language: &str) {
        if self.chosen().is_some_and(|t| t.languages().iter().any(|l| l == language)) {
            self.language = Some(language.into());
            self.retarget();
        }
    }
    pub fn set_framework(&mut self, framework: &str) {
        if self.chosen().is_some_and(|t| t.frameworks(self.language.as_deref()).iter().any(|f| f == framework)) {
            self.framework = Some(framework.into());
            self.retarget();
        }
    }
    fn retarget(&mut self) {
        let Some(template) = self.chosen().cloned() else {
            self.values.clear();
            return;
        };
        let languages = template.languages();
        if self.language.as_ref().is_none_or(|l| !languages.contains(l)) {
            self.language = languages.first().cloned();
        }
        let frameworks = template.frameworks(self.language.as_deref());
        if self.framework.as_ref().is_none_or(|f| !frameworks.contains(f)) {
            self.framework = template
                .variant(self.language.as_deref(), None)
                .default_framework
                .clone()
                .or_else(|| frameworks.first().cloned());
        }
        let variant = template.variant(self.language.as_deref(), self.framework.as_deref());
        self.values = variant
            .options
            .iter()
            .map(|o| {
                let value = self
                    .by_person
                    .get(&o.name)
                    .filter(|v| o.kind != TemplateOptionKind::Choice || o.choices.iter().any(|c| c.value == **v))
                    .unwrap_or(&o.default);
                (o.name.clone(), value.clone())
            })
            .collect();
    }
    pub fn set_option(&mut self, symbol: &str, value: String) {
        if self.option_enabled(symbol) {
            self.values.insert(symbol.into(), value.clone());
            self.by_person.insert(symbol.into(), value);
        }
    }
    fn project_named(&self) -> bool {
        !self.project_name().is_empty() && self.project_name() != self.project_given
    }
    fn solution_named(&self) -> bool {
        !self.solution_name().is_empty() && self.solution_name() != self.solution_given
    }
    pub fn set_solution_name(&mut self, value: String) {
        let follows = !self.project_named();
        self.solution_name = value;
        if follows {
            self.project_name = self.solution_name().into();
            self.project_given = self.project_name.clone();
        }
    }
    pub fn set_project_name(&mut self, value: String) {
        self.project_name = value;
    }
    pub fn set_occupied_names(&mut self, names: Vec<String>) {
        self.occupied = names;
        self.default_names();
    }
    fn default_names(&mut self) {
        let Some(chosen) = self.chosen() else { return };
        let base = chosen.default_name.clone();
        let free = |name: &str| {
            !self.occupied.iter().any(|n| n == name)
                && self.target.as_ref().is_none_or(|t| !t.projects.iter().any(|p| p.eq_ignore_ascii_case(name)))
        };
        let name = (1..).map(|n| format!("{base}{n}")).find(|n| free(n)).unwrap();
        let project_named = self.project_named();
        if self.target.is_none() {
            if !self.solution_named() {
                self.solution_name = name;
                self.solution_given = self.solution_name.clone();
            }
            if !project_named {
                self.project_name = self.solution_name().into();
                self.project_given = self.project_name.clone();
            }
        } else if !project_named {
            self.project_name = name;
            self.project_given = self.project_name.clone();
        }
    }
    pub fn request(&self) -> Result<CreateRequest, String> {
        if !self.authoritative {
            return Err("The templates are still loading".into());
        }
        let template = self.chosen().ok_or("Choose a template")?;
        if let Some(pack) = &template.pack {
            return Err(format!("{} comes with {}, which is not installed: Install it first", template.name, pack.id));
        }
        Ok(CreateRequest {
            target: self.target.clone(),
            template: template.clone(),
            language: self.language.clone(),
            framework: self.framework.clone(),
            solution_name: self.solution_name().into(),
            project_name: self.project_name().into(),
            directory: self.directory.clone(),
            same_directory: self.same_directory,
            solution_format: self.solution_format.clone(),
            values: self.values.clone(),
        })
    }
}

#[cfg(test)]
mod tests;
