//! Model Router (SPEC §10, plan 022): FAST / CODER / REASONER as a pure function.
//!
//! Model names are configuration (decision 0002, rule 5). The router never hard-codes a tag.
//! Switching costs a load; the already-resident model wins unless the gain is real (FAST cannot
//! edit, or the Coder has failed enough times to justify REASONER).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::agent::role::TaskKind;
use crate::ollama::{LoadedModel, ModelInfo};

/// Failures of quality (Verifier correction or invalid tool call) before escalating.
pub const ESCALATE_AFTER: u32 = 2;
/// FAST and trivial-edit windows (decision 0002: 8k is the measured FAST point).
pub const FAST_CTX: u32 = 8_192;
pub const TRIVIAL_CTX: u32 = 8_192;

/// Prefill-cost weights for tests. Not billing: local inference has no token price (0008).
const WEIGHT_FAST: u64 = 1;
const WEIGHT_CODER: u64 = 3;
const WEIGHT_REASONER: u64 = 8;

/// Which configured slot the router picked. Default `Coder` so older task states resume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum ModelCategory {
    Fast,
    #[default]
    Coder,
    Reasoner,
}

impl ModelCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Coder => "coder",
            Self::Reasoner => "reasoner",
        }
    }

    fn weight(self) -> u64 {
        match self {
            Self::Fast => WEIGHT_FAST,
            Self::Coder => WEIGHT_CODER,
            Self::Reasoner => WEIGHT_REASONER,
        }
    }
}

/// User-configured names per category. Empty means "use the task's model for this slot".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelAssignment {
    pub fast: Option<String>,
    pub coder: Option<String>,
    pub reasoner: Option<String>,
}

/// What the provider currently sees. An empty `available` list means "unknown": treat every
/// configured name as present (scripted eval, Ollama down).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelInventory {
    pub available: Vec<ModelSlot>,
    pub loaded: Vec<ModelSlot>,
    pub mem_available_bytes: Option<u64>,
}

/// One local model, as the router needs it: name and on-disk size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSlot {
    pub name: String,
    pub size_bytes: u64,
}

impl From<&ModelInfo> for ModelSlot {
    fn from(model: &ModelInfo) -> Self {
        Self {
            name: model.name.clone(),
            size_bytes: model.size_bytes,
        }
    }
}

impl From<&LoadedModel> for ModelSlot {
    fn from(model: &LoadedModel) -> Self {
        Self {
            name: model.name.clone(),
            size_bytes: model.size_bytes,
        }
    }
}

impl ModelInventory {
    pub fn from_ollama(
        models: &[ModelInfo],
        loaded: &[LoadedModel],
        mem_available_bytes: Option<u64>,
    ) -> Self {
        Self {
            available: models.iter().map(ModelSlot::from).collect(),
            loaded: loaded.iter().map(ModelSlot::from).collect(),
            mem_available_bytes,
        }
    }

    fn knows_listing(&self) -> bool {
        !self.available.is_empty()
    }

    fn is_available(&self, name: &str) -> bool {
        if !self.knows_listing() {
            return true;
        }
        self.available.iter().any(|slot| slot.name == name)
    }

    fn is_loaded(&self, name: &str) -> bool {
        self.loaded.iter().any(|slot| slot.name == name)
    }

    fn loaded_name(&self) -> Option<&str> {
        self.loaded.first().map(|slot| slot.name.as_str())
    }

    fn size_of(&self, name: &str) -> Option<u64> {
        self.available
            .iter()
            .chain(self.loaded.iter())
            .find(|slot| slot.name == name)
            .map(|slot| slot.size_bytes)
    }

    fn fits(&self, name: &str) -> bool {
        if self.is_loaded(name) {
            return true;
        }
        let Some(mem) = self.mem_available_bytes else {
            return true;
        };
        let Some(size) = self.size_of(name) else {
            return true;
        };
        let loaded_total: u64 = self.loaded.iter().map(|slot| slot.size_bytes).sum();
        size <= mem.saturating_add(loaded_total)
    }
}

/// Linux: `MemAvailable` from `/proc/meminfo`. Elsewhere (or unreadable): unknown.
pub fn mem_available_bytes() -> Option<u64> {
    mem_available_from(|path| std::fs::read_to_string(path).ok())
}

