// Command deploytool 是 TA-DEPLOY-TOOL 的 Go 重写版入口。
//
// 用法：
//
//	deploytool                 # 自动定位 / 首次生成 config.yaml 后启动 GUI
//	deploytool <config.yaml>   # 指定配置文件（与原 Python 版位置参数兼容）
//	deploytool -config x.yaml  # 同上
//	deploytool -check          # 仅校验配置（无 GUI，适合 CI）
package main

import (
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"

	deploytool "github.com/jimmy-yao/ta-deploy-tool"
	"github.com/jimmy-yao/ta-deploy-tool/internal/config"
	"github.com/jimmy-yao/ta-deploy-tool/ui"
)

func main() {
	// Android 构建下 Fyne 会把标准输出改道到 logcat，先还原。
	deploytool.RestoreStdio()

	configPath := flag.String("config", "", "配置文件路径（默认自动定位）")
	check := flag.Bool("check", false, "仅加载并校验配置，打印结果后退出")
	showVersion := flag.Bool("version", false, "打印版本信息后退出")
	flag.Parse()

	if *showVersion {
		fmt.Printf("TA-DEPLOY-TOOL %s (%s/%s)\n", deploytool.Version, runtime.GOOS, runtime.GOARCH)
		return
	}

	path := strings.TrimSpace(*configPath)
	if path == "" && flag.NArg() > 0 {
		path = strings.TrimSpace(flag.Arg(0))
	}

	var err error
	if path == "" {
		path, err = ensureConfig()
		if err != nil {
			fatal(err)
		}
	}

	if *check {
		if err := checkConfig(path); err != nil {
			fatal(err)
		}
		return
	}

	if err := ui.Run(path, deploytool.AppIconPNG); err != nil {
		fatal(err)
	}
}

// checkConfig 加载并校验配置，返回带汇总信息的错误。
func checkConfig(path string) error {
	cfg, err := config.Load(path)
	if err != nil {
		return err
	}
	problems := config.Validate(cfg)
	fmt.Printf("配置文件: %s\n", path)
	fmt.Printf("环境数量: %d\n", len(cfg.Environments))
	for name, env := range cfg.Environments {
		fmt.Printf("  - %s: %d 个服务, %d 个命名服务器\n",
			name, len(env.Services), len(env.Servers))
	}
	if len(problems) > 0 {
		return fmt.Errorf("配置存在 %d 个问题:\n  - %s",
			len(problems), strings.Join(problems, "\n  - "))
	}
	fmt.Println("配置校验通过")
	return nil
}

// ============================================================
// 首次运行的配置定位（与原 Python main.ensure_config 对应）
// ============================================================

// appDir 返回可执行文件所在目录。
func appDir() string {
	exe, err := os.Executable()
	if err != nil {
		wd, _ := os.Getwd()
		return wd
	}
	if resolved, err := filepath.EvalSymlinks(exe); err == nil {
		exe = resolved
	}
	return filepath.Dir(exe)
}

// isTempDir 判断目录是否位于系统临时目录。
// `go run` 生成的二进制在临时目录里，此时应以工作目录为准，
// 否则每次运行都会重新生成一份"消失的"配置。
func isTempDir(dir string) bool {
	tmp := os.TempDir()
	if tmp == "" {
		return false
	}
	a, errA := filepath.Abs(dir)
	b, errB := filepath.Abs(tmp)
	if errA != nil || errB != nil {
		return false
	}
	return strings.EqualFold(a, b) || strings.HasPrefix(a, b+string(filepath.Separator))
}

// userConfigDir 返回用户级配置目录（与 Python user_config_dir 对应）。
func userConfigDir() string {
	var base string
	switch runtime.GOOS {
	case "windows":
		base = os.Getenv("APPDATA")
		if base == "" {
			home, _ := os.UserHomeDir()
			base = filepath.Join(home, "AppData", "Roaming")
		}
	case "darwin":
		home, _ := os.UserHomeDir()
		base = filepath.Join(home, "Library", "Application Support")
	default:
		base = os.Getenv("XDG_CONFIG_HOME")
		if base == "" {
			home, _ := os.UserHomeDir()
			base = filepath.Join(home, ".config")
		}
	}
	return filepath.Join(base, "deploy_tool")
}

func fileExists(p string) bool {
	st, err := os.Stat(p)
	return err == nil && !st.IsDir()
}

func dirWritable(dir string) bool {
	info, err := os.Stat(dir)
	if err != nil || !info.IsDir() {
		return false
	}
	test := filepath.Join(dir, fmt.Sprintf(".write_test_%d", os.Getpid()))
	if err := os.WriteFile(test, []byte("x"), 0o600); err != nil {
		return false
	}
	_ = os.Remove(test)
	return true
}

// ensureConfig 定位配置文件，必要时从内置模板生成一份。
func ensureConfig() (string, error) {
	exeDir := appDir()
	wd, _ := os.Getwd()
	userPath := filepath.Join(userConfigDir(), "config.yaml")

	var existing, dirs []string
	// `go run` 的二进制落在临时目录，此时 exe 目录没有意义，
	// 以工作目录为准，否则每次运行都会重新生成一份"消失的"配置。
	if !isTempDir(exeDir) {
		existing = append(existing, filepath.Join(exeDir, "config.yaml"))
		dirs = append(dirs, exeDir)
	}
	if wd != "" {
		existing = append(existing, filepath.Join(wd, "config.yaml"))
		dirs = append(dirs, wd)
	}
	existing = append(existing, userPath)
	dirs = append(dirs, filepath.Dir(userPath))

	return locateConfig(existing, dirs, deploytool.TemplateYAML)
}

// locateConfig：先在 existing 里找已存在的配置；都没有则在 dirs 中
// 按优先级挑第一个可写目录，用模板生成一份。
func locateConfig(existing, dirs []string, template []byte) (string, error) {
	for _, c := range existing {
		if fileExists(c) {
			return c, nil
		}
	}

	var lastErr error
	for _, dir := range dirs {
		// 用户配置目录（如 ~/.config/deploy_tool）首次运行时并不存在
		if err := os.MkdirAll(dir, 0o755); err != nil {
			lastErr = err
			continue
		}
		if !dirWritable(dir) {
			continue
		}
		target := filepath.Join(dir, "config.yaml")
		if err := os.WriteFile(target, template, 0o644); err != nil {
			lastErr = err
			continue
		}
		fmt.Printf("[首次运行] 已生成默认配置文件：%s\n", target)
		fmt.Println("[首次运行] 请检查后重新启动程序（也可在程序内点击「设置」直接编辑）。")
		return target, nil
	}
	if lastErr != nil {
		return "", fmt.Errorf("无法创建配置文件: %w", lastErr)
	}
	return "", fmt.Errorf("没有可写目录用于创建 config.yaml（尝试过: %s）",
		strings.Join(dirs, ", "))
}

func fatal(err error) {
	fmt.Fprintln(os.Stderr, "[错误]", err)
	os.Exit(1)
}
