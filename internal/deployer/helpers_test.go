package deployer

import (
	"crypto/ed25519"
	"crypto/rand"
	"encoding/pem"
	"os"
	"path/filepath"
	"testing"

	"golang.org/x/crypto/ssh"
)

// writeTestKey 生成一份无口令的 ed25519 私钥文件。
func writeTestKey(t *testing.T, path string) {
	t.Helper()
	_, priv, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatalf("生成密钥失败: %v", err)
	}
	block, err := ssh.MarshalPrivateKey(priv, "test-key")
	if err != nil {
		t.Fatalf("序列化私钥失败: %v", err)
	}
	if err := os.WriteFile(path, pem.EncodeToMemory(block), 0o600); err != nil {
		t.Fatalf("写入私钥失败: %v", err)
	}
}

// writeEncryptedTestKey 生成一份带口令的 ed25519 私钥文件。
func writeEncryptedTestKey(t *testing.T, path, passphrase string) {
	t.Helper()
	_, priv, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatalf("生成密钥失败: %v", err)
	}
	block, err := ssh.MarshalPrivateKeyWithPassphrase(priv, "test-key", []byte(passphrase))
	if err != nil {
		t.Fatalf("序列化加密私钥失败: %v", err)
	}
	if err := os.WriteFile(path, pem.EncodeToMemory(block), 0o600); err != nil {
		t.Fatalf("写入私钥失败: %v", err)
	}
}

// tempKeyPath 返回一个临时私钥路径。
func tempKeyPath(t *testing.T) string {
	t.Helper()
	return filepath.Join(t.TempDir(), "id_ed25519")
}
