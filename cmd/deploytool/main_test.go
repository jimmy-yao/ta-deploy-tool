package main

import (
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	deploytool "github.com/jimmy-yao/ta-deploy-tool"
)

// TestMain 在跑测试前还原标准输出：
// 本包经由 ui 引入 Fyne，Android 构建下 Fyne 会把 fd 1/2 改道到 logcat，
// 否则 t.Errorf 的失败详情全部看不见（根包 init 已提前复制好原始 fd）。
func TestMain(m *testing.M) {
	deploytool.RestoreStdio()
	os.Exit(m.Run())
}

const validConfigYAML = `maven: mvn
log_encoding: utf-8
backup_keep: 5
default_restart_script: start.sh
auto_remote_name: true
git:
  enabled: true
  branch_mode: fixed
  default_branch: main
  iteration_pattern: '^feature_(\d{8})_xc$'
  remote: origin
  strategy: reset
  fetch_timeout: 180
  ls_remote_timeout: 60
environments:
  test:
    default_server:
      host: 127.0.0.1
      port: 22
      user: deploy
      password: 'pw'
    servers: {}
    services:
      demo:
        project_dir: /tmp/demo
        build_args: clean install -DskipTests
        local_jar: target/demo.jar
        remote_dir: /opt/demo
        remote_name: ""
        depends_on: []
`

