use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::sandbox::SandboxCapabilities;

/// Deterministic classification of a command line (design §4), pure function.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum CommandClass {
    Read,
    Validate,
    Write,
    Network,
    Destructive,
    Unknown,
}

/// How a permission decision came about (design §5.3: `denied`/`auto`/`granted`).
///
/// `Deserialize` too, so a stored `ToolEvent` can be read back for a typed replay
/// (plan 015, Part D, step 0).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum PermissionDecision {
    Denied,
    Auto,
    Granted,
}

/// User-facing permission mode (SPEC §20.4). Default is Ask: the v1 behaviour before a sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    #[default]
    Ask,
    Auto,
    FullAccess,
}

/// What the policy is deciding on. Paths outside the workspace never reach this: they die in
/// `Workspace::resolve` first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionKind {
    ReadFile { secret: bool },
    EditFile,
    WriteFile,
    RunCommand { class: CommandClass },
}

/// Pure outcome of [`policy`]: execute, ask the user, or refuse without asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    Auto,
    Ask,
    Deny,
}

/// SPEC §20.4. Full Access without a complete sandbox is treated as Ask (D5).
pub fn effective_mode(mode: PermissionMode, caps: SandboxCapabilities) -> PermissionMode {
    match mode {
        PermissionMode::FullAccess if !caps.ready() => PermissionMode::Ask,
        other => other,
    }
}

/// Deterministic permission table. Never reads model text or tool output (SPEC §20.5).
pub fn policy(mode: PermissionMode, kind: &PermissionKind, caps: SandboxCapabilities) -> Policy {
    let mode = effective_mode(mode, caps);
    match kind {
        PermissionKind::ReadFile { secret: false } => Policy::Auto,
        PermissionKind::ReadFile { secret: true } => Policy::Ask,
        PermissionKind::EditFile | PermissionKind::WriteFile => match mode {
            PermissionMode::Ask => Policy::Ask,
            PermissionMode::Auto | PermissionMode::FullAccess => Policy::Auto,
        },
        PermissionKind::RunCommand { class } => match class {
            CommandClass::Read | CommandClass::Validate => Policy::Auto,
            CommandClass::Write => match mode {
                PermissionMode::Ask => Policy::Ask,
                PermissionMode::Auto | PermissionMode::FullAccess if caps.filesystem => {
                    Policy::Auto
                }
                PermissionMode::Auto | PermissionMode::FullAccess => Policy::Ask,
            },
            CommandClass::Network | CommandClass::Destructive | CommandClass::Unknown => {
                Policy::Ask
            }
        },
    }
}

impl PermissionMode {
    /// CLI / config spelling: `ask`, `auto`, `full-access`.
    pub fn parse_slug(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "ask" => Some(Self::Ask),
            "auto" => Some(Self::Auto),
            "full-access" | "fullaccess" | "full_access" => Some(Self::FullAccess),
            _ => None,
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Auto => "auto",
            Self::FullAccess => "full-access",
        }
    }
}

/// The exact thing the user is asked to approve (design §5.3). Never a model summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ApprovalAction {
    RunCommand {
        argv: Vec<String>,
        class: CommandClass,
        cwd: String,
    },
    EditFile {
        path: String,
        diff: String,
    },
    WriteFile {
        path: String,
        #[ts(type = "number")]
        size: u64,
    },
    ReadFile {
        path: String,
    },
}

/// What the engine needs back from the responder (design §5.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalResponse {
    Granted,
    Denied { reason: Option<String> },
}

/// A single open approval, with an idempotent id (`aprv_NNNN`, design §5.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub id: String,
    pub task_id: String,
    pub at: String,
    pub action: ApprovalAction,
}

/// Holds the open approvals for a task (design §5.4). Identical pending actions reuse one id.
#[derive(Debug, Default)]
pub struct PermissionManager {
    pending: HashMap<String, ApprovalRequest>,
    next_id: u64,
}

