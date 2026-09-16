//! Deterministic Skill Router (SPEC §17.3, plan 021 D4–D7).
//!
//! Pure function: same inputs → same loaded names, in a stable order. No LLM, no IO,
//! no permission changes.

use super::{SelectedSkill, Skill, SkillSkip, registry};

use crate::agent::role::TaskKind;

/// Minimum score to become a candidate (language match is +3; one keyword is +2).
const LOAD_THRESHOLD: i32 = 2;

/// Signals the router is allowed to see. Built from profile + map + request in the core.
#[derive(Debug, Clone)]
pub struct RouteInput<'a> {
    pub request: &'a str,
    pub kind: TaskKind,
    pub languages: &'a [String],
    pub frameworks: &'a [String],
    pub paths: &'a [String],
    /// Character budget for the concatenated condensed bodies (section budget).
    pub budget_chars: usize,
}

/// What the orchestrator records and (at task start) emits.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RouteDecision {
    pub detected: Vec<String>,
    pub loaded: Vec<SelectedSkill>,
    pub skipped: Vec<SkillSkip>,
}

pub fn route_builtin(input: &RouteInput<'_>) -> RouteDecision {
    route(&registry(), input)
}

pub fn route(skills: &[&Skill], input: &RouteInput<'_>) -> RouteDecision {
    let request = input.request.to_ascii_lowercase();
    let langs = normalize_list(input.languages);
    let frameworks = normalize_list(input.frameworks);
    let path_tags = tags_from_paths(input.paths);

    let mut scored: Vec<Scored<'_>> = skills
        .iter()
        .copied()
        .map(|skill| {
            let (score, why) =
                score_skill(skill, &request, input.kind, &langs, &frameworks, &path_tags);
            Scored { skill, score, why }
        })
        .collect();

    scored.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.skill.name.cmp(b.skill.name))
    });

    let detected: Vec<String> = scored
        .iter()
        .filter(|row| row.score >= LOAD_THRESHOLD)
        .map(|row| row.skill.name.to_string())
        .collect();

    let mut skipped = Vec::new();
    let mut chosen: Vec<Scored<'_>> = Vec::new();

    for row in scored.iter().filter(|row| row.score >= LOAD_THRESHOLD) {
        if let Some(conflict) = first_conflict(row.skill, &chosen) {
            skipped.push(SkillSkip {
                name: row.skill.name.to_string(),
                reason: format!("conflicts with {conflict}"),
            });
            continue;
        }
        chosen.push(row.clone());
    }

    // Dependencies of chosen skills, even with score 0.
    let mut added = true;
    while added {
        added = false;
        let needed: Vec<&str> = chosen
            .iter()
            .flat_map(|row| row.skill.dependencies.iter().copied())
            .collect();
        for name in needed {
            if chosen.iter().any(|row| row.skill.name == name) {
                continue;
            }
            let Some(skill) = skills.iter().copied().find(|skill| skill.name == name) else {
                skipped.push(SkillSkip {
                    name: name.to_string(),
                    reason: "missing dependency".to_string(),
                });
                continue;
            };
            if let Some(conflict) = first_conflict(skill, &chosen) {
                skipped.push(SkillSkip {
                    name: skill.name.to_string(),
                    reason: format!("dependency conflicts with {conflict}"),
                });
                continue;
            }
            let parent = chosen
                .iter()
                .find(|row| row.skill.dependencies.contains(&name))
                .map(|row| row.skill.name)
                .unwrap_or("unknown");
            chosen.push(Scored {
                skill,
                score: 0,
                why: format!("dependency of {parent}"),
            });
            added = true;
        }
    }

    chosen.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.skill.name.cmp(b.skill.name))
    });

    let mut loaded = Vec::new();
    let mut used_chars = 0usize;
    for row in chosen {
        let body_chars = row.skill.condensed.chars().count() + row.skill.name.len() + 8;
        if used_chars + body_chars > input.budget_chars && !loaded.is_empty() {
            skipped.push(SkillSkip {
                name: row.skill.name.to_string(),
                reason: "over_budget".to_string(),
            });
            continue;
        }
        used_chars += body_chars;
        loaded.push(SelectedSkill {
            name: row.skill.name.to_string(),
            reason: row.why,
        });
    }

    RouteDecision {
        detected,
        loaded,
        skipped,
    }
}

#[derive(Clone)]
struct Scored<'a> {
    skill: &'a Skill,
    score: i32,
    why: String,
}