func writeFile(t *testing.T, dir, name, content string) string {
	t.Helper()
	p := filepath.Join(dir, name)
	if err := os.WriteFile(p, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
	return p
}

// ============================================================
// checkConfig
// ============================================================

func TestCheckConfigValid(t *testing.T) {
	p := writeFile(t, t.TempDir(), "config.yaml", validConfigYAML)
	if err := checkConfig(p); err != nil {
		t.Fatalf("合法配置应通过校验: %v", err)
	}
}

func TestCheckConfigMissingFile(t *testing.T) {
	err := checkConfig(filepath.Join(t.TempDir(), "nope.yaml"))
	if err == nil {
		t.Fatal("文件不存在时应报错")
	}
}

func TestCheckConfigInvalidService(t *testing.T) {
	// project_dir 为空是模板里已知的占位，必须被检出
	p := filepath.Join("..", "..", "config.template.yaml")
	err := checkConfig(p)
	if err == nil {
		t.Fatal("模板中的空 project_dir 应被检出")
	}
	if !strings.Contains(err.Error(), "project_dir") {
		t.Fatalf("错误信息应包含 project_dir: %v", err)
	}
}

// ============================================================
// locateConfig（首次运行的定位 / 生成）
// ============================================================

func TestLocateConfigPrefersExisting(t *testing.T) {
	base := t.TempDir()
	miss1 := filepath.Join(base, "miss1", "config.yaml")
	hit := writeFile(t, base, "config.yaml", validConfigYAML)
	miss2 := filepath.Join(base, "miss2", "config.yaml")

	got, err := locateConfig([]string{miss1, hit, miss2}, []string{filepath.Join(base, "gen")}, []byte("x"))
	if err != nil {
		t.Fatal(err)
	}
	if got != hit {
		t.Fatalf("应选中已存在的配置: got %q want %q", got, hit)
	}
	// 不应生成新文件
	if _, err := os.Stat(filepath.Join(base, "gen", "config.yaml")); err == nil {
		t.Fatal("已存在配置时不应再生成")
	}
}

func TestLocateConfigCreatesFromTemplate(t *testing.T) {
	base := t.TempDir()
	genDir := filepath.Join(base, "gen")
	tmpl := []byte("# template\n" + validConfigYAML)

	got, err := locateConfig(
		[]string{filepath.Join(base, "missing.yaml")},
		[]string{genDir},
		tmpl,
	)
	if err != nil {
		t.Fatal(err)
	}
	if got != filepath.Join(genDir, "config.yaml") {
		t.Fatalf("生成路径不符: %q", got)
	}
	raw, err := os.ReadFile(got)
	if err != nil {
		t.Fatal(err)
	}
	if string(raw) != string(tmpl) {
		t.Fatalf("生成内容与模板不一致:\n%s", raw)
	}
}

func TestLocateConfigUnwritable(t *testing.T) {
	base := t.TempDir()
	// 在普通文件下面建目录必然失败（ENOTDIR），比依赖 chmod 更可靠
	blocker := writeFile(t, base, "blocker", "x")
	badDir := filepath.Join(blocker, "child")

	_, err := locateConfig(nil, []string{badDir}, []byte("x"))
	if err == nil {
		t.Fatal("全部目录不可写时应报错")
	}
}

func TestLocateConfigCreatesMissingDir(t *testing.T) {
	base := t.TempDir()
	genDir := filepath.Join(base, "nested", "deep", "dir")

	got, err := locateConfig(nil, []string{genDir}, []byte("payload"))
	if err != nil {
		t.Fatal(err)
	}
	if got != filepath.Join(genDir, "config.yaml") {
		t.Fatalf("路径不符: %q", got)
	}
	if !fileExists(got) {
		t.Fatal("应创建缺失的父目录后写入配置")
	}
}

func TestLocateConfigSkipsUnwritableThenUsesNext(t *testing.T) {
	base := t.TempDir()
	good := filepath.Join(base, "good")
	if err := os.Mkdir(good, 0o755); err != nil {
		t.Fatal(err)
	}
	blocker := writeFile(t, base, "blocker", "x")
	bad := filepath.Join(blocker, "child")

	got, err := locateConfig(nil, []string{bad, good}, []byte("payload"))
	if err != nil {
		t.Fatal(err)
	}
	if got != filepath.Join(good, "config.yaml") {
		t.Fatalf("应跳过不可写目录: %q", got)
	}
}

// ============================================================
// 路径 / 权限工具
// ============================================================

func TestFileExists(t *testing.T) {
	base := t.TempDir()
	f := writeFile(t, base, "a.yaml", "x")
	if !fileExists(f) {
		t.Fatal("普通文件应存在")
	}
	if fileExists(filepath.Join(base, "nope.yaml")) {
		t.Fatal("不存在的文件不应判为存在")
	}
	if fileExists(base) {
		t.Fatal("目录不应判为配置文件")
	}
}

func TestDirWritable(t *testing.T) {
	if !dirWritable(t.TempDir()) {
		t.Fatal("临时目录应可写")
	}
	if dirWritable(filepath.Join(t.TempDir(), "nope", "deep")) {
		t.Fatal("不存在的目录应判为不可写")
	}
}

func TestIsTempDir(t *testing.T) {
	if !isTempDir(os.TempDir()) {
		t.Fatalf("%q 应被判为临时目录", os.TempDir())
	}
	wd, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	if isTempDir(wd) {
		t.Fatalf("%q 不应被判为临时目录", wd)
	}
}

func TestUserConfigDir(t *testing.T) {
	d := userConfigDir()
	if !strings.HasSuffix(d, "deploy_tool") {
		t.Fatalf("用户配置目录应以 deploy_tool 结尾: %q", d)
	}
	switch runtime.GOOS {
	case "windows":
		if !strings.Contains(strings.ToLower(d), "appdata") {
			t.Fatalf("Windows 下应在 APPDATA 下: %q", d)
		}
	case "darwin":
		if !strings.Contains(d, "Application Support") {
			t.Fatalf("macOS 下应在 Application Support 下: %q", d)
		}
	default:
		if !strings.Contains(d, ".config") && !strings.Contains(os.Getenv("XDG_CONFIG_HOME"), "deploy_tool") {
			t.Fatalf("Linux 下应在 XDG 配置目录下: %q", d)
		}
	}
}

func TestAppDirIsDirectory(t *testing.T) {
	d := appDir()
	if d == "" {
		t.Fatal("appDir 不应为空")
	}
	st, err := os.Stat(d)
	if err != nil {
		t.Fatalf("appDir %q 不存在: %v", d, err)
	}
	if !st.IsDir() {
		t.Fatalf("appDir %q 不是目录", d)
	}
}

func TestVersionIsSet(t *testing.T) {
	if deploytool.Version == "" {
		t.Fatal("Version 不应为空")
	}
	if len(deploytool.TemplateYAML) == 0 {
		t.Fatal("模板未被嵌入")
	}
	if len(deploytool.AppIconPNG) == 0 {
		t.Fatal("图标未被嵌入")
	}
}