fn mem_available_from(read: impl Fn(&str) -> Option<String>) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let text = read("/proc/meminfo")?;
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("MemAvailable:") {
                let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
                return Some(kb.saturating_mul(1024));
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = read;
        None
    }
}

/// Inputs the orchestrator already has when a task starts (or after a quality failure).
#[derive(Debug, Clone)]
pub struct RouteInput<'a> {
    pub kind: TaskKind,
    pub default_model: &'a str,
    pub assignment: &'a ModelAssignment,
    pub inventory: &'a ModelInventory,
    pub requested_ctx: u32,
    pub coder_failures: u32,
}

/// What the loop should send to the provider, plus why (SPEC §28).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteDecision {
    pub category: ModelCategory,
    pub model: String,
    pub num_ctx: u32,
    pub reason: String,
}

impl RouteDecision {
    /// Deterministic stand-in for prefill time: window × category weight.
    /// A live Ollama run remains the real latency number; this is what the gate can prove.
    pub fn expected_prefill_cost(&self) -> u64 {
        u64::from(self.num_ctx).saturating_mul(self.category.weight())
    }
}

/// Picks category, model name and `num_ctx`. Pure: same inputs, same decision.
pub fn route(input: &RouteInput<'_>) -> RouteDecision {
    let default = input.default_model;
    let fast = resolve_slot(input.assignment.fast.as_deref(), input, default);
    let coder = resolve_slot(input.assignment.coder.as_deref(), input, default);
    let reasoner = resolve_slot(input.assignment.reasoner.as_deref(), input, default);

    let mut reasons: Vec<String> = Vec::new();
    if fast == coder && coder == reasoner {
        reasons.push(format!(
            "só um modelo configurado; todas as categorias usam {default}"
        ));
    }

    let wants_reasoner = input.coder_failures >= ESCALATE_AFTER && reasoner != coder;
    let (mut category, mut model) = if wants_reasoner {
        reasons.push(format!(
            "escalonamento após {} falhas do CODER → REASONER",
            input.coder_failures
        ));
        (ModelCategory::Reasoner, reasoner.clone())
    } else if input.kind == TaskKind::Question && fast != coder {
        (ModelCategory::Fast, fast.clone())
    } else {
        (ModelCategory::Coder, coder.clone())
    };

    if !input.inventory.is_available(&model) {
        reasons.push(format!(
            "modelo {model} indisponível; fallback para {default}"
        ));
        model = default.to_string();
        category = category_of(&model, &fast, &coder, &reasoner);
    }

    if let Some(loaded) = input.inventory.loaded_name() {
        if loaded != model {
            let loaded_is_fast_only = loaded == fast && loaded != coder;
            let need_edits = input.kind != TaskKind::Question;
            if wants_reasoner && input.inventory.is_available(&reasoner) {
                // Escalation is the one mid-task switch the spec names.
            } else if need_edits && loaded_is_fast_only {
                reasons.push(
                    "FAST estava carregado; tarefa edita código, troca para CODER".to_string(),
                );
            } else if !need_edits || !loaded_is_fast_only {
                reasons.push(format!("{loaded} já carregado, evita descarregar"));
                model = loaded.to_string();
                category = category_of(&model, &fast, &coder, &reasoner);
            }
        } else {
            reasons.push(format!("{model} já carregado"));
        }
    }

    if !input.inventory.fits(&model) {
        if let Some(loaded) = input.inventory.loaded_name() {
            if loaded != model {
                reasons.push(format!(
                    "{model} não cabe na RAM disponível; usando o modelo já carregado"
                ));
                model = loaded.to_string();
                category = category_of(&model, &fast, &coder, &reasoner);
            }
        } else {
            reasons.push(format!(
                "{model} pode não caber na RAM disponível; seguindo mesmo assim"
            ));
        }
    }

    if input.kind == TaskKind::Question && category == ModelCategory::Fast {
        reasons.push("pergunta; FAST".to_string());
    } else if input.kind == TaskKind::Trivial {
        reasons.push("trivial; CODER (FAST não edita)".to_string());
    } else if input.kind == TaskKind::Question {
        reasons.push("pergunta".to_string());
    } else if matches!(input.kind, TaskKind::Normal | TaskKind::Complex) {
        reasons.push(format!("{}; CODER", kind_label(input.kind)));
    }

    let num_ctx = ctx_for(input.kind, category, input.requested_ctx);
    if num_ctx < input.requested_ctx {
        reasons.push(format!("janela {num_ctx} (pedido {})", input.requested_ctx));
    }

    if reasons.is_empty() {
        reasons.push(format!(
            "{} → {}",
            kind_label(input.kind),
            category.as_str()
        ));
    }

    RouteDecision {
        category,
        model,
        num_ctx,
        reason: reasons.join("; "),
    }
}

