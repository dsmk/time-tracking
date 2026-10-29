package ot

import (
	"strings"
	"unicode"
)

// Best-effort scrubbing of secrets from text that came out of ot.
//
// tt never handles credentials itself (docs/opentodo-compatibility.md §8), so
// this exists only so that an unexpected ot message cannot put a token into a
// terminal, a log, or a pasted bug report. It is deliberately conservative:
// over-redacting an error message makes it useless, so the heuristics only fire
// on things that look like secrets rather than like words, ids, or timestamps.

var sensitive = []string{
	"token", "authorization", "bearer", "password", "passphrase", "secret",
	"apikey", "api_key", "credential",
}

// Placeholder replaces anything that looked like a credential.
const Placeholder = "[redacted]"

func isWordChar(r rune) bool {
	return r < unicode.MaxASCII &&
		(unicode.IsLetter(r) || unicode.IsDigit(r) || r == '_' || r == '-' || r == '.')
}

func isSensitiveKey(word string) bool {
	lower := strings.ToLower(word)
	for _, key := range sensitive {
		if strings.Contains(lower, key) {
			return true
		}
	}
	return false
}

// looksLikeSecret matches a bare word that looks like a credential on its own:
// long, mixed, and not shaped like a UUID (OpenTodo ids are useful in error
// messages).
func looksLikeSecret(word string) bool {
	return len(word) >= 40 &&
		strings.ContainsFunc(word, isASCIIDigit) &&
		strings.ContainsFunc(word, isASCIILetter) &&
		strings.Count(word, "-") != 4
}

// looksLikeValue matches a word that follows a sensitive key and is probably
// its value rather than the next English word in a sentence.
func looksLikeValue(word string, afterBearer bool) bool {
	if afterBearer {
		return true
	}
	return (len(word) >= 16 && strings.ContainsFunc(word, isASCIIDigit)) || len(word) >= 24
}

// Redact masks anything in text that looks like a credential.
func Redact(text string) string {
	var out strings.Builder
	out.Grow(len(text))
	var word strings.Builder
	armed := false
	afterBearer := false

	flush := func() {
		if word.Len() == 0 {
			return
		}
		current := word.String()
		word.Reset()
		switch {
		case isSensitiveKey(current):
			out.WriteString(current)
			armed = true
			afterBearer = strings.EqualFold(current, "bearer")
		case armed && looksLikeValue(current, afterBearer):
			out.WriteString(Placeholder)
			armed = false
			afterBearer = false
		case looksLikeSecret(current):
			out.WriteString(Placeholder)
			armed = false
		default:
			out.WriteString(current)
		}
	}

	for _, r := range text {
		if isWordChar(r) {
			word.WriteRune(r)
			continue
		}
		flush()
		if r == '\n' {
			armed = false
			afterBearer = false
		}
		out.WriteRune(r)
	}
	flush()
	return out.String()
}

func isASCIIDigit(r rune) bool  { return r >= '0' && r <= '9' }
func isASCIILetter(r rune) bool { return (r >= 'a' && r <= 'z') || (r >= 'A' && r <= 'Z') }
