//! Offers transcribed from Passenger's TemplatePacks.cs, with their source versions in packs.json.

use super::*;

pub fn all() -> Vec<TemplatePack> {
    serde_json::from_str(include_str!("packs.json")).expect("the carried template-pack metadata is valid")
}

pub fn offered(installed: &[ProjectTemplate]) -> Vec<ProjectTemplate> {
    let mut offered = Vec::new();
    for pack in all() {
        for kind in &pack.kinds {
            if installed.iter().any(|t| t.short_names.iter().any(|s| s.eq_ignore_ascii_case(&kind.short_name))) {
                continue;
            }
            offered.push(ProjectTemplate {
                name: kind.name.clone(),
                short_names: vec![kind.short_name.clone()],
                author: pack.author.clone(),
                classifications: kind.classifications.clone(),
                default_name: kind.default_name.clone(),
                variants: kind
                    .languages
                    .iter()
                    .map(|language| TemplateVariant {
                        identity: format!("{}::{}::{language}", pack.id, kind.short_name),
                        language: Some(language.clone()),
                        description: kind.description.clone(),
                        ..Default::default()
                    })
                    .collect(),
                family: kind.name.clone(),
                type_name: kind.name.clone(),
                is_solution: kind.is_solution,
                pack: Some(pack.clone()),
            });
        }
    }
    offered
}