fn timestamp() -> String {
    // RFC 3339 (UTC). Implemented with std only; two-digit fields, no sub-second precision needed.
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    let days = (secs / 86_400) as i64;
    let (year, month, day) = civil_from_days(days);
    let hour = secs / 3600 % 24;
    let minute = secs / 60 % 60;
    let second = secs % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Civil calendar date from a Unix day number (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    (year + if month <= 2 { 1 } else { 0 }, month, day)
}

impl PermissionManager {
    /// Returns the pending request for `action` — an existing identical one (same id) or a new one.
    pub fn pending_for(&mut self, task_id: &str, action: ApprovalAction) -> ApprovalRequest {
        if let Some(existing) = self
            .pending
            .values()
            .find(|req| req.action == action && req.task_id == task_id)
        {
            return existing.clone();
        }
        self.next_id += 1;
        let request = ApprovalRequest {
            id: format!("aprv_{:04}", self.next_id),
            task_id: task_id.to_string(),
            at: timestamp(),
            action,
        };
        self.pending.insert(request.id.clone(), request.clone());
        request
    }

    /// Resolves an open approval; returns the removed request, if any.
    pub fn resolve(&mut self, id: &str) -> Option<ApprovalRequest> {
        self.pending.remove(id)
    }

    /// Clears every open approval (task cancelled or finished, design §5.3).
    pub fn clear(&mut self) -> Vec<ApprovalRequest> {
        self.pending.drain().map(|(_, req)| req).collect()
    }

    pub fn pending_ids(&self) -> impl Iterator<Item = &String> {
        self.pending.keys()
    }
}

/// Maps argv to a command class using the basename of argv[0] plus flags (design §4).
pub fn classify(argv: &[String]) -> CommandClass {
    let Some(program) = argv.first() else {
        return CommandClass::Unknown;
    };
    // Basename only: `grep | xargs rm` must never become a `read`.
    let basename = program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .to_lowercase();
    let flags = argv
        .iter()
        .filter(|token| token.starts_with('-'))
        .map(String::as_str);

    let compound = argv.iter().any(|token| has_shell_metachar(token));

    if compound {
        // Tabela §4: composto assume a classe mais perigosa que o mesmo argv sugere; a Fase 4
        // recusa compostos com CompoundCommand de qualquer forma (D2).
        return most_dangerous(argv);
    }

    match basename.as_str() {
        "ls" | "cat" | "head" | "tail" | "less" | "grep" | "rg" | "find" | "wc" | "file"
        | "stat" | "which" | "echo" => CommandClass::Read,
        "git" => classify_git(argv),
        "npm" | "yarn" | "pnpm" | "bun" => classify_package_tagged(argv, flags),
        "cargo" => classify_cargo(argv, flags),
        "biome" => classify_biome(argv, flags),
        "curl" | "wget" | "ssh" | "ping" | "git-lfs" => CommandClass::Network,
        "pip" | "uv" => CommandClass::Network,
        "python" | "python3" => classify_python(argv, flags),
        "rm" => classify_rm(flags),
        "tf" | "tofu" | "aws" | "kubectl" => CommandClass::Network,
        "touch" | "mkdir" | "cp" | "mv" | "tee" | "sed" | "awk" => CommandClass::Write,
        "sh" | "bash" | "zsh" | "dash" => CommandClass::Unknown,
        _ => {
            if is_validate_candidate(&basename, argv, flags) {
                CommandClass::Validate
            } else {
                CommandClass::Unknown
            }
        }
    }
}

fn most_dangerous(argv: &[String]) -> CommandClass {
    // Reclassify without the compound flag: destructive > network > write > validate > read > unknown.
    // Starts at `unknown` (§20.2 "na dúvida, unknown"): a compound whose parts are all unclassified
    // must stay unknown, because `read`/`validate` is the one class that runs without approval.
    let mut worst = CommandClass::Unknown;
    for token in argv {
        if has_shell_metachar(token) {
            continue;
        }
        let class = classify(std::slice::from_ref(token));
        if danger_rank(&class) > danger_rank(&worst) {
            worst = class;
        }
    }
    worst
}

fn danger_rank(class: &CommandClass) -> u8 {
    match class {
        CommandClass::Destructive => 5,
        CommandClass::Network => 4,
        CommandClass::Write => 3,
        CommandClass::Validate => 2,
        CommandClass::Read => 1,
        CommandClass::Unknown => 0,
    }
}

pub fn has_shell_metachar(token: &str) -> bool {
    token.chars().any(|c| {
        matches!(
            c,
            '|' | '&' | ';' | '<' | '>' | '$' | '`' | '\n' | '\r' | '\0'
        )
    })
}

fn classify_git(argv: &[String]) -> CommandClass {
    let subcommand = argv.get(1).map(String::as_str).unwrap_or("");
    // Flags resolved before matching so guards never consume the iterator mid-match.
    let has_flag = |flag: &str| argv.iter().any(|token| token == flag);
    let has_short_flag = |chars: &str| {
        argv.iter().any(|token| {
            token.starts_with('-')
                && !token.starts_with("--")
                && token.chars().skip(1).any(|c| chars.contains(c))
        })
    };
    match subcommand {
        "status" | "diff" | "log" | "branch" | "show" | "blame" | "remote" => CommandClass::Read,
        "add" | "commit" | "mv" | "restore" => CommandClass::Write,
        "checkout" if has_flag("--") => CommandClass::Destructive,
        "checkout" if has_flag("-b") => CommandClass::Write,
        "checkout" => CommandClass::Write,
        "reset" if has_flag("--hard") => CommandClass::Destructive,
        "reset" => CommandClass::Write,
        "clean" if has_short_flag("f") => CommandClass::Destructive,
        "clean" => CommandClass::Write,
        "fetch" | "pull" | "push" | "clone" => CommandClass::Network,
        _ => CommandClass::Unknown,
    }
}

fn classify_package_tagged<'a>(
    argv: &[String],
    flags: impl Iterator<Item = &'a str>,
) -> CommandClass {
    let subcommand = argv.get(1).map(String::as_str).unwrap_or("");
    let _ = flags; // npm run <script> is ruled by the script name below.
    match subcommand {
        // Network wins over the script table: `bun add test` still downloads a package.
        "install" | "add" | "update" | "remove" | "uninstall" => CommandClass::Network,
        "run" => {
            let script = argv.get(2).map(String::as_str).unwrap_or("");
            if is_validate_script(script) {
                CommandClass::Validate
            } else {
                CommandClass::Unknown
            }
        }
        // `bun test` is the short form of `bun run test`, so it goes through the same table.
        script if is_validate_script(script) => CommandClass::Validate,
        _ => CommandClass::Unknown,
    }
}

