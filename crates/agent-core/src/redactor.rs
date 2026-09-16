use std::path::Path;

use serde::Serialize;
use ts_rs::TS;

/// What kind of secret a path or content looks like (design §6.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum SecretKind {
    #[serde(rename = ".env")]
    DotEnv,
    #[serde(rename = "chavePrivada")]
    PrivateKey,
    #[serde(rename = "credentials")]
    Credentials,
}

/// A run of bytes in `text` that must not reach the model (design §6.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretSpan {
    pub start: usize,
    pub end: usize,
    pub kind: SecretKind,
}

/// Text redacted for model consumption (design §6.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactedText {
    pub text: String,
    pub spans: Vec<SecretSpan>,
}

impl RedactedText {
    pub fn count(&self) -> usize {
        self.spans.len()
    }
}

const ENTROPY_WINDOW: usize = 32;
const ENTROPY_LIMIT: f64 = 4.7;
const ENTROPY_RUN: usize = 2;
/// Windows with fewer distinct bytes cannot exceed [`ENTROPY_LIMIT`]:
/// max H = log2(k) and log2(25) ≈ 4.64 < 4.7.
const ENTROPY_MIN_UNIQUE: u32 = 26;

/// Detects a secret by file path (design §6.1). Never touches the content.
pub fn detect_path_secret(path: &Path) -> Option<SecretKind> {
    let file_name = path.file_name()?.to_string_lossy();
    let lower_name = file_name.to_lowercase();

    if lower_name == ".env" || lower_name.ends_with(".env") || lower_name.contains(".env.") {
        return Some(SecretKind::DotEnv);
    }
    if lower_name.starts_with("id_")
        && (lower_name.ends_with("rsa")
            || lower_name.ends_with("ed25519")
            || lower_name.ends_with("ecdsa"))
    {
        return Some(SecretKind::PrivateKey);
    }
    if lower_name.contains("credential") || lower_name.contains("secret") {
        return Some(SecretKind::Credentials);
    }
    if let Some(extension) = path.extension() {
        match extension.to_string_lossy().to_lowercase().as_str() {
            "pem" | "key" | "p12" | "pfx" | "cer" | "crt" => {
                return Some(SecretKind::PrivateKey);
            }
            "kdbx" => return Some(SecretKind::Credentials),
            _ => {}
        }
    }
    None
}

/// Recognizes secret spans inside arbitrary text (design §6.2). Returns spans sorted, non-overlapping.
pub fn recognize(text: &str) -> Vec<SecretSpan> {
    let mut spans: Vec<SecretSpan> = Vec::new();
    spans.extend(prefix_spans(text));
    spans.extend(pem_spans(text));
    spans.extend(jwt_spans(text));
    spans.extend(userinfo_spans(text));
    spans.extend(entropy_spans(text));

    spans.sort_by_key(|s| (s.start, s.end));
    spans.into_iter().fold(Vec::new(), |mut merged, span| {
        match merged.last_mut() {
            Some(prev) if span.start <= prev.end => {
                prev.end = prev.end.max(span.end);
                prev.kind = prev.kind.clone();
            }
            _ => merged.push(span),
        }
        merged
    })
}

/// Redacts every recognized secret, keeping a short fragment of the type (design §6.3).
pub fn redact(text: &str) -> RedactedText {
    let spans = recognize(text);
    if spans.is_empty() {
        return RedactedText {
            text: text.to_string(),
            spans,
        };
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for span in &spans {
        out.push_str(&text[cursor..span.start]);
        out.push_str(&format!("[REDIGIDO:{}]", fragment(&span.kind)));
        cursor = span.end;
    }
    out.push_str(&text[cursor..]);
    RedactedText { text: out, spans }
}

fn fragment(kind: &SecretKind) -> &'static str {
    match kind {
        SecretKind::DotEnv => ".env",
        SecretKind::PrivateKey => "chave",
        SecretKind::Credentials => "segredo",
    }
}