fn score_skill(
    skill: &Skill,
    request: &str,
    kind: TaskKind,
    langs: &[String],
    frameworks: &[String],
    path_tags: &[String],
) -> (i32, String) {
    let mut score = 0i32;
    let mut reasons: Vec<String> = Vec::new();

    for tag in skill.tags {
        let tag_l = tag.to_ascii_lowercase();
        if langs.iter().any(|lang| lang == &tag_l) {
            score += 3;
            reasons.push(format!("language:{tag}"));
        }
        if frameworks.iter().any(|fw| fw == &tag_l) {
            score += 4;
            reasons.push(format!("framework:{tag}"));
        }
        if path_tags.iter().any(|path| path == &tag_l) {
            score += 2;
            reasons.push(format!("path:{tag}"));
        }
    }

    let mut keyword_hits = 0;
    for keyword in skill.keywords {
        if request.contains(keyword) {
            keyword_hits += 1;
        }
    }
    if keyword_hits > 0 {
        score += 2 + (keyword_hits - 1).min(2);
        reasons.push("keywords".to_string());
    }

    // Kind is a weak hint only: review-like questions boost code-review.
    if skill.name == "code-review" && kind == TaskKind::Question {
        score += 1;
        reasons.push("kind:question".to_string());
    }

    reasons.sort();
    reasons.dedup();
    let why = if reasons.is_empty() {
        "matched".to_string()
    } else {
        reasons.join(", ")
    };
    (score, why)
}

fn first_conflict(skill: &Skill, chosen: &[Scored<'_>]) -> Option<String> {
    for other in chosen {
        if skill.conflicts.contains(&other.skill.name)
            || other.skill.conflicts.contains(&skill.name)
        {
            return Some(other.skill.name.to_string());
        }
    }
    None
}

fn normalize_list(values: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        let lower = value.to_ascii_lowercase();
        push_unique(&mut out, lower.clone());
        match lower.as_str() {
            "js/ts" | "ts" | "js" | "javascript" | "typescript" => {
                push_unique(&mut out, "typescript".into());
                push_unique(&mut out, "javascript".into());
                push_unique(&mut out, "js".into());
                push_unique(&mut out, "ts".into());
            }
            "rust" => {
                push_unique(&mut out, "rust".into());
                push_unique(&mut out, "rs".into());
            }
            _ => {}
        }
    }
    out
}

fn tags_from_paths(paths: &[String]) -> Vec<String> {
    let mut tags = Vec::new();
    for path in paths {
        let lower = path.to_ascii_lowercase();
        if lower.ends_with(".ts") || lower.ends_with(".js") {
            push_unique(&mut tags, "typescript".into());
            push_unique(&mut tags, "javascript".into());
            push_unique(&mut tags, "ts".into());
            push_unique(&mut tags, "js".into());
        }
        if lower.ends_with(".tsx") || lower.ends_with(".jsx") {
            push_unique(&mut tags, "typescript".into());
            push_unique(&mut tags, "react".into());
            push_unique(&mut tags, "tsx".into());
            push_unique(&mut tags, "jsx".into());
        }
        if lower.ends_with(".rs") {
            push_unique(&mut tags, "rust".into());
            push_unique(&mut tags, "rs".into());
        }
        if lower.contains("tailwind") {
            push_unique(&mut tags, "tailwind".into());
        }
        if lower.contains("next.config") {
            push_unique(&mut tags, "nextjs".into());
            push_unique(&mut tags, "next".into());
        }
    }
    tags
}

