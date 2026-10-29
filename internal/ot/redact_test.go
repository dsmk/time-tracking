package ot

import (
	"strings"
	"testing"
)

func TestMasksBearerTokens(t *testing.T) {
	out := Redact("Authorization: Bearer ot_abc123def456")
	if strings.Contains(out, "ot_abc123def456") {
		t.Fatalf("the token survived: %q", out)
	}
	if !strings.Contains(out, "Bearer "+Placeholder) {
		t.Fatalf("got %q", out)
	}
}

func TestMasksJSONTokenFields(t *testing.T) {
	out := Redact(`{"token":"a1b2c3d4e5f6g7h8i9j0k1l2"}`)
	if strings.Contains(out, "a1b2c3d4") {
		t.Fatalf("the token survived: %q", out)
	}
}

func TestMasksALongBareSecret(t *testing.T) {
	out := Redact("failed with sk-0123456789abcdef0123456789abcdef0123456789")
	if !strings.Contains(out, Placeholder) {
		t.Fatalf("got %q", out)
	}
}

func TestLeavesOrdinaryMessagesAlone(t *testing.T) {
	input := "token revoked; run `ot login https://todo.example.com` again"
	if got := Redact(input); got != input {
		t.Fatalf("over-redacted: %q", got)
	}
}

func TestLeavesIDsAndTimestampsAlone(t *testing.T) {
	for _, input := range []string{
		"entry 3f1d-8c2a started at 2026-10-27T14:00:00Z in project Work",
		"entry 123e4567-e89b-12d3-a456-426614174000 not found",
	} {
		if got := Redact(input); got != input {
			t.Errorf("over-redacted %q as %q", input, got)
		}
	}
}

func TestANewlineDisarmsADanglingKey(t *testing.T) {
	out := Redact("token\n0123456789abcdefghij")
	if !strings.Contains(out, "0123456789abcdefghij") {
		t.Fatalf("a new line is a new sentence: %q", out)
	}
}