/// Rule 1: well-known token prefixes. Scans by byte slices, no regex.
fn prefix_spans(text: &str) -> Vec<SecretSpan> {
    const PREFIXES: &[(&str, SecretKind)] = &[
        ("sk_live_", SecretKind::Credentials),
        ("sk-", SecretKind::Credentials),
        ("ghp_", SecretKind::Credentials),
        ("gho_", SecretKind::Credentials),
        ("ghu_", SecretKind::Credentials),
        ("xoxb-", SecretKind::Credentials),
        ("xoxp-", SecretKind::Credentials),
        ("gsk_", SecretKind::Credentials),
        ("rk_live_", SecretKind::Credentials),
        ("AKIA", SecretKind::Credentials),
    ];
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    for (prefix, kind) in PREFIXES {
        let pb = prefix.as_bytes();
        let mut index = 0;
        while let Some(offset) = bytes[index..]
            .windows(pb.len())
            .position(|window| window == pb)
        {
            let start = index + offset;
            let end = start + token_run(&bytes[start + pb.len()..]) + pb.len();
            spans.push(SecretSpan {
                start,
                end,
                kind: kind.clone(),
            });
            index = end.max(start + 1);
        }
    }
    spans
}

/// Length of the alphanumeric tail that makes a prefix a real token (design: "seguido de base64").
fn token_run(bytes: &[u8]) -> usize {
    let mut len = 0;
    for byte in bytes {
        let alphanumeric = byte.is_ascii_alphanumeric();
        let underscore = *byte == b'_' || *byte == b'-';
        if !alphanumeric && !underscore {
            break;
        }
        len += 1;
    }
    len
}

/// Rule 2: PEM private-key blocks.
fn pem_spans(text: &str) -> Vec<SecretSpan> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut start = 0;
    while let Some(offset) = find_subslice(&bytes[start..], b"-----BEGIN") {
        let block_start = start + offset;
        let block_end = match find_subslice(&bytes[block_start..], b"-----END") {
            Some(e) => {
                block_start
                    + e
                    + b"-----END".len()
                    + pem_tail(&bytes[block_start + e + b"-----END".len()..])
            }
            None => break,
        };
        let block = &text[block_start..block_end.min(text.len())];
        if block.contains("PRIVATE KEY") {
            spans.push(SecretSpan {
                start: block_start,
                end: block_end,
                kind: SecretKind::PrivateKey,
            });
        }
        start = block_end.max(block_start + 1);
    }
    spans
}

fn pem_tail(bytes: &[u8]) -> usize {
    let mut len = 0;
    for byte in bytes {
        if *byte == b'\r' || *byte == b'\n' || *byte == b'-' {
            len += 1;
        } else {
            break;
        }
    }
    len
}

/// Rule 3: JWTs (three base64url segments, first starts with `eyJ` and payload decodes).
fn jwt_spans(text: &str) -> Vec<SecretSpan> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'e'
            || bytes.get(index + 1) != Some(&b'y')
            || bytes.get(index + 2) != Some(&b'J')
        {
            index += 1;
            continue;
        }
        let start = index;
        // Absolute positions within `bytes`.
        let Some(first_dot) = bytes[start..].iter().position(|b| *b == b'.') else {
            break;
        };
        let first_dot = start + first_dot;
        let Some(second_dot) = bytes[first_dot + 1..].iter().position(|b| *b == b'.') else {
            index = start + 1;
            continue;
        };
        let second_dot = first_dot + 1 + second_dot;
        let Some(third_dot) = bytes[second_dot + 1..]
            .iter()
            .position(|b| matches!(*b, b'.' | b' ' | b'\n' | b'\r'))
        else {
            index = start + 1;
            continue;
        };
        let end = second_dot + 1 + third_dot;
        let payload = &bytes[first_dot + 1..second_dot];
        if payload.len() > 1_000 || !payload.iter().all(|b| base64url_ok(*b)) {
            index = start + 1;
            continue;
        }
        // A decoded base64url payload contains mostly printable-ish bytes; require at least a JSON brace.
        match base64url_decode(payload) {
            Ok(decoded) if decoded.contains(&b'{') => {
                spans.push(SecretSpan {
                    start,
                    end,
                    kind: SecretKind::Credentials,
                });
                index = end;
                continue;
            }
            _ => {}
        }
        index = start + 1;
    }
    spans
}