fn classify_cargo<'a>(argv: &[String], flags: impl Iterator<Item = &'a str>) -> CommandClass {
    let subcommand = argv.get(1).map(String::as_str).unwrap_or("");
    let _ = flags;
    match subcommand {
        "test" | "check" | "build" | "clippy" | "fmt" | "doc" => CommandClass::Validate,
        "add" | "install" | "publish" | "login" => CommandClass::Network,
        "clean" | "update" => CommandClass::Network,
        "run" => CommandClass::Unknown,
        _ => CommandClass::Unknown,
    }
}

fn classify_biome<'a>(argv: &[String], mut flags: impl Iterator<Item = &'a str>) -> CommandClass {
    if argv
        .iter()
        .skip(1)
        .take_while(|token| !token.starts_with('-'))
        .any(|token| token == "check" || token == "lint")
    {
        CommandClass::Validate
    } else if flags.any(|f| f == "--write") {
        CommandClass::Write
    } else {
        CommandClass::Unknown
    }
}

fn classify_python<'a>(argv: &[String], flags: impl Iterator<Item = &'a str>) -> CommandClass {
    let _ = flags;
    if argv.get(1).is_some_and(|script| {
        script.ends_with("test") || script.contains("pytest") || script.contains("manage.py")
    }) {
        return CommandClass::Validate;
    }
    if argv.get(1).map(String::as_str) == Some("-m") {
        let module = argv.get(2).map(String::as_str).unwrap_or("");
        return if module.starts_with("pip") {
            CommandClass::Network
        } else {
            CommandClass::Unknown
        };
    }
    CommandClass::Unknown
}

