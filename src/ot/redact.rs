//! Best-effort scrubbing of secrets from text that came out of `ot`.
//!
//! `tt` never handles credentials itself (`docs/opentodo-compatibility.md` §8),
//! so this exists only so that an unexpected `ot` message cannot put a token
//! into a terminal, a log, or a pasted bug report. It is deliberately
//! conservative: over-redacting an error message makes it useless, so the
//! heuristics only fire on things that look like secrets rather than like
//! words, ids, or timestamps.

const SENSITIVE: [&str; 9] = [
    "token", "authorization", "bearer", "password", "passphrase", "secret", "apikey", "api_key",
    "credential",
];

pub const PLACEHOLDER: &str = "[redacted]";

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')
}

fn is_sensitive_key(word: &str) -> bool {
    let lower = word.to_ascii_lowercase();
    SENSITIVE.iter().any(|key| lower.contains(key))
}

/// A bare word that looks like a credential on its own. Long, mixed, and not
/// shaped like a UUID (OpenTodo ids are useful in error messages).
fn looks_like_secret(word: &str) -> bool {
    word.len() >= 40
        && word.chars().any(|c| c.is_ascii_digit())
        && word.chars().any(|c| c.is_ascii_alphabetic())
        && word.matches('-').count() != 4
}

/// A word that follows a sensitive key and is probably its value rather than
/// the next English word in a sentence.
fn looks_like_value(word: &str, after_bearer: bool) -> bool {
    if after_bearer {
        return true;
    }
    (word.len() >= 16 && word.chars().any(|c| c.is_ascii_digit())) || word.len() >= 24
}

pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let mut armed = false;
    let mut after_bearer = false;

    let flush = |word: &mut String, out: &mut String, armed: &mut bool, after_bearer: &mut bool| {
        if word.is_empty() {
            return;
        }
        if is_sensitive_key(word) {
            out.push_str(word);
            *armed = true;
            *after_bearer = word.eq_ignore_ascii_case("bearer");
        } else if *armed && looks_like_value(word, *after_bearer) {
            out.push_str(PLACEHOLDER);
            *armed = false;
            *after_bearer = false;
        } else if looks_like_secret(word) {
            out.push_str(PLACEHOLDER);
            *armed = false;
        } else {
            out.push_str(word);
        }
        word.clear();
    };

    for ch in text.chars() {
        if is_word_char(ch) {
            word.push(ch);
            continue;
        }
        flush(&mut word, &mut out, &mut armed, &mut after_bearer);
        if ch == '\n' {
            armed = false;
            after_bearer = false;
        }
        out.push(ch);
    }
    flush(&mut word, &mut out, &mut armed, &mut after_bearer);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_bearer_tokens() {
        let out = redact("Authorization: Bearer ot_abc123def456");
        assert!(!out.contains("ot_abc123def456"), "{out}");
        assert!(out.contains("Bearer [redacted]"), "{out}");
    }

    #[test]
    fn masks_json_token_fields() {
        let out = redact(r#"{"token":"a1b2c3d4e5f6g7h8i9j0k1l2"}"#);
        assert!(!out.contains("a1b2c3d4"), "{out}");
    }

    #[test]
    fn masks_a_long_bare_secret() {
        let out = redact("failed with sk-0123456789abcdef0123456789abcdef0123456789");
        assert!(out.contains(PLACEHOLDER), "{out}");
    }

    #[test]
    fn leaves_ordinary_messages_alone() {
        let input = "token revoked; run `ot login https://todo.example.com` again";
        assert_eq!(redact(input), input);
    }

    #[test]
    fn leaves_ids_and_timestamps_alone() {
        let input = "entry 3f1d-8c2a started at 2026-10-27T14:00:00Z in project Work";
        assert_eq!(redact(input), input);
        let uuid = "entry 123e4567-e89b-12d3-a456-426614174000 not found";
        assert_eq!(redact(uuid), uuid);
    }

    #[test]
    fn a_newline_disarms_a_dangling_key() {
        let out = redact("token\n0123456789abcdefghij");
        assert!(out.contains("0123456789abcdefghij"), "{out}");
    }
}