fn base64url_ok(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
}

fn base64url_decode(slice: &[u8]) -> Result<Vec<u8>, ()> {
    if slice.is_empty() || slice.len() % 4 == 1 {
        return Err(());
    }
    let mut out = Vec::with_capacity(slice.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0;
    for byte in slice {
        let value = match byte {
            b'A'..=b'Z' => *byte - b'A',
            b'a'..=b'z' => *byte - b'a' + 26,
            b'0'..=b'9' => *byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return Err(()),
        };
        buffer = (buffer << 6) | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Ok(out)
}

/// Rule 4: URLs with userinfo (`scheme://user:pass@host`).
fn userinfo_spans(text: &str) -> Vec<SecretSpan> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes.get(index..index + 3) != Some(b"://") {
            index += 1;
            continue;
        }
        let at = bytes[index + 3..]
            .iter()
            .position(|b| *b == b'@')
            .map(|p| p + index + 3);
        let Some(at) = at else {
            index += 3;
            continue;
        };
        // The userinfo part must carry a ':' (a password) to be a secret.
        let userinfo = &bytes[index + 3..at];
        if userinfo
            .iter()
            .position(|b| *b == b':')
            .is_some_and(|colon| colon > 0 && colon + 1 < userinfo.len())
        {
            spans.push(SecretSpan {
                start: index + 3,
                end: at + 1,
                kind: SecretKind::Credentials,
            });
            index = at + 1;
            continue;
        }
        index = at + 1;
    }
    spans
}

/// Rule 5: Shannon-mean entropy across a sliding 32-byte window, limit 4.7 bits/char,
/// two consecutive windows — calibrated on Spike B (design §9). Never a 16-byte window:
/// its theoretical ceiling is log2(16) = 4.0, unreachable above any real limit.
fn entropy_spans(text: &str) -> Vec<SecretSpan> {
    let bytes = text.as_bytes();
    if bytes.len() < ENTROPY_WINDOW * ENTROPY_RUN {
        return Vec::new();
    }

    let mut counts = [0u32; 256];
    let mut unique = 0_u32;
    for &byte in &bytes[..ENTROPY_WINDOW] {
        let slot = byte as usize;
        if counts[slot] == 0 {
            unique += 1;
        }
        counts[slot] += 1;
    }

    let n = bytes.len() - ENTROPY_WINDOW + 1;
    let mut high = vec![false; n];
    for start in 0..n {
        if unique >= ENTROPY_MIN_UNIQUE && shannon_from_counts(&counts) > ENTROPY_LIMIT {
            high[start] = true;
        }
        if start + 1 == n {
            break;
        }
        let outgoing = bytes[start] as usize;
        counts[outgoing] -= 1;
        if counts[outgoing] == 0 {
            unique -= 1;
        }
        let incoming = bytes[start + ENTROPY_WINDOW] as usize;
        if counts[incoming] == 0 {
            unique += 1;
        }
        counts[incoming] += 1;
    }

    let mut spans = Vec::new();
    let mut index = 0;
    while index < high.len() {
        if high[index] {
            let run_start = index;
            let mut run_end = index;
            while run_end + 1 < high.len() && high[run_end + 1] {
                run_end += 1;
            }
            if run_end - run_start + 1 >= ENTROPY_RUN {
                let span_start = run_start;
                let span_end = (run_end + ENTROPY_WINDOW).min(bytes.len());
                let span_bytes = &bytes[span_start..span_end];
                // Trim surrounding whitespace so the redaction keeps the text tidy.
                let mut trimmed_start = span_start;
                let mut trimmed_end = span_end;
                while trimmed_start < trimmed_end
                    && span_bytes[trimmed_start - span_start].is_ascii_whitespace()
                {
                    trimmed_start += 1;
                }
                while trimmed_end > trimmed_start
                    && span_bytes[trimmed_end - span_start - 1].is_ascii_whitespace()
                {
                    trimmed_end -= 1;
                }
                if trimmed_end > trimmed_start {
                    spans.push(SecretSpan {
                        start: trimmed_start,
                        end: trimmed_end,
                        kind: SecretKind::Credentials,
                    });
                }
                index = run_end + 1;
                continue;
            }
        }
        index += 1;
    }
    spans
}