fn classify_rm<'a>(mut flags: impl Iterator<Item = &'a str>) -> CommandClass {
    // `rm` sem recursão é um arquivo único → write; `rm -r*` → destructive (design §4).
    let recursive = flags.any(|flag| {
        !flag.starts_with("--")
            && flag.starts_with('-')
            && flag.chars().skip(1).any(|c| c == 'r' || c == 'R')
    });
    if recursive {
        CommandClass::Destructive
    } else {
        CommandClass::Write
    }
}

/// Prefix rules for workspace validation commands, cached per workspace in Fase 5 (design §4).
fn is_validate_script(script: &str) -> bool {
    let lower = script.to_lowercase();
    ["test", "check", "typecheck", "build", "lint", "verify"]
        .iter()
        .any(|prefix| lower == *prefix || lower.starts_with(&format!("{prefix}:")))
}

fn is_validate_candidate<'a>(
    basename: &str,
    argv: &[String],
    _flags: impl Iterator<Item = &'a str>,
) -> bool {
    match basename {
        // Known workspace validators that don't live in the tagged table.
        "pytest" | "nosetests" | "go" => argv.get(1).map(String::as_str) == Some("test"),
        "make" | "just" => argv
            .get(1)
            .map(String::as_str)
            .map(is_validate_script)
            .unwrap_or(false),
        "dune" => argv.get(1).map(String::as_str) == Some("build"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn read_commands() {
        assert_eq!(classify(&cv(&["ls", "-la"])), CommandClass::Read);
        assert_eq!(classify(&cv(&["cat", "src/main.rs"])), CommandClass::Read);
        assert_eq!(
            classify(&cv(&["rg", "--fixed-strings", "TODO", "src"])),
            CommandClass::Read
        );
        assert_eq!(classify(&cv(&["git", "status"])), CommandClass::Read);
        assert_eq!(classify(&cv(&["git", "diff"])), CommandClass::Read);
        assert_eq!(
            classify(&cv(&["git", "log", "--oneline"])),
            CommandClass::Read
        );
    }

    #[test]
    fn validate_commands() {
        assert_eq!(
            classify(&cv(&["cargo", "test", "--workspace"])),
            CommandClass::Validate
        );
        assert_eq!(classify(&cv(&["cargo", "check"])), CommandClass::Validate);
        assert_eq!(
            classify(&cv(&["bun", "run", "verify"])),
            CommandClass::Validate
        );
        assert_eq!(
            classify(&cv(&["npm", "run", "test"])),
            CommandClass::Validate
        );
        assert_eq!(
            classify(&cv(&["biome", "check", "."])),
            CommandClass::Validate
        );
    }

    #[test]
    fn package_short_form_matches_run_script() {
        // `bun test` is `bun run test`; the fixture `evals/fixtures/soma` uses exactly this form.
        for tool in ["bun", "npm", "yarn", "pnpm"] {
            assert_eq!(
                classify(&cv(&[tool, "test"])),
                CommandClass::Validate,
                "{tool} test"
            );
            assert_eq!(
                classify(&cv(&[tool, "run", "test"])),
                CommandClass::Validate,
                "{tool} run test"
            );
        }
        assert_eq!(
            classify(&cv(&["bun", "test", "--coverage"])),
            CommandClass::Validate
        );
        assert_eq!(classify(&cv(&["bun", "lint"])), CommandClass::Validate);
        assert_eq!(
            classify(&cv(&["npm", "run", "verify"])),
            CommandClass::Validate
        );
        assert_eq!(
            classify(&cv(&["bun", "run", "verify"])),
            CommandClass::Validate
        );
    }

    #[test]
    fn package_network_subcommands_beat_the_script_table() {
        assert_eq!(classify(&cv(&["bun", "install"])), CommandClass::Network);
        assert_eq!(classify(&cv(&["pnpm", "install"])), CommandClass::Network);
        // `add`/`remove` are network even though a script could carry the same name.
        assert_eq!(
            classify(&cv(&["bun", "add", "test"])),
            CommandClass::Network
        );
        assert_eq!(
            classify(&cv(&["npm", "uninstall", "left-pad"])),
            CommandClass::Network
        );
    }

    #[test]
    fn package_non_validate_subcommands_stay_unknown() {
        assert_eq!(
            classify(&cv(&["bun", "run", "deploy"])),
            CommandClass::Unknown
        );
        assert_eq!(classify(&cv(&["bun", "publish"])), CommandClass::Unknown);
        assert_eq!(classify(&cv(&["bun"])), CommandClass::Unknown);
        assert_eq!(classify(&cv(&["bun", "run"])), CommandClass::Unknown);
        // `bunx`/`npx` fetch a package before running it: not part of the validate table.
        assert_eq!(classify(&cv(&["bunx", "test"])), CommandClass::Unknown);
        assert_eq!(classify(&cv(&["npx", "test"])), CommandClass::Unknown);
    }

    #[test]
    fn write_commands() {
        assert_eq!(classify(&cv(&["touch", "novo.txt"])), CommandClass::Write);
        assert_eq!(
            classify(&cv(&["mkdir", "-p", "src/x"])),
            CommandClass::Write
        );
        assert_eq!(classify(&cv(&["git", "add", "."])), CommandClass::Write);
        assert_eq!(
            classify(&cv(&["git", "commit", "-m", "x"])),
            CommandClass::Write
        );
        assert_eq!(
            classify(&cv(&["git", "checkout", "-b", "feat"])),
            CommandClass::Write
        );
        assert_eq!(classify(&cv(&["rm", "arquivo.txt"])), CommandClass::Write);
        assert_eq!(
            classify(&cv(&["git", "reset", "HEAD~1"])),
            CommandClass::Write
        );
    }

    #[test]
    fn network_commands() {
        assert_eq!(
            classify(&cv(&["curl", "-s", "https://x.com"])),
            CommandClass::Network
        );
        assert_eq!(classify(&cv(&["git", "pull"])), CommandClass::Network);
        assert_eq!(classify(&cv(&["git", "fetch"])), CommandClass::Network);
        assert_eq!(classify(&cv(&["npm", "install"])), CommandClass::Network);
        assert_eq!(
            classify(&cv(&["cargo", "add", "serde"])),
            CommandClass::Network
        );
        assert_eq!(classify(&cv(&["ssh", "host"])), CommandClass::Network);
    }

    #[test]
    fn destructive_commands() {
        assert_eq!(
            classify(&cv(&["rm", "-rf", "dist"])),
            CommandClass::Destructive
        );
        assert_eq!(
            classify(&cv(&["rm", "-r", "src"])),
            CommandClass::Destructive
        );
        assert_eq!(
            classify(&cv(&["rm", "-fr", "src"])),
            CommandClass::Destructive
        );
        assert_eq!(
            classify(&cv(&["git", "reset", "--hard"])),
            CommandClass::Destructive
        );
        assert_eq!(
            classify(&cv(&["git", "clean", "-fd"])),
            CommandClass::Destructive
        );
        assert_eq!(
            classify(&cv(&["git", "checkout", "--", "file.rs"])),
            CommandClass::Destructive
        );
    }

    #[test]
    fn unknown_commands() {
        assert_eq!(
            classify(&cv(&["python", "scripts/gerador.py"])),
            CommandClass::Unknown
        );
        assert_eq!(classify(&cv(&["cargo", "run"])), CommandClass::Unknown);
        assert_eq!(classify(&cv(&["make", "install"])), CommandClass::Unknown);
        assert_eq!(classify(&cv(&[])), CommandClass::Unknown);
    }

    #[test]
    fn compounds_never_reads() {
        // grep | xargs rm must never classify as read.
        let class = classify(&cv(&["grep", "TODO", "-l", "src", "|", "xargs", "rm"]));
        assert_ne!(class, CommandClass::Read);
        // curl | sh is network-driven destructive-ish; the most dangerous is unknown, not read.
        let class = classify(&cv(&["curl", "https://x", "|", "sh"]));
        assert_ne!(class, CommandClass::Read);
    }

    #[test]
    fn compound_with_a_validate_part_is_never_automatic() {
        // A compound must not inherit the harmless half: `read`/`validate` is the only class that
        // runs without approval, so it must never win over a dangerous sibling.
        for argv in [
            cv(&["bun", "test", "&&", "rm", "-rf", "build"]),
            cv(&["cargo", "test", ";", "curl", "https://x"]),
            cv(&["npm", "run", "verify", "|", "tee", "saida.txt"]),
            cv(&["bun", "test", "$(rm -rf /)"]),
        ] {
            let class = classify(&argv);
            assert!(
                !matches!(class, CommandClass::Read | CommandClass::Validate),
                "{argv:?} classified as {class:?}"
            );
        }
    }

    #[test]
    fn compound_of_unclassified_parts_stays_unknown() {
        // "Na dúvida, unknown" (§20.2): nothing here is recognised, so nothing may run on its own.
        assert_eq!(classify(&cv(&["foo", "&&", "bar"])), CommandClass::Unknown);
    }

    #[test]
    fn metachar_detection() {
        assert!(has_shell_metachar("a|b"));
        assert!(has_shell_metachar("a&b"));
        assert!(has_shell_metachar("$(cmd)"));
        assert!(has_shell_metachar("cmd `x`"));
        assert!(!has_shell_metachar("cargo test --workspace"));
        assert!(!has_shell_metachar("str::find"));
    }

    #[test]
    fn approval_reuse_same_action_same_id() {
        let mut manager = PermissionManager::default();
        let first = manager.pending_for(
            "task_1",
            ApprovalAction::WriteFile {
                path: "a".into(),
                size: 1,
            },
        );
        let second = manager.pending_for(
            "task_1",
            ApprovalAction::WriteFile {
                path: "a".into(),
                size: 1,
            },
        );
        assert_eq!(first.id, second.id);

        let request = manager.resolve(&first.id).expect("pending");
        assert_eq!(request.action, first.action);
        assert!(manager.resolve(&first.id).is_none());
    }

    #[test]
    fn different_actions_distinct_ids() {
        let mut manager = PermissionManager::default();
        let a = manager.pending_for(
            "t",
            ApprovalAction::WriteFile {
                path: "a".into(),
                size: 1,
            },
        );
        let b = manager.pending_for(
            "t",
            ApprovalAction::WriteFile {
                path: "b".into(),
                size: 1,
            },
        );
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn clear_drops_all_pending() {
        let mut manager = PermissionManager::default();
        manager.pending_for(
            "t",
            ApprovalAction::WriteFile {
                path: "a".into(),
                size: 1,
            },
        );
        manager.pending_for(
            "t",
            ApprovalAction::RunCommand {
                argv: cv(&["rm", "-rf", "x"]),
                class: CommandClass::Destructive,
                cwd: "/".into(),
            },
        );
        let cleared = manager.clear();
        assert_eq!(cleared.len(), 2);
        assert_eq!(manager.pending_ids().count(), 0);
    }

    fn caps(filesystem: bool, network_block: bool) -> SandboxCapabilities {
        SandboxCapabilities {
            filesystem,
            network_block,
        }
    }

    fn none() -> SandboxCapabilities {
        caps(false, false)
    }

    fn ready() -> SandboxCapabilities {
        caps(true, true)
    }

    #[test]
    fn ask_mode_matches_the_phase_4_table() {
        assert_eq!(
            policy(
                PermissionMode::Ask,
                &PermissionKind::ReadFile { secret: false },
                none()
            ),
            Policy::Auto
        );
        assert_eq!(
            policy(
                PermissionMode::Ask,
                &PermissionKind::ReadFile { secret: true },
                none()
            ),
            Policy::Ask
        );
        assert_eq!(
            policy(PermissionMode::Ask, &PermissionKind::EditFile, none()),
            Policy::Ask
        );
        assert_eq!(
            policy(PermissionMode::Ask, &PermissionKind::WriteFile, none()),
            Policy::Ask
        );
        assert_eq!(
            policy(
                PermissionMode::Ask,
                &PermissionKind::RunCommand {
                    class: CommandClass::Read
                },
                none()
            ),
            Policy::Auto
        );
        assert_eq!(
            policy(
                PermissionMode::Ask,
                &PermissionKind::RunCommand {
                    class: CommandClass::Validate
                },
                none()
            ),
            Policy::Auto
        );
        for class in [
            CommandClass::Write,
            CommandClass::Network,
            CommandClass::Destructive,
            CommandClass::Unknown,
        ] {
            assert_eq!(
                policy(
                    PermissionMode::Ask,
                    &PermissionKind::RunCommand {
                        class: class.clone()
                    },
                    ready()
                ),
                Policy::Ask,
                "{class:?}"
            );
        }
    }

    #[test]
    fn auto_mode_edits_without_asking_and_write_commands_need_a_filesystem_sandbox() {
        assert_eq!(
            policy(PermissionMode::Auto, &PermissionKind::EditFile, none()),
            Policy::Auto
        );
        assert_eq!(
            policy(PermissionMode::Auto, &PermissionKind::WriteFile, none()),
            Policy::Auto
        );
        assert_eq!(
            policy(
                PermissionMode::Auto,
                &PermissionKind::RunCommand {
                    class: CommandClass::Write
                },
                caps(true, false)
            ),
            Policy::Auto
        );
        assert_eq!(
            policy(
                PermissionMode::Auto,
                &PermissionKind::RunCommand {
                    class: CommandClass::Write
                },
                none()
            ),
            Policy::Ask
        );
        for class in [
            CommandClass::Network,
            CommandClass::Destructive,
            CommandClass::Unknown,
        ] {
            assert_eq!(
                policy(
                    PermissionMode::Auto,
                    &PermissionKind::RunCommand {
                        class: class.clone()
                    },
                    ready()
                ),
                Policy::Ask,
                "{class:?}"
            );
        }
        assert_eq!(
            policy(
                PermissionMode::Auto,
                &PermissionKind::ReadFile { secret: true },
                ready()
            ),
            Policy::Ask
        );
    }

    #[test]
    fn full_access_without_a_complete_sandbox_behaves_like_ask() {
        assert_eq!(
            effective_mode(PermissionMode::FullAccess, none()),
            PermissionMode::Ask
        );
        assert_eq!(
            effective_mode(PermissionMode::FullAccess, caps(true, false)),
            PermissionMode::Ask
        );
        assert_eq!(
            effective_mode(PermissionMode::FullAccess, ready()),
            PermissionMode::FullAccess
        );
        assert_eq!(
            policy(
                PermissionMode::FullAccess,
                &PermissionKind::EditFile,
                none()
            ),
            Policy::Ask
        );
        assert_eq!(
            policy(
                PermissionMode::FullAccess,
                &PermissionKind::EditFile,
                ready()
            ),
            Policy::Auto
        );
        assert_eq!(
            policy(
                PermissionMode::FullAccess,
                &PermissionKind::RunCommand {
                    class: CommandClass::Write
                },
                ready()
            ),
            Policy::Auto
        );
        assert_eq!(
            policy(
                PermissionMode::FullAccess,
                &PermissionKind::RunCommand {
                    class: CommandClass::Network
                },
                ready()
            ),
            Policy::Ask
        );
    }

    #[test]
    fn permission_mode_slug_round_trips() {
        for mode in [
            PermissionMode::Ask,
            PermissionMode::Auto,
            PermissionMode::FullAccess,
        ] {
            assert_eq!(PermissionMode::parse_slug(mode.slug()), Some(mode));
        }
        assert_eq!(
            PermissionMode::parse_slug("full-access"),
            Some(PermissionMode::FullAccess)
        );
        assert_eq!(PermissionMode::parse_slug("nope"), None);
    }
}
