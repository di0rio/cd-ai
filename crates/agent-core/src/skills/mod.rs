//! Built-in skills (SPEC §17, plan 021): registry, license gate, condensed bodies, router.
//!
//! A skill is operational knowledge, not a tool. Nothing here changes path checks, sandbox,
//! or PermissionMode. External names without a redistribution license are not embedded.

mod catalog;
mod router;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub use catalog::{CATALOG, UNLICENSED_CONCEPTUAL, find};
pub use router::{RouteDecision, RouteInput, route, route_builtin};

/// SPDX-like identifiers that allow redistributing the skill text inside this app.
pub const REDISTRIBUTABLE_LICENSES: &[&str] = &[
    "MIT",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "CC0-1.0",
    "Unlicense",
];

/// Whether this license text may be shipped in the binary (SPEC §17.4).
pub fn license_allows_embed(license: &str) -> bool {
    REDISTRIBUTABLE_LICENSES.contains(&license)
}

/// Lifecycle. Disabled skills stay in the catalog for tests but never load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillStatus {
    Active,
    Disabled,
}

/// One built-in skill. Static: the catalog is compiled in, never downloaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: &'static str,
    pub description: &'static str,
    pub version: &'static str,
    pub origin: &'static str,
    pub license: &'static str,
    pub dependencies: &'static [&'static str],
    pub conflicts: &'static [&'static str],
    pub tags: &'static [&'static str],
    pub keywords: &'static [&'static str],
    /// Declared tools/actions the skill *talks about*. Never granted by the router.
    pub permissions: &'static [&'static str],
    pub status: SkillStatus,
    /// Max size of [`Skill::condensed`] in `chars/4` tokens.
    pub token_budget: u32,
    pub condensed: &'static str,
}

/// A skill the orchestrator loaded for a task, with the reason the router kept it (SPEC §23).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SelectedSkill {
    pub name: String,
    pub reason: String,
}

/// A skill the router considered and then dropped, with why (SPEC §17.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SkillSkip {
    pub name: String,
    pub reason: String,
}

/// Licensed, active skills. This is the only catalog the router sees at runtime.
pub fn registry() -> Vec<&'static Skill> {
    CATALOG
        .iter()
        .filter(|skill| license_allows_embed(skill.license) && skill.status == SkillStatus::Active)
        .collect()
}

/// Plain text for the skills section of the system prompt, clipped to `max_chars`.
pub fn render(selected: &[SelectedSkill], max_chars: usize) -> String {
    let mut out = String::new();
    for item in selected {
        let Some(skill) = find(&item.name) else {
            continue;
        };
        let block = format!("### {}\n{}\n", skill.name, skill.condensed.trim());
        let next = out.chars().count() + block.chars().count();
        if next > max_chars && !out.is_empty() {
            break;
        }
        out.push_str(&block);
        if next > max_chars {
            break;
        }
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_entry_is_licensed_original_and_within_budget() {
        assert!(!CATALOG.is_empty());
        for skill in CATALOG {
            assert!(
                license_allows_embed(skill.license),
                "{}: license {} is not redistributable",
                skill.name,
                skill.license
            );
            assert_eq!(skill.origin, "cd-ai");
            assert_eq!(skill.version, "1");
            assert_eq!(skill.status, SkillStatus::Active);
            let tokens = skill.condensed.chars().count() as u32 / 4;
            assert!(
                tokens <= skill.token_budget,
                "{}: {tokens} tok > budget {}",
                skill.name,
                skill.token_budget
            );
            assert!(!skill.condensed.is_empty());
            assert!(
                !skill
                    .permissions
                    .iter()
                    .any(|p| *p == "bypass" || *p == "unsandbox" || *p == "full-access"),
                "{} must not declare a permission that sounds like a bypass",
                skill.name
            );
        }
    }

    #[test]
    fn third_party_conceptual_names_are_not_embedded() {
        let names: Vec<&str> = CATALOG.iter().map(|skill| skill.name).collect();
        for (name, _) in UNLICENSED_CONCEPTUAL {
            assert!(
                !names.contains(name),
                "{name} has no redistribution license and must not ship"
            );
            assert!(!license_allows_embed("NONE"));
            assert!(!license_allows_embed("proprietary"));
            assert!(!license_allows_embed(""));
        }
        assert!(UNLICENSED_CONCEPTUAL.len() >= 6);
    }

    #[test]
    fn registry_drops_an_unlicensed_skill() {
        let sneaky = Skill {
            name: "stolen",
            description: "no",
            version: "1",
            origin: "somewhere",
            license: "NONE",
            dependencies: &[],
            conflicts: &[],
            tags: &[],
            keywords: &[],
            permissions: &[],
            status: SkillStatus::Active,
            token_budget: 10,
            condensed: "secret sauce",
        };
        let allowed = [sneaky];
        let licensed: Vec<&Skill> = allowed
            .iter()
            .filter(|skill| license_allows_embed(skill.license))
            .collect();
        assert!(licensed.is_empty());
        assert!(!registry().iter().any(|skill| skill.name == "stolen"));
    }

    #[test]
    fn apache_and_mit_are_allowed() {
        assert!(license_allows_embed("Apache-2.0"));
        assert!(license_allows_embed("MIT"));
    }
}