fn shannon_from_counts(counts: &[u32; 256]) -> f64 {
    let length = ENTROPY_WINDOW as f64;
    -counts
        .iter()
        .filter(|count| **count > 0)
        .map(|count| {
            let p = *count as f64 / length;
            p * p.log2()
        })
        .sum::<f64>()
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Friendly view of an approved secret file: keys only for `.env`, metadata for keys (design §6.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SecretFileView {
    DotEnv {
        keys: Vec<EnvKey>,
    },
    PrivateKey {
        /// e.g. "RSA", "OPENSSH".
        key_kind: String,
        /// short hex digest of the PEM body, for identification without the key material.
        fingerprint: String,
    },
    Credentials {
        /// Nothing to show: only the fact that the file matched a credentials pattern.
        matched_pattern: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct EnvKey {
    pub key: String,
    pub set: bool,
}

/// Builds the approved view of a secret file, never exposing values (design §6.4).
pub fn secret_file_view(kind: &SecretKind, content: &str) -> SecretFileView {
    match kind {
        SecretKind::DotEnv => {
            let keys = content
                .lines()
                .filter_map(|line| {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with('#') {
                        return None;
                    }
                    let (key, value) = line.split_once('=')?;
                    // Multi-line quoted values are out of scope for v1 (design §6.4).
                    if value.trim_start().starts_with('"') {
                        return None;
                    }
                    Some(EnvKey {
                        key: key.trim().to_string(),
                        set: !value.trim().is_empty(),
                    })
                })
                .collect();
            SecretFileView::DotEnv { keys }
        }
        SecretKind::PrivateKey => {
            let fingerprint = fingerprint(content);
            SecretFileView::PrivateKey {
                key_kind: private_key_kind(content).to_string(),
                fingerprint,
            }
        }
        SecretKind::Credentials => SecretFileView::Credentials {
            matched_pattern: "credentials".to_string(),
        },
    }
}

fn private_key_kind(content: &str) -> &'static str {
    if content.starts_with("-----BEGIN OPENSSH") {
        "OPENSSH"
    } else if content.starts_with("-----BEGIN RSA") {
        "RSA"
    } else if content.starts_with("-----BEGIN EC") {
        "EC"
    } else if content.starts_with("-----BEGIN PGP") {
        "PGP"
    } else {
        "PRIVATE KEY"
    }
}

/// Short stable digest of the key body, hex-encoded.
fn fingerprint(content: &str) -> String {
    use sha2::digest::FixedOutput;
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let digest = hasher.finalize_fixed();
    let mut hex = String::with_capacity(12);
    for byte in digest.iter().take(6) {
        use std::fmt::Write;
        write!(hex, "{byte:02x}").ok();
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_detects_dotenv_variants() {
        for name in [".env", ".env.local", ".env.production"] {
            assert_eq!(
                detect_path_secret(Path::new(name)),
                Some(SecretKind::DotEnv)
            );
        }
        assert_eq!(
            detect_path_secret(Path::new(".env/../.env")),
            Some(SecretKind::DotEnv)
        );
    }

    #[test]
    fn path_detects_keys_and_credentials() {
        assert_eq!(
            detect_path_secret(Path::new("id_rsa")),
            Some(SecretKind::PrivateKey)
        );
        assert_eq!(
            detect_path_secret(Path::new("id_ed25519")),
            Some(SecretKind::PrivateKey)
        );
        assert_eq!(
            detect_path_secret(Path::new("server.pem")),
            Some(SecretKind::PrivateKey)
        );
        assert_eq!(
            detect_path_secret(Path::new("chain.cer")),
            Some(SecretKind::PrivateKey)
        );
        assert_eq!(
            detect_path_secret(Path::new("credentials.json")),
            Some(SecretKind::Credentials)
        );
        assert_eq!(
            detect_path_secret(Path::new("secret-token.txt")),
            Some(SecretKind::Credentials)
        );
    }

    #[test]
    fn path_ignores_plain_files() {
        for name in ["main.rs", "README.md", "package.json", "src/main.tsx"] {
            assert_eq!(detect_path_secret(Path::new(name)), None);
        }
    }

    #[test]
    fn path_ignores_dot_env_folder_prefix() {
        // A folder named ".env.development" must NOT become a secret by substring alone.
        assert_eq!(
            detect_path_secret(Path::new(".env.example/readme.md")),
            None
        );
    }

    #[test]
    fn redacts_known_prefix_tokens() {
        let redacted = redact("use o sk-live-foo123 no deploy; o ghp_abcDEF123 não");
        assert_eq!(
            redacted.text,
            "use o [REDIGIDO:segredo] no deploy; o [REDIGIDO:segredo] não"
        );
        assert!(redacted.count() >= 2);
    }

    #[test]
    fn redacts_aws_style_caps_prefix() {
        let text = "creds=AKIAIOSFODNN7EXAMPLE";
        let redacted = redact(text);
        assert!(redacted.text.contains("[REDIGIDO:segredo]"));
    }

    #[test]
    fn redacts_pem_block() {
        let pem = "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEF\n-----END PRIVATE KEY-----\n";
        let redacted = redact(pem);
        assert!(redacted.text.contains("[REDIGIDO:chave]"));
        assert!(!redacted.text.contains("MIIEvQIB"));
    }

    #[test]
    fn leaves_public_pem_alone() {
        // -----BEGIN CERTIFICATE----- has no PRIVATE KEY inside.
        let cert = "-----BEGIN CERTIFICATE-----\nMIIC\n-----END CERTIFICATE-----\n";
        let redacted = redact(cert);
        assert!(!redacted.text.contains("chave"));
        assert!(redacted.text.contains("MIIC"));
    }

    fn base64url(bytes: &[u8]) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let mut buffer = (chunk[0] as u32) << 16;
            if chunk.len() > 1 {
                buffer |= (chunk[1] as u32) << 8;
            }
            if chunk.len() > 2 {
                buffer |= chunk[2] as u32;
            }
            out.push(TABLE[(buffer >> 18) as usize & 0x3f] as char);
            out.push(TABLE[(buffer >> 12) as usize & 0x3f] as char);
            if chunk.len() > 1 {
                out.push(TABLE[(buffer >> 6) as usize & 0x3f] as char);
            }
            if chunk.len() > 2 {
                out.push(TABLE[buffer as usize & 0x3f] as char);
            }
        }
        out
    }

    #[test]
    fn redacts_jwt() {
        // Header {"alg":"HS256","typ":"JWT"} in base64url.
        let header = base64url(b"{\"alg\":\"HS256\",\"typ\":\"JWT\"}");
        let payload = base64url(b"{\"sub\":\"123\"}");
        let token = format!("{header}.{payload}.signature_bytes_aaaa");
        let redacted = redact(&format!("token={token} fim"));
        assert!(redacted.text.contains("[REDIGIDO:segredo]"));
        assert!(!redacted.text.contains("signature_bytes"));
    }

    #[test]
    fn redacts_url_userinfo() {
        let text = "git clone http://usuario:senha123@github.com/repo.git";
        let redacted = redact(text);
        assert!(redacted.text.contains("[REDIGIDO:segredo]"));
        assert!(!redacted.text.contains("senha123"));
    }

    #[test]
    fn entropy_catches_high_cardinality_run() {
        // 200 random-ish chars: Shannon entropy far above 4.7.
        let needs_more: Vec<u8> = (0..200)
            .map(|i| 0x21 + ((i * 37 + 11) % 0x5e) as u8)
            .collect();
        let text = String::from_utf8(needs_more).unwrap();
        let redacted = redact(&text);
        assert!(redacted.count() > 0, "high entropy run must be redacted");
    }

    #[test]
    fn entropy_leaves_pt_br_prose_alone() {
        let text = "O agente deve ler os arquivos do workspace antes de editar qualquer coisa, porque o contexto importa e erros custam tempo de todos.";
        let redacted = redact(text);
        assert_eq!(redacted.text, text);
        assert_eq!(redacted.count(), 0);
    }

    #[test]
    fn entropy_leaves_rust_code_alone() {
        let text = "fn classify(argv: &[String]) -> CommandClass { /* tabela determinística, sem regex */ }\nlet Ok(outcome) = engine.run(request, &mut sink) else { return Err; };";
        let redacted = redact(text);
        assert_eq!(redacted.text, text);
    }

    #[test]
    fn entropy_scan_of_low_cardinality_source_is_bounded() {
        use std::time::Instant;
        let text = "x".repeat(64 * 1024);
        let started = Instant::now();
        let redacted = redact(&text);
        let elapsed = started.elapsed();
        assert_eq!(redacted.count(), 0);
        assert!(
            elapsed.as_millis() < 50,
            "low-cardinality 64 KiB must stay cheap: {elapsed:?}"
        );
    }

    #[test]
    fn entropy_window_defaults_are_spike_b_calibration() {
        // Regression: the ceiling of a 16-byte window is log2(16) = 4.0, so a limit of 4.7
        // would be unreachable. The design (decision D7) mandates 32 bytes.
        const {
            assert!(
                ENTROPY_WINDOW >= 32,
                "janela de 16 bytes tem teto 4.0 bits/char"
            )
        }
        const { assert!(ENTROPY_LIMIT > 4.0) }
    }

    #[test]
    fn merged_spans_do_not_emit_twice() {
        // "sk_live_" + "AKIA" in the same line must not produce overlapping duplicate spans.
        let text = "pk=sk_live_abcdefg hip=AKIAIOSFODNN7EXAMPLE";
        let spans = recognize(text);
        for window in spans.windows(2) {
            assert!(window[0].end <= window[1].start);
        }
    }

    #[test]
    fn dotenv_view_exposes_only_keys() {
        let view = secret_file_view(
            &SecretKind::DotEnv,
            "DB_URL=postgres://u:p@h/db\nDISCORD=123\nMULTI_LINE=\"a\nb\"",
        );
        match view {
            SecretFileView::DotEnv { keys } => {
                assert_eq!(keys.len(), 2); // MULTI_LINE quebra na primeira linha (v1)
                assert_eq!(
                    keys[0],
                    EnvKey {
                        key: "DB_URL".to_string(),
                        set: true
                    }
                );
                assert_eq!(
                    keys[1],
                    EnvKey {
                        key: "DISCORD".to_string(),
                        set: true
                    }
                );
                let _ = keys;
            }
            _ => panic!("esperado DotEnv"),
        }
    }

    #[test]
    fn private_key_view_exposes_only_metadata() {
        let pem =
            "-----BEGIN RSA PRIVATE KEY-----\nMIIEvQIBADANBgkqhki\n-----END RSA PRIVATE KEY-----\n";
        let view = secret_file_view(&SecretKind::PrivateKey, pem);
        match view {
            SecretFileView::PrivateKey {
                key_kind,
                fingerprint,
            } => {
                assert_eq!(key_kind, "RSA");
                assert_eq!(fingerprint.len(), 12);
            }
            _ => panic!("esperado PrivateKey"),
        }
    }
}
