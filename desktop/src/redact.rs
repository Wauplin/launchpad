//! Secret redaction for every log line: bearer tokens, device secrets, Hugging Face tokens, OAuth codes and long hex
//! secrets never reach a log file or the terminal.

use std::sync::LazyLock;

use regex::Regex;

static RULES: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        // Headers: `Authorization: Bearer …`, `X-Launchpad-Device: …`, `Cookie: …`.
        (r"(?i)(bearer\s+)[^\s,;\x22']+", "${1}[redacted]"),
        (r"(?i)(x-launchpad-device\x22?\s*[:=]\s*\x22?)[^\s,;\x22']+", "${1}[redacted]"),
        (r"(?i)(cookie\x22?\s*[:=]\s*\x22?)[^\x22\n]+", "${1}[redacted]"),
        // Query, form and JSON fields that carry secrets.
        (
            r#"(?i)\b((?:access_token|refresh_token|id_token|code_verifier|code|state|secret|device|token)(?:\x22\s*:\s*\x22|=))[^&\s\x22,;]+"#,
            "${1}[redacted]",
        ),
        // Hugging Face tokens and long hex secrets (device secrets are 64 hex characters).
        (r"\bhf_[A-Za-z0-9_]{6,}", "hf_[redacted]"),
        (r"\b[0-9a-fA-F]{32,}\b", "[redacted]"),
    ]
    .into_iter()
    .map(|(re, to)| (Regex::new(re).expect("valid redaction rule"), to))
    .collect()
});

/// The text with every secret replaced by `[redacted]`.
pub fn redact(text: &str) -> String {
    RULES.iter().fold(text.to_string(), |acc, (re, to)| re.replace_all(&acc, *to).into_owned())
}

#[cfg(test)]
mod tests {
    use super::redact;

    #[test]
    fn secrets_never_survive() {
        let secret = "a".repeat(64);
        for (line, leaked) in [
            ("GET /api/today Authorization: Bearer hf_abcdefghijkl", "abcdefghijkl"),
            (&*format!("X-Launchpad-Device: {secret}"), &*secret),
            (&*format!(r#"{{"ok":true,"device":"{secret}"}}"#), &*secret),
            ("callback /callback?code=XYZ123&state=s7a7e9", "XYZ123"),
            ("callback /callback?code=XYZ123&state=s7a7e9", "s7a7e9"),
            ("grant_type=refresh_token&refresh_token=r3fr3sh&client_id=abc", "r3fr3sh"),
            (r#"{"access_token":"tok123","expires_in":3600}"#, "tok123"),
            ("token hf_ZZZZZZZZZZZZ in text", "ZZZZZZZZZZZZ"),
            (&*format!("pairing secret {secret}"), &*secret),
        ] {
            let out = redact(line);
            assert!(!out.contains(leaked), "{line} -> {out}");
            assert!(out.contains("[redacted]"), "{line} -> {out}");
        }
        // Ordinary text stays readable.
        assert_eq!(redact("Home answered 503 for /api/today"), "Home answered 503 for /api/today");
        assert_eq!(redact("client_id=abc workspace=w1"), "client_id=abc workspace=w1");
    }
}
