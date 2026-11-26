/// Replaces every occurrence of each `secret` in `text` with `"***"`.
/// Matching is case-sensitive (Binance API keys are uppercase hex).
#[must_use]
pub fn redact_secrets(text: &str, secrets: &[&str]) -> String {
    let mut result = text.to_owned();
    for secret in secrets {
        if secret.is_empty() {
            continue;
        }
        result = result.replace(*secret, "***");
    }
    result
}

/// Stateful redactor that holds a list of secrets to scrub from log lines.
/// Matches `paperbot.journal.Redact` without depending on the Python logging API.
#[derive(Debug, Default)]
pub struct LogRedactor {
    secrets: Vec<String>,
}

impl LogRedactor {
    /// Creates a new `LogRedactor` from an iterator of secret strings.
    pub fn new<I, S>(secrets: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let secrets: Vec<String> = secrets
            .into_iter()
            .map(|s| s.into())
            .filter(|s| !s.is_empty())
            .collect();
        Self { secrets }
    }

    /// Adds a secret at runtime (e.g. when credentials are injected after startup).
    pub fn add_secret(&mut self, secret: impl Into<String>) {
        let s = secret.into();
        if !s.is_empty() {
            self.secrets.push(s);
        }
    }

    /// Returns a redacted copy of `text`.
    #[must_use]
    pub fn redact(&self, text: &str) -> String {
        let refs: Vec<&str> = self.secrets.iter().map(String::as_str).collect();
        redact_secrets(text, &refs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_known_secrets_and_leaves_other_text_intact() {
        let redacted = redact_secrets(
            "oops SECRETKEY123 and SECRETSEC456",
            &["SECRETKEY123", "SECRETSEC456"],
        );
        assert!(!redacted.contains("SECRET"));
        assert!(redacted.contains("***"));
        assert!(redacted.contains("oops"));
    }

    #[test]
    fn log_redactor_scrubs_all_registered_secrets() {
        let mut r = LogRedactor::new(["KEY_A"]);
        r.add_secret("KEY_B");
        let out = r.redact("message contains KEY_A and KEY_B here");
        assert!(!out.contains("KEY_A") && !out.contains("KEY_B"));
        assert!(out.contains("***"));
    }

    #[test]
    fn empty_secrets_are_ignored() {
        let r = LogRedactor::new(["", "REALKEY"]);
        let out = r.redact("text REALKEY more");
        assert!(!out.contains("REALKEY"));
        assert!(out.contains("***"));
    }
}