fn resolve_slot(configured: Option<&str>, input: &RouteInput<'_>, default: &str) -> String {
    match configured {
        Some(name) if !name.is_empty() && input.inventory.is_available(name) => name.to_string(),
        Some(name) if !name.is_empty() => {
            // Listed but missing from the provider: the caller still sees this in the reason
            // when it was the *chosen* model; for the slot itself we fall back.
            let _ = name;
            default.to_string()
        }
        _ => default.to_string(),
    }
}

fn category_of(name: &str, fast: &str, coder: &str, reasoner: &str) -> ModelCategory {
    if name == reasoner && name != coder {
        ModelCategory::Reasoner
    } else if name == fast && name != coder {
        ModelCategory::Fast
    } else {
        ModelCategory::Coder
    }
}

fn ctx_for(kind: TaskKind, category: ModelCategory, requested: u32) -> u32 {
    let cap = match (kind, category) {
        (TaskKind::Question, _) | (TaskKind::Trivial, ModelCategory::Coder) => {
            Some(if kind == TaskKind::Question {
                FAST_CTX
            } else {
                TRIVIAL_CTX
            })
        }
        (TaskKind::Trivial, ModelCategory::Fast) => Some(FAST_CTX),
        _ => None,
    };
    match cap {
        Some(limit) => requested.min(limit),
        None => requested,
    }
}