fn push_unique(out: &mut Vec<String>, value: String) {
    if !out.iter().any(|existing| existing == &value) {
        out.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::{SkillStatus, catalog::CATALOG};

    fn skill<'a>(name: &'a str, catalog: &'a [Skill]) -> &'a Skill {
        catalog.iter().find(|s| s.name == name).expect(name)
    }

    #[test]
    fn eval_style_typescript_test_loads_testing_and_typescript() {
        let skills = registry();
        let decision = route(
            &skills,
            &RouteInput {
                request: "O teste de soma falha. Corrija e rode os testes.",
                kind: TaskKind::Trivial,
                languages: &["JS/TS".to_string()],
                frameworks: &[],
                paths: &["src/soma.ts".to_string()],
                budget_chars: 4_000,
            },
        );
        let names: Vec<&str> = decision.loaded.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"typescript"), "{names:?}");
        assert!(names.contains(&"testing"), "{names:?}");
        assert!(names.contains(&"debugging"), "{names:?}");
        assert!(!names.contains(&"security"), "security is keyword-gated");
        assert!(!names.contains(&"react"));
        for name in ["typescript", "testing", "debugging"] {
            assert!(
                decision.detected.iter().any(|n| n == name),
                "{:?}",
                decision.detected
            );
        }
        let again = route(
            &skills,
            &RouteInput {
                request: "O teste de soma falha. Corrija e rode os testes.",
                kind: TaskKind::Trivial,
                languages: &["JS/TS".to_string()],
                frameworks: &[],
                paths: &["src/soma.ts".to_string()],
                budget_chars: 4_000,
            },
        );
        assert_eq!(decision.loaded, again.loaded, "router is deterministic");
    }

    #[test]
    fn nextjs_pulls_react_and_typescript() {
        let skills = registry();
        let decision = route(
            &skills,
            &RouteInput {
                request: "ajuste a página inicial",
                kind: TaskKind::Normal,
                languages: &[],
                frameworks: &["nextjs".to_string()],
                paths: &[],
                budget_chars: 8_000,
            },
        );
        let names: Vec<&str> = decision.loaded.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"nextjs"), "{names:?}");
        assert!(names.contains(&"react"), "dep of nextjs: {names:?}");
        assert!(names.contains(&"typescript"), "dep of react: {names:?}");
        assert!(
            decision
                .loaded
                .iter()
                .any(|s| s.name == "react" && s.reason.contains("dependency")),
            "{:?}",
            decision.loaded
        );
        assert!(
            decision
                .loaded
                .iter()
                .any(|s| s.name == "typescript" && s.reason.contains("dependency")),
            "{:?}",
            decision.loaded
        );
    }

    #[test]
    fn conflicts_keep_the_higher_score() {
        let a = Skill {
            name: "alpha",
            description: "",
            version: "1",
            origin: "cd-ai",
            license: "Apache-2.0",
            dependencies: &[],
            conflicts: &["beta"],
            tags: &["alpha"],
            keywords: &["alpha-key"],
            permissions: &["read"],
            status: SkillStatus::Active,
            token_budget: 40,
            condensed: "A",
        };
        let b = Skill {
            name: "beta",
            description: "",
            version: "1",
            origin: "cd-ai",
            license: "Apache-2.0",
            dependencies: &[],
            conflicts: &["alpha"],
            tags: &[],
            keywords: &["beta-key"],
            permissions: &["read"],
            status: SkillStatus::Active,
            token_budget: 40,
            condensed: "B",
        };
        let catalog = [&a, &b];
        let decision = route(
            &catalog,
            &RouteInput {
                request: "alpha-key and beta-key",
                kind: TaskKind::Normal,
                languages: &["alpha".to_string()],
                frameworks: &[],
                paths: &[],
                budget_chars: 400,
            },
        );
        let names: Vec<&str> = decision.loaded.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["alpha"]);
        assert!(
            decision
                .skipped
                .iter()
                .any(|skip| skip.name == "beta" && skip.reason.contains("conflicts")),
            "{:?}",
            decision.skipped
        );
    }

    #[test]
    fn budget_drops_the_lowest_score() {
        let skills = registry();
        let decision = route(
            &skills,
            &RouteInput {
                request: "O teste de soma falha. Corrija e rode os testes.",
                kind: TaskKind::Trivial,
                languages: &["JS/TS".to_string()],
                frameworks: &[],
                paths: &["src/soma.ts".to_string()],
                budget_chars: 80,
            },
        );
        assert!(
            !decision.loaded.is_empty(),
            "at least the top skill fits: {:?}",
            decision.loaded
        );
        assert!(
            decision
                .skipped
                .iter()
                .any(|skip| skip.reason == "over_budget"),
            "tight budget must skip: {:?}",
            decision.skipped
        );
        let loaded: Vec<&str> = decision.loaded.iter().map(|s| s.name.as_str()).collect();
        for skip in &decision.skipped {
            assert!(!loaded.contains(&skip.name.as_str()));
        }
    }

    #[test]
    fn security_skill_does_not_claim_a_bypass() {
        let security = skill("security", CATALOG);
        assert_eq!(security.permissions, &["read"]);
        assert!(security.condensed.contains("Do not weaken path checks"));
        assert!(security.condensed.contains("do not add exploit"));
    }

    #[test]
    fn rust_workspace_loads_rust_not_react() {
        let decision = route_builtin(&RouteInput {
            request: "corrija o clippy no crate",
            kind: TaskKind::Trivial,
            languages: &["Rust".to_string()],
            frameworks: &[],
            paths: &["src/lib.rs".to_string()],
            budget_chars: 4_000,
        });
        let names: Vec<&str> = decision.loaded.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"rust"), "{names:?}");
        assert!(!names.contains(&"react"));
        assert!(!names.contains(&"nextjs"));
    }
}
