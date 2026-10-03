package deploytool

import (
	"testing"

	"github.com/jimmy-yao/ta-deploy-tool/internal/config"
)

// TestTemplateIsValid 保证嵌入的 config.template.yaml 始终能被 Go 版加载器解析，
// 避免模板与 config 包字段演进脱节。
func TestTemplateIsValid(t *testing.T) {
	if len(TemplateYAML) == 0 {
		t.Fatal("嵌入的 config.template.yaml 为空")
	}
	cfg, err := config.LoadBytes(TemplateYAML)
	if err != nil {
		t.Fatalf("模板无法被 config.LoadBytes 解析: %v", err)
	}
	if len(cfg.Environments) == 0 {
		t.Fatal("模板应至少包含一个环境")
	}
	if cfg.Git.Remote == "" || cfg.Git.BranchMode == "" {
		t.Fatalf("模板缺少 git 配置: %+v", cfg.Git)
	}
	if cfg.LogEncoding == "" {
		t.Fatal("模板缺少 log_encoding")
	}
}

func TestVersionSet(t *testing.T) {
	if Version == "" {
		t.Fatal("Version 不应为空")
	}
}