fn kind_label(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::Question => "pergunta",
        TaskKind::Trivial => "trivial",
        TaskKind::Normal => "normal",
        TaskKind::Complex => "complexa",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(name: &str, size: u64) -> ModelSlot {
        ModelSlot {
            name: name.to_string(),
            size_bytes: size,
        }
    }

    fn input<'a>(
        kind: TaskKind,
        default: &'a str,
        assignment: &'a ModelAssignment,
        inventory: &'a ModelInventory,
        ctx: u32,
        failures: u32,
    ) -> RouteInput<'a> {
        RouteInput {
            kind,
            default_model: default,
            assignment,
            inventory,
            requested_ctx: ctx,
            coder_failures: failures,
        }
    }

    #[test]
    fn a_single_model_serves_every_category() {
        let assignment = ModelAssignment::default();
        let inventory = ModelInventory::default();
        let question = route(&input(
            TaskKind::Question,
            "modelo-x",
            &assignment,
            &inventory,
            16_384,
            0,
        ));
        let trivial = route(&input(
            TaskKind::Trivial,
            "modelo-x",
            &assignment,
            &inventory,
            16_384,
            0,
        ));
        let normal = route(&input(
            TaskKind::Normal,
            "modelo-x",
            &assignment,
            &inventory,
            16_384,
            0,
        ));
        assert_eq!(question.model, "modelo-x");
        assert_eq!(trivial.model, "modelo-x");
        assert_eq!(normal.model, "modelo-x");
        assert_eq!(question.category, ModelCategory::Coder);
        assert_eq!(trivial.category, ModelCategory::Coder);
        assert_eq!(normal.category, ModelCategory::Coder);
        assert!(question.reason.contains("só um modelo"));
        assert_eq!(question.num_ctx, FAST_CTX);
        assert_eq!(trivial.num_ctx, TRIVIAL_CTX);
        assert_eq!(normal.num_ctx, 16_384);
    }

    #[test]
    fn a_question_uses_fast_when_it_is_distinct_and_free() {
        let assignment = ModelAssignment {
            fast: Some("fast-x".into()),
            coder: Some("coder-x".into()),
            reasoner: Some("reasoner-x".into()),
        };
        let inventory = ModelInventory {
            available: vec![
                slot("fast-x", 2_000),
                slot("coder-x", 20_000),
                slot("reasoner-x", 20_000),
            ],
            ..Default::default()
        };
        let decision = route(&input(
            TaskKind::Question,
            "coder-x",
            &assignment,
            &inventory,
            16_384,
            0,
        ));
        assert_eq!(decision.model, "fast-x");
        assert_eq!(decision.category, ModelCategory::Fast);
        assert_eq!(decision.num_ctx, FAST_CTX);
        assert!(decision.reason.contains("FAST"));
    }

    #[test]
    fn fast_does_not_edit_when_coder_is_distinct() {
        let assignment = ModelAssignment {
            fast: Some("fast-x".into()),
            coder: Some("coder-x".into()),
            reasoner: None,
        };
        let inventory = ModelInventory {
            available: vec![slot("fast-x", 2_000), slot("coder-x", 20_000)],
            ..Default::default()
        };
        for kind in [TaskKind::Trivial, TaskKind::Normal, TaskKind::Complex] {
            let decision = route(&input(kind, "coder-x", &assignment, &inventory, 16_384, 0));
            assert_eq!(decision.model, "coder-x", "{kind:?}");
            assert_eq!(decision.category, ModelCategory::Coder, "{kind:?}");
        }
        let trivial = route(&input(
            TaskKind::Trivial,
            "coder-x",
            &assignment,
            &inventory,
            16_384,
            0,
        ));
        assert_eq!(trivial.num_ctx, TRIVIAL_CTX);
        assert!(trivial.reason.contains("FAST não edita"));
    }

    #[test]
    fn a_loaded_coder_is_kept_for_a_question() {
        let assignment = ModelAssignment {
            fast: Some("fast-x".into()),
            coder: Some("coder-x".into()),
            reasoner: None,
        };
        let inventory = ModelInventory {
            available: vec![slot("fast-x", 2_000), slot("coder-x", 20_000)],
            loaded: vec![slot("coder-x", 20_000)],
            mem_available_bytes: Some(600_000_000),
        };
        let decision = route(&input(
            TaskKind::Question,
            "coder-x",
            &assignment,
            &inventory,
            16_384,
            0,
        ));
        assert_eq!(decision.model, "coder-x");
        assert_eq!(decision.category, ModelCategory::Coder);
        assert!(decision.reason.contains("já carregado"));
    }

    #[test]
    fn a_loaded_fast_is_unloaded_when_the_task_edits() {
        let assignment = ModelAssignment {
            fast: Some("fast-x".into()),
            coder: Some("coder-x".into()),
            reasoner: None,
        };
        let inventory = ModelInventory {
            available: vec![slot("fast-x", 2_000), slot("coder-x", 20_000)],
            loaded: vec![slot("fast-x", 2_000)],
            mem_available_bytes: Some(30_000_000_000),
        };
        let decision = route(&input(
            TaskKind::Trivial,
            "coder-x",
            &assignment,
            &inventory,
            16_384,
            0,
        ));
        assert_eq!(decision.model, "coder-x");
        assert!(decision.reason.contains("troca para CODER"));
    }

    #[test]
    fn a_missing_configured_model_falls_back_to_the_task_model() {
        let assignment = ModelAssignment {
            fast: Some("ghost".into()),
            coder: Some("ghost".into()),
            reasoner: Some("ghost".into()),
        };
        let inventory = ModelInventory {
            available: vec![slot("modelo-x", 5_000)],
            ..Default::default()
        };
        let decision = route(&input(
            TaskKind::Normal,
            "modelo-x",
            &assignment,
            &inventory,
            16_384,
            0,
        ));
        assert_eq!(decision.model, "modelo-x");
        assert_eq!(decision.category, ModelCategory::Coder);
    }

    #[test]
    fn a_model_that_does_not_fit_keeps_the_resident() {
        let assignment = ModelAssignment {
            fast: Some("fast-x".into()),
            coder: Some("coder-x".into()),
            reasoner: Some("reasoner-x".into()),
        };
        let inventory = ModelInventory {
            available: vec![
                slot("fast-x", 2_000),
                slot("coder-x", 20_000_000_000),
                slot("reasoner-x", 20_000_000_000),
            ],
            loaded: vec![slot("fast-x", 2_000)],
            mem_available_bytes: Some(1_000),
        };
        // Question: FAST is loaded and fits. Asking to escalate would want REASONER, but
        // quality failures are zero here — stay on FAST.
        let decision = route(&input(
            TaskKind::Question,
            "coder-x",
            &assignment,
            &inventory,
            8_192,
            0,
        ));
        assert_eq!(decision.model, "fast-x");
    }

    #[test]
    fn two_coder_failures_escalate_to_reasoner() {
        let assignment = ModelAssignment {
            fast: Some("fast-x".into()),
            coder: Some("coder-x".into()),
            reasoner: Some("reasoner-x".into()),
        };
        let inventory = ModelInventory {
            available: vec![
                slot("fast-x", 2_000),
                slot("coder-x", 20_000),
                slot("reasoner-x", 20_000),
            ],
            loaded: vec![slot("coder-x", 20_000)],
            mem_available_bytes: Some(30_000_000_000),
        };
        let before = route(&input(
            TaskKind::Normal,
            "coder-x",
            &assignment,
            &inventory,
            16_384,
            1,
        ));
        assert_eq!(before.model, "coder-x");
        let after = route(&input(
            TaskKind::Normal,
            "coder-x",
            &assignment,
            &inventory,
            16_384,
            2,
        ));
        assert_eq!(after.model, "reasoner-x");
        assert_eq!(after.category, ModelCategory::Reasoner);
        assert_eq!(after.num_ctx, 16_384);
        assert!(after.reason.contains("escalonamento"));
    }

    #[test]
    fn same_reasoner_name_does_not_escalate() {
        let assignment = ModelAssignment::default();
        let inventory = ModelInventory::default();
        let decision = route(&input(
            TaskKind::Normal,
            "modelo-x",
            &assignment,
            &inventory,
            16_384,
            5,
        ));
        assert_eq!(decision.model, "modelo-x");
        assert_eq!(decision.category, ModelCategory::Coder);
        assert!(!decision.reason.contains("escalonamento"));
    }

    #[test]
    fn trivial_and_question_cost_less_than_normal() {
        let assignment = ModelAssignment {
            fast: Some("fast-x".into()),
            coder: Some("coder-x".into()),
            reasoner: Some("reasoner-x".into()),
        };
        let inventory = ModelInventory {
            available: vec![slot("fast-x", 1), slot("coder-x", 1), slot("reasoner-x", 1)],
            ..Default::default()
        };
        let question = route(&input(
            TaskKind::Question,
            "coder-x",
            &assignment,
            &inventory,
            16_384,
            0,
        ));
        let trivial = route(&input(
            TaskKind::Trivial,
            "coder-x",
            &assignment,
            &inventory,
            16_384,
            0,
        ));
        let normal = route(&input(
            TaskKind::Normal,
            "coder-x",
            &assignment,
            &inventory,
            16_384,
            0,
        ));
        assert!(
            question.expected_prefill_cost() < trivial.expected_prefill_cost(),
            "question {} vs trivial {}",
            question.expected_prefill_cost(),
            trivial.expected_prefill_cost()
        );
        assert!(
            trivial.expected_prefill_cost() < normal.expected_prefill_cost(),
            "trivial {} vs normal {}",
            trivial.expected_prefill_cost(),
            normal.expected_prefill_cost()
        );
        // 8k FAST vs 16k CODER: 8192 vs 49152.
        assert_eq!(question.expected_prefill_cost(), u64::from(FAST_CTX));
        assert_eq!(
            trivial.expected_prefill_cost(),
            u64::from(TRIVIAL_CTX) * WEIGHT_CODER
        );
        assert_eq!(normal.expected_prefill_cost(), 16_384 * WEIGHT_CODER);
    }

    #[test]
    fn routing_is_stable() {
        let assignment = ModelAssignment {
            fast: Some("fast-x".into()),
            coder: Some("coder-x".into()),
            reasoner: Some("reasoner-x".into()),
        };
        let inventory = ModelInventory {
            available: vec![slot("fast-x", 1), slot("coder-x", 1), slot("reasoner-x", 1)],
            loaded: vec![slot("coder-x", 1)],
            ..Default::default()
        };
        let first = route(&input(
            TaskKind::Trivial,
            "coder-x",
            &assignment,
            &inventory,
            16_384,
            0,
        ));
        let second = route(&input(
            TaskKind::Trivial,
            "coder-x",
            &assignment,
            &inventory,
            16_384,
            0,
        ));
        assert_eq!(first, second);
    }

    #[test]
    fn mem_available_parses_proc_meminfo() {
        let sample = "MemTotal:        32654832 kB\nMemFree:           1234 kB\nMemAvailable:    8000000 kB\n";
        let bytes = mem_available_from(|_| Some(sample.to_string()));
        #[cfg(target_os = "linux")]
        {
            assert_eq!(bytes, Some(8000000 * 1024));
        }
        #[cfg(not(target_os = "linux"))]
        {
            assert_eq!(bytes, None);
        }
    }
}
